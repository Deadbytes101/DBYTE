//! Project-level semantic analysis for the freestanding native systems profile.
//!
//! This pass consumes the shared frontend AST for every statically resolved
//! module, establishes target-specific types and layouts, and produces typed,
//! machine-independent IR. It never invokes the hosted interpreter or VM.

use crate::diagnostics::{Diagnostic, DiagnosticCategory, NativeResult};
use crate::ir::{CompareOp, LabelId, LocalId, StringId};
use crate::modules::{NativeModuleGraph, NativeSourceModule};
use crate::system_types::{
    checked_literal_bits, explicit_cast_allowed, StructDecl, StructFieldDecl, SystemType,
    TypeLayouts,
};
use crate::systems_ir::{
    ExternalFunction, ExternalGlobal, GlobalSection, InterruptClass, SystemIntrinsic,
    SystemsFunction, SystemsGlobal, SystemsInst, SystemsModuleIr, SystemsParam, SystemsSlot,
    Visibility,
};
use dbyte_ast::{BinOp, Expr, Param, Span, Stmt, StructField, TypeAnnotation, UnaryOp};
use std::collections::{BTreeMap, BTreeSet};

const ENTRY_FUNCTION: &str = "kernel_main";

/// Fully analyzed systems project in the same deterministic module order as
/// [`NativeModuleGraph`]. The shared layout table uses canonical nominal names
/// (`module::Type`) so imported struct types never collide accidentally.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemsProjectIr {
    pub root_module: usize,
    pub modules: Vec<SystemsModuleIr>,
    pub layouts: TypeLayouts,
}

impl SystemsProjectIr {
    pub fn module(&self, id: usize) -> Option<&SystemsModuleIr> {
        self.modules.get(id)
    }

    pub fn root_module(&self) -> &SystemsModuleIr {
        &self.modules[self.root_module]
    }

    pub fn dump(&self) -> String {
        let mut output = String::new();
        for module in &self.modules {
            output.push_str(&module.dump());
        }
        output
    }
}

#[derive(Debug, Clone)]
struct StructHeader {
    public: bool,
    packed: bool,
    fields: Vec<StructField>,
    span: Span,
}

#[derive(Debug, Clone)]
struct RawFunction {
    public: bool,
    interrupt: Option<InterruptClass>,
    params: Vec<Param>,
    return_type: TypeAnnotation,
    span: Span,
}

#[derive(Debug, Clone)]
struct RawConst {
    public: bool,
    ty: TypeAnnotation,
    value: Expr,
    span: Span,
}

#[derive(Debug, Clone)]
struct RawStatic {
    public: bool,
    mutable: bool,
    ty: TypeAnnotation,
    value: Option<Expr>,
    span: Span,
}

#[derive(Debug, Clone, Default)]
struct ModuleHeader {
    imports: BTreeMap<String, usize>,
    structs: BTreeMap<String, StructHeader>,
    functions: BTreeMap<String, RawFunction>,
    constants: BTreeMap<String, RawConst>,
    statics: BTreeMap<String, RawStatic>,
}

#[derive(Debug, Clone)]
struct FunctionSignature {
    public: bool,
    interrupt: Option<InterruptClass>,
    symbol: String,
    params: Vec<SystemType>,
    return_type: SystemType,
    span: Span,
}

#[derive(Debug, Clone)]
struct ConstantDecl {
    public: bool,
    ty: SystemType,
    value: Expr,
    span: Span,
}

#[derive(Debug, Clone)]
struct GlobalSignature {
    public: bool,
    mutable: bool,
    symbol: String,
    ty: SystemType,
    value: Option<Expr>,
    span: Span,
}

#[derive(Debug, Clone, Default)]
struct ModuleSymbols {
    functions: BTreeMap<String, FunctionSignature>,
    constants: BTreeMap<String, ConstantDecl>,
    globals: BTreeMap<String, GlobalSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConstantValue {
    ty: SystemType,
    bits: u32,
}

struct ProjectContext<'a> {
    graph: &'a NativeModuleGraph,
    headers: Vec<ModuleHeader>,
    symbols: Vec<ModuleSymbols>,
    layouts: TypeLayouts,
    constants: BTreeMap<(usize, String), ConstantValue>,
}

/// Analyze all native modules in two project-wide passes.
///
/// The declaration pass fixes nominal types, layouts, symbols, visibility, and
/// constant values before any function body is compiled. The body pass can
/// therefore resolve forward and cross-module references deterministically.
pub fn analyze_systems_project(graph: &NativeModuleGraph) -> NativeResult<SystemsProjectIr> {
    analyze_systems_graph(graph, true)
}

/// Analyze a module graph for relocatable-object emission.
///
/// Object files may intentionally provide only library functions or hardware
/// shims, so this mode does not require the graph root to define
/// `kernel_main`. All other declaration, visibility, type, and body checks are
/// identical to integrated project analysis.
pub fn analyze_systems_objects(graph: &NativeModuleGraph) -> NativeResult<SystemsProjectIr> {
    analyze_systems_graph(graph, false)
}

fn analyze_systems_graph(
    graph: &NativeModuleGraph,
    require_entry: bool,
) -> NativeResult<SystemsProjectIr> {
    validate_graph_shape(graph)?;
    let headers = collect_headers(graph)?;
    let layouts = build_project_layouts(graph, &headers)?;
    let symbols = resolve_module_symbols(graph, &headers, &layouts)?;
    validate_exports(graph, &symbols)?;
    if require_entry {
        validate_entry(graph, &symbols)?;
    }
    let constants = evaluate_project_constants(graph, &headers, &symbols, &layouts)?;
    let context = ProjectContext {
        graph,
        headers,
        symbols,
        layouts,
        constants,
    };

    let mut modules = Vec::with_capacity(graph.modules.len());
    for module in &graph.modules {
        modules.push(compile_module(&context, module)?);
    }
    Ok(SystemsProjectIr {
        root_module: graph.root,
        modules,
        layouts: context.layouts,
    })
}

fn validate_graph_shape(graph: &NativeModuleGraph) -> NativeResult<()> {
    if graph.modules.is_empty() || graph.root >= graph.modules.len() {
        return Err(Diagnostic::without_span(
            DiagnosticCategory::InvalidModuleImport,
            "native module graph has no valid root module",
        ));
    }
    let mut names = BTreeSet::new();
    for (index, module) in graph.modules.iter().enumerate() {
        if module.id != index {
            return Err(module_diagnostic(
                module,
                DiagnosticCategory::InvalidModuleImport,
                format!(
                    "native module '{}' has id {}, expected deterministic index {index}",
                    module.name, module.id
                ),
                Span::zero(),
            ));
        }
        if !names.insert(module.name.as_str()) {
            return Err(module_diagnostic(
                module,
                DiagnosticCategory::DuplicateModule,
                format!("duplicate native module name '{}'", module.name),
                Span::zero(),
            ));
        }
        let mut aliases = BTreeSet::new();
        for import in &module.imports {
            if import.target >= graph.modules.len() {
                return Err(module_diagnostic(
                    module,
                    DiagnosticCategory::InvalidModuleImport,
                    format!(
                        "native import alias '{}' has invalid target id {}",
                        import.alias, import.target
                    ),
                    import.span,
                ));
            }
            if import.alias == "native" || !aliases.insert(import.alias.as_str()) {
                return Err(module_diagnostic(
                    module,
                    DiagnosticCategory::InvalidModuleImport,
                    format!(
                        "invalid or duplicate native import alias '{}'",
                        import.alias
                    ),
                    import.span,
                ));
            }
        }
    }
    Ok(())
}

fn collect_headers(graph: &NativeModuleGraph) -> NativeResult<Vec<ModuleHeader>> {
    let mut headers = Vec::with_capacity(graph.modules.len());
    for module in &graph.modules {
        let mut header = ModuleHeader::default();
        for import in &module.imports {
            header.imports.insert(import.alias.clone(), import.target);
        }
        let mut declared_values = BTreeMap::<String, &'static str>::new();
        for statement in &module.program.stmts {
            match statement {
                Stmt::Import { .. } => {}
                Stmt::StructDef {
                    is_pub,
                    is_packed,
                    name,
                    fields,
                    span,
                } => {
                    insert_module_name(
                        module,
                        &mut declared_values,
                        name,
                        "struct constructor",
                        *span,
                    )?;
                    if header
                        .structs
                        .insert(
                            name.clone(),
                            StructHeader {
                                public: *is_pub,
                                packed: *is_packed,
                                fields: fields.clone(),
                                span: *span,
                            },
                        )
                        .is_some()
                    {
                        return Err(module_diagnostic(
                            module,
                            DiagnosticCategory::DuplicateType,
                            format!("duplicate native struct '{name}'"),
                            *span,
                        ));
                    }
                }
                Stmt::FnDef {
                    is_pub,
                    interrupt,
                    name,
                    params,
                    ret_ty,
                    span,
                    ..
                } => {
                    insert_module_name(module, &mut declared_values, name, "function", *span)?;
                    header.functions.insert(
                        name.clone(),
                        RawFunction {
                            public: *is_pub,
                            interrupt: *interrupt,
                            params: params.clone(),
                            return_type: ret_ty.clone(),
                            span: *span,
                        },
                    );
                }
                Stmt::Const {
                    is_pub,
                    name,
                    ty,
                    value,
                    span,
                } => {
                    insert_module_name(module, &mut declared_values, name, "constant", *span)?;
                    header.constants.insert(
                        name.clone(),
                        RawConst {
                            public: *is_pub,
                            ty: ty.clone(),
                            value: value.clone(),
                            span: *span,
                        },
                    );
                }
                Stmt::Static {
                    is_pub,
                    is_mut,
                    name,
                    ty,
                    value,
                    span,
                } => {
                    insert_module_name(module, &mut declared_values, name, "static", *span)?;
                    header.statics.insert(
                        name.clone(),
                        RawStatic {
                            public: *is_pub,
                            mutable: *is_mut,
                            ty: ty.clone(),
                            value: value.clone(),
                            span: *span,
                        },
                    );
                }
                other => {
                    return Err(module_diagnostic(
                        module,
                        DiagnosticCategory::UnsupportedFeature,
                        "native module top level admits only imports, structs, consts, statics, and functions",
                        other.span(),
                    ));
                }
            }
        }
        for (alias, target) in &header.imports {
            if declared_values.contains_key(alias) {
                return Err(module_diagnostic(
                    module,
                    DiagnosticCategory::DuplicateModuleSymbol,
                    format!(
                        "native import alias '{alias}' conflicts with a module symbol (target module {})",
                        graph.modules[*target].name
                    ),
                    module
                        .imports
                        .iter()
                        .find(|import| import.alias == *alias)
                        .map(|import| import.span)
                        .unwrap_or_else(Span::zero),
                ));
            }
        }
        headers.push(header);
    }
    Ok(headers)
}

fn insert_module_name(
    module: &NativeSourceModule,
    names: &mut BTreeMap<String, &'static str>,
    name: &str,
    kind: &'static str,
    span: Span,
) -> NativeResult<()> {
    if let Some(previous) = names.insert(name.to_owned(), kind) {
        return Err(module_diagnostic(
            module,
            DiagnosticCategory::DuplicateModuleSymbol,
            format!("duplicate module symbol '{name}' ({previous} conflicts with {kind})"),
            span,
        ));
    }
    Ok(())
}

fn build_project_layouts(
    graph: &NativeModuleGraph,
    headers: &[ModuleHeader],
) -> NativeResult<TypeLayouts> {
    let mut declarations = Vec::new();
    for module in &graph.modules {
        for (name, header) in &headers[module.id].structs {
            let mut fields = Vec::with_capacity(header.fields.len());
            for field in &header.fields {
                fields.push(StructFieldDecl {
                    name: field.name.clone(),
                    ty: resolve_annotation(graph, headers, module.id, &field.ty, field.span)?,
                    span: field.span,
                });
            }
            declarations.push(StructDecl {
                name: canonical_type_name(module, name),
                packed: header.packed,
                fields,
                span: header.span,
            });
        }
    }
    TypeLayouts::build(&declarations).map_err(|error| attach_layout_path(error, graph))
}

fn resolve_module_symbols(
    graph: &NativeModuleGraph,
    headers: &[ModuleHeader],
    layouts: &TypeLayouts,
) -> NativeResult<Vec<ModuleSymbols>> {
    let mut all = Vec::with_capacity(graph.modules.len());
    for module in &graph.modules {
        let header = &headers[module.id];
        let mut symbols = ModuleSymbols::default();
        for (name, raw) in &header.functions {
            let mut parameter_names = BTreeSet::new();
            let mut params = Vec::with_capacity(raw.params.len());
            for parameter in &raw.params {
                if !parameter_names.insert(parameter.name.as_str()) {
                    return Err(module_diagnostic(
                        module,
                        DiagnosticCategory::DuplicateLocal,
                        format!(
                            "duplicate parameter '{}' in native function '{name}'",
                            parameter.name
                        ),
                        parameter.span,
                    ));
                }
                let ty =
                    resolve_annotation(graph, headers, module.id, &parameter.ty, parameter.span)?;
                require_abi_scalar(module, &ty, parameter.span, "parameter")?;
                params.push(ty);
            }
            let return_type = if let Some(class) = raw.interrupt {
                if !matches!(raw.return_type, TypeAnnotation::Inferred) {
                    return Err(module_diagnostic(
                        module,
                        DiagnosticCategory::InvalidReturn,
                        "interrupt functions cannot declare a return type",
                        raw.span,
                    ));
                }
                validate_interrupt_signature(
                    module,
                    layouts,
                    class,
                    &params,
                    &raw.params,
                    raw.span,
                )?;
                SystemType::I32
            } else {
                let ty = resolve_annotation(graph, headers, module.id, &raw.return_type, raw.span)?;
                require_abi_scalar(module, &ty, raw.span, "return type")?;
                ty
            };
            symbols.functions.insert(
                name.clone(),
                FunctionSignature {
                    public: raw.public,
                    interrupt: raw.interrupt,
                    symbol: emitted_symbol(graph, module.id, name, raw.public),
                    params,
                    return_type,
                    span: raw.span,
                },
            );
        }
        for (name, raw) in &header.constants {
            let ty = resolve_annotation(graph, headers, module.id, &raw.ty, raw.span)?;
            if !ty.is_scalar() || matches!(ty, SystemType::Str) {
                return Err(module_diagnostic(
                    module,
                    DiagnosticCategory::InvalidType,
                    format!(
                        "native constant '{name}' requires a scalar non-string type, found {ty}"
                    ),
                    raw.span,
                ));
            }
            symbols.constants.insert(
                name.clone(),
                ConstantDecl {
                    public: raw.public,
                    ty,
                    value: raw.value.clone(),
                    span: raw.span,
                },
            );
        }
        for (name, raw) in &header.statics {
            let ty = resolve_annotation(graph, headers, module.id, &raw.ty, raw.span)?;
            let _ = layouts
                .layout_of(&ty, raw.span)
                .map_err(|error| attach_module_path(error, module))?;
            if !raw.mutable && raw.value.is_none() {
                return Err(module_diagnostic(
                    module,
                    DiagnosticCategory::InvalidType,
                    format!("immutable native static '{name}' requires an initializer"),
                    raw.span,
                ));
            }
            symbols.globals.insert(
                name.clone(),
                GlobalSignature {
                    public: raw.public,
                    mutable: raw.mutable,
                    symbol: emitted_symbol(graph, module.id, name, raw.public),
                    ty,
                    value: raw.value.clone(),
                    span: raw.span,
                },
            );
        }
        all.push(symbols);
    }
    Ok(all)
}

fn validate_exports(graph: &NativeModuleGraph, symbols: &[ModuleSymbols]) -> NativeResult<()> {
    let mut exports = BTreeMap::<String, (usize, &'static str)>::new();
    let mut emitted = BTreeMap::<String, (usize, &'static str)>::new();
    for module in &graph.modules {
        for (name, function) in &symbols[module.id].functions {
            insert_emitted_symbol(
                graph,
                &mut emitted,
                module.id,
                &function.symbol,
                "function",
                function.span,
            )?;
            if function.public {
                insert_export(
                    graph,
                    &mut exports,
                    module.id,
                    name,
                    "function",
                    function.span,
                )?;
            }
        }
        for (name, constant) in &symbols[module.id].constants {
            if constant.public {
                insert_export(
                    graph,
                    &mut exports,
                    module.id,
                    name,
                    "constant",
                    constant.span,
                )?;
            }
        }
        for (name, global) in &symbols[module.id].globals {
            insert_emitted_symbol(
                graph,
                &mut emitted,
                module.id,
                &global.symbol,
                "static",
                global.span,
            )?;
            if global.public {
                insert_export(graph, &mut exports, module.id, name, "static", global.span)?;
            }
        }
    }

    Ok(())
}

fn insert_emitted_symbol(
    graph: &NativeModuleGraph,
    emitted: &mut BTreeMap<String, (usize, &'static str)>,
    module_id: usize,
    symbol: &str,
    kind: &'static str,
    span: Span,
) -> NativeResult<()> {
    if let Some((previous_module, previous_kind)) =
        emitted.insert(symbol.to_owned(), (module_id, kind))
    {
        return Err(module_diagnostic(
            &graph.modules[module_id],
            DiagnosticCategory::DuplicateModuleSymbol,
            format!(
                "duplicate emitted native symbol '{symbol}': {previous_kind} in module '{}' conflicts with {kind} in module '{}'",
                graph.modules[previous_module].name, graph.modules[module_id].name
            ),
            span,
        ));
    }
    Ok(())
}

fn validate_entry(graph: &NativeModuleGraph, symbols: &[ModuleSymbols]) -> NativeResult<()> {
    let root = graph.root_module();
    let entry = symbols[root.id]
        .functions
        .get(ENTRY_FUNCTION)
        .ok_or_else(|| {
            module_diagnostic(
                root,
                DiagnosticCategory::InvalidEntryPoint,
                "missing native entry function 'kernel_main'",
                Span::zero(),
            )
        })?;
    let legacy = entry.params.is_empty();
    let multiboot_v1 = entry.params.len() == 2
        && entry.params[0] == SystemType::U32
        && entry.params[1] == SystemType::Usize;
    if (!legacy && !multiboot_v1) || entry.return_type != SystemType::I32 {
        return Err(module_diagnostic(
            root,
            DiagnosticCategory::InvalidEntryPoint,
            format!(
                "native entry 'kernel_main' must have signature fn kernel_main() -> int/i32 or fn kernel_main(multiboot_magic: u32, multiboot_info: usize) -> int/i32, found {} parameter(s) and return type {}",
                entry.params.len(), entry.return_type
            ),
            entry.span,
        ));
    }
    Ok(())
}

fn insert_export(
    graph: &NativeModuleGraph,
    exports: &mut BTreeMap<String, (usize, &'static str)>,
    module_id: usize,
    name: &str,
    kind: &'static str,
    span: Span,
) -> NativeResult<()> {
    if let Some((previous_module, previous_kind)) =
        exports.insert(name.to_owned(), (module_id, kind))
    {
        return Err(module_diagnostic(
            &graph.modules[module_id],
            DiagnosticCategory::DuplicateModuleSymbol,
            format!(
                "duplicate public native symbol '{name}': {previous_kind} in module '{}' conflicts with {kind} in module '{}'",
                graph.modules[previous_module].name, graph.modules[module_id].name
            ),
            span,
        ));
    }
    Ok(())
}

fn resolve_annotation(
    graph: &NativeModuleGraph,
    headers: &[ModuleHeader],
    module_id: usize,
    annotation: &TypeAnnotation,
    span: Span,
) -> NativeResult<SystemType> {
    let primitive = match annotation {
        TypeAnnotation::Int | TypeAnnotation::I32 => Some(SystemType::I32),
        TypeAnnotation::I8 => Some(SystemType::I8),
        TypeAnnotation::U8 => Some(SystemType::U8),
        TypeAnnotation::I16 => Some(SystemType::I16),
        TypeAnnotation::U16 => Some(SystemType::U16),
        TypeAnnotation::U32 => Some(SystemType::U32),
        TypeAnnotation::Usize => Some(SystemType::Usize),
        TypeAnnotation::Bool => Some(SystemType::Bool),
        TypeAnnotation::Str => Some(SystemType::Str),
        _ => None,
    };
    if let Some(primitive) = primitive {
        return Ok(primitive);
    }
    match annotation {
        TypeAnnotation::Pointer(inner) => Ok(SystemType::Pointer(Box::new(resolve_annotation(
            graph, headers, module_id, inner, span,
        )?))),
        TypeAnnotation::Array { element, length } => Ok(SystemType::Array(
            Box::new(resolve_annotation(
                graph, headers, module_id, element, span,
            )?),
            *length,
        )),
        TypeAnnotation::Named(name) => resolve_named_type(graph, headers, module_id, name, span),
        TypeAnnotation::Float
        | TypeAnnotation::Bytes
        | TypeAnnotation::Buffer
        | TypeAnnotation::List(_)
        | TypeAnnotation::Inferred => Err(module_diagnostic(
            &graph.modules[module_id],
            DiagnosticCategory::InvalidType,
            format!("unsupported or inferred native type '{annotation}'"),
            span,
        )),
        _ => unreachable!("primitive native annotation handled above"),
    }
}

fn resolve_named_type(
    graph: &NativeModuleGraph,
    headers: &[ModuleHeader],
    module_id: usize,
    name: &str,
    span: Span,
) -> NativeResult<SystemType> {
    if let Some((alias, imported_name)) = name.split_once('.') {
        let target = headers[module_id]
            .imports
            .get(alias)
            .copied()
            .ok_or_else(|| {
                module_diagnostic(
                    &graph.modules[module_id],
                    DiagnosticCategory::UnknownType,
                    format!("unknown native module alias '{alias}' in type '{name}'"),
                    span,
                )
            })?;
        let header = headers[target].structs.get(imported_name).ok_or_else(|| {
            module_diagnostic(
                &graph.modules[module_id],
                DiagnosticCategory::UnknownType,
                format!(
                    "module '{}' has no native struct '{}'",
                    graph.modules[target].name, imported_name
                ),
                span,
            )
        })?;
        if !header.public {
            return Err(module_diagnostic(
                &graph.modules[module_id],
                DiagnosticCategory::UnknownType,
                format!(
                    "native struct '{}.{}' is module-private",
                    graph.modules[target].name, imported_name
                ),
                span,
            ));
        }
        Ok(SystemType::Struct(canonical_type_name(
            &graph.modules[target],
            imported_name,
        )))
    } else if headers[module_id].structs.contains_key(name) {
        Ok(SystemType::Struct(canonical_type_name(
            &graph.modules[module_id],
            name,
        )))
    } else {
        Err(module_diagnostic(
            &graph.modules[module_id],
            DiagnosticCategory::UnknownType,
            format!("unknown native type '{name}'"),
            span,
        ))
    }
}

fn require_abi_scalar(
    module: &NativeSourceModule,
    ty: &SystemType,
    span: Span,
    role: &str,
) -> NativeResult<()> {
    if ty.is_scalar() {
        Ok(())
    } else {
        Err(module_diagnostic(
            module,
            DiagnosticCategory::InvalidType,
            format!("native ABI {role} must be scalar, found {ty}"),
            span,
        ))
    }
}

fn validate_interrupt_signature(
    module: &NativeSourceModule,
    layouts: &TypeLayouts,
    class: InterruptClass,
    params: &[SystemType],
    raw_params: &[Param],
    span: Span,
) -> NativeResult<()> {
    let expected_count = match class {
        InterruptClass::NoErrorCode => 1,
        InterruptClass::ErrorCode => 2,
    };
    if params.len() != expected_count {
        return Err(module_diagnostic(
            module,
            DiagnosticCategory::WrongArity,
            format!(
                "interrupt({class}) function expects {expected_count} parameters, got {}",
                params.len()
            ),
            span,
        ));
    }
    let Some(SystemType::Pointer(frame)) = params.first() else {
        return Err(module_diagnostic(
            module,
            DiagnosticCategory::InvalidType,
            "interrupt frame parameter must be a pointer to InterruptFrame",
            raw_params.first().map_or(span, |param| param.span),
        ));
    };
    let SystemType::Struct(frame_name) = frame.as_ref() else {
        return Err(module_diagnostic(
            module,
            DiagnosticCategory::InvalidType,
            "interrupt frame parameter must point to a struct",
            raw_params[0].span,
        ));
    };
    let layout = layouts.structure(frame_name).ok_or_else(|| {
        module_diagnostic(
            module,
            DiagnosticCategory::UnknownType,
            format!("unknown interrupt frame type '{frame_name}'"),
            raw_params[0].span,
        )
    })?;
    let valid_frame = layout.size == 12
        && layout.fields.len() == 3
        && layout
            .fields
            .iter()
            .zip(["eip", "cs", "eflags"])
            .all(|(field, name)| field.name == name && field.ty == SystemType::U32);
    if !valid_frame {
        return Err(module_diagnostic(
            module,
            DiagnosticCategory::InvalidType,
            "InterruptFrame must contain declaration-order u32 fields eip, cs, eflags",
            raw_params[0].span,
        ));
    }
    if class == InterruptClass::ErrorCode && params[1] != SystemType::U32 {
        return Err(module_diagnostic(
            module,
            DiagnosticCategory::InvalidType,
            "interrupt(error_code) second parameter must be u32",
            raw_params[1].span,
        ));
    }
    Ok(())
}

fn emitted_symbol(graph: &NativeModuleGraph, module_id: usize, name: &str, public: bool) -> String {
    if public || (module_id == graph.root && name == ENTRY_FUNCTION) {
        name.to_owned()
    } else {
        format!("__dbyte_{}_{}", graph.modules[module_id].name, name)
    }
}

fn canonical_type_name(module: &NativeSourceModule, name: &str) -> String {
    format!("{}::{name}", module.name)
}

fn module_diagnostic(
    module: &NativeSourceModule,
    category: DiagnosticCategory,
    message: impl Into<String>,
    span: Span,
) -> Diagnostic {
    Diagnostic::new(category, message, span).with_path(module.display_path.clone())
}

fn attach_module_path(mut error: Diagnostic, module: &NativeSourceModule) -> Diagnostic {
    if error.path.is_none() {
        error.path = Some(module.display_path.clone());
    }
    error
}

fn attach_layout_path(mut error: Diagnostic, graph: &NativeModuleGraph) -> Diagnostic {
    if error.path.is_none() {
        error.path = Some(graph.root_module().display_path.clone());
    }
    error
}

fn evaluate_project_constants(
    graph: &NativeModuleGraph,
    headers: &[ModuleHeader],
    symbols: &[ModuleSymbols],
    layouts: &TypeLayouts,
) -> NativeResult<BTreeMap<(usize, String), ConstantValue>> {
    let mut evaluator = ConstantEvaluator {
        graph,
        headers,
        symbols,
        layouts,
        values: BTreeMap::new(),
        visiting: BTreeSet::new(),
    };
    for module in &graph.modules {
        for name in symbols[module.id].constants.keys() {
            evaluator.evaluate_named(module.id, name)?;
        }
    }
    Ok(evaluator.values)
}

struct ConstantEvaluator<'a> {
    graph: &'a NativeModuleGraph,
    headers: &'a [ModuleHeader],
    symbols: &'a [ModuleSymbols],
    layouts: &'a TypeLayouts,
    values: BTreeMap<(usize, String), ConstantValue>,
    visiting: BTreeSet<(usize, String)>,
}

impl ConstantEvaluator<'_> {
    fn evaluate_named(&mut self, module_id: usize, name: &str) -> NativeResult<ConstantValue> {
        let key = (module_id, name.to_owned());
        if let Some(value) = self.values.get(&key) {
            return Ok(value.clone());
        }
        let declaration = self.symbols[module_id]
            .constants
            .get(name)
            .cloned()
            .ok_or_else(|| {
                module_diagnostic(
                    &self.graph.modules[module_id],
                    DiagnosticCategory::UnknownSymbol,
                    format!("unknown native constant '{name}'"),
                    Span::zero(),
                )
            })?;
        if !self.visiting.insert(key.clone()) {
            return Err(module_diagnostic(
                &self.graph.modules[module_id],
                DiagnosticCategory::InvalidType,
                format!("native constant cycle contains '{name}'"),
                declaration.span,
            ));
        }
        let value = self.evaluate_expr(module_id, &declaration.value, Some(&declaration.ty))?;
        if value.ty != declaration.ty {
            return Err(module_diagnostic(
                &self.graph.modules[module_id],
                DiagnosticCategory::InvalidType,
                format!(
                    "native constant '{name}' expects {}, found {}",
                    declaration.ty, value.ty
                ),
                declaration.span,
            ));
        }
        self.visiting.remove(&key);
        self.values.insert(key, value.clone());
        Ok(value)
    }

    fn evaluate_expr(
        &mut self,
        module_id: usize,
        expression: &Expr,
        expected: Option<&SystemType>,
    ) -> NativeResult<ConstantValue> {
        let module = &self.graph.modules[module_id];
        match expression {
            Expr::IntLit(value, span) => {
                let ty = expected
                    .filter(|ty| ty.is_integer())
                    .cloned()
                    .unwrap_or(SystemType::I32);
                let bits = checked_literal_bits(*value, &ty, *span)
                    .map_err(|error| attach_module_path(error, module))?;
                Ok(ConstantValue { ty, bits })
            }
            Expr::BoolLit(value, span) => {
                if expected.is_some_and(|ty| *ty != SystemType::Bool) {
                    return Err(module_diagnostic(
                        module,
                        DiagnosticCategory::InvalidType,
                        format!(
                            "boolean constant cannot initialize native type '{}'",
                            expected.expect("checked Some")
                        ),
                        *span,
                    ));
                }
                Ok(ConstantValue {
                    ty: SystemType::Bool,
                    bits: u32::from(*value),
                })
            }
            Expr::Ident(name, span) => self.evaluate_identifier(module_id, name, *span),
            Expr::Member {
                object,
                property,
                span,
            } => {
                let Expr::Ident(alias, _) = object.as_ref() else {
                    return Err(module_diagnostic(
                        module,
                        DiagnosticCategory::UnknownSymbol,
                        "compile-time module member requires an import alias",
                        *span,
                    ));
                };
                let target = self.import_target(module_id, alias, *span)?;
                let declaration =
                    self.symbols[target]
                        .constants
                        .get(property)
                        .ok_or_else(|| {
                            module_diagnostic(
                                module,
                                DiagnosticCategory::UnknownSymbol,
                                format!(
                                    "module '{}' has no public constant '{property}'",
                                    self.graph.modules[target].name
                                ),
                                *span,
                            )
                        })?;
                if !declaration.public {
                    return Err(module_diagnostic(
                        module,
                        DiagnosticCategory::UnknownSymbol,
                        format!(
                            "native constant '{}.{property}' is module-private",
                            self.graph.modules[target].name
                        ),
                        *span,
                    ));
                }
                self.evaluate_named(target, property)
            }
            Expr::Cast { expr, ty, span } => {
                let target = resolve_annotation(self.graph, self.headers, module_id, ty, *span)?;
                if let Some(literal) = signed_literal(expression_without_cast(expr)) {
                    if target.is_integer() {
                        let bits = checked_literal_bits(literal, &target, *span)
                            .map_err(|error| attach_module_path(error, module))?;
                        return Ok(ConstantValue { ty: target, bits });
                    }
                }
                let source = self.evaluate_expr(module_id, expr, None)?;
                if !explicit_cast_allowed(&source.ty, &target) {
                    return Err(module_diagnostic(
                        module,
                        DiagnosticCategory::InvalidNarrowing,
                        format!("invalid native cast from {} to {target}", source.ty),
                        *span,
                    ));
                }
                let bits = checked_constant_cast_bits(&source, &target, *span)
                    .map_err(|error| attach_module_path(error, module))?;
                Ok(ConstantValue { ty: target, bits })
            }
            Expr::Unary { op, expr, span } => {
                let value = self.evaluate_expr(module_id, expr, expected)?;
                match op {
                    UnaryOp::Neg if value.ty.is_signed_integer() => Ok(ConstantValue {
                        bits: normalize_integer_bits((0u32).wrapping_sub(value.bits), &value.ty),
                        ty: value.ty,
                    }),
                    UnaryOp::BitNot if value.ty.is_integer() => Ok(ConstantValue {
                        bits: normalize_integer_bits(!value.bits, &value.ty),
                        ty: value.ty,
                    }),
                    UnaryOp::Not if value.ty == SystemType::Bool => Ok(ConstantValue {
                        ty: SystemType::Bool,
                        bits: u32::from(value.bits == 0),
                    }),
                    _ => Err(module_diagnostic(
                        module,
                        DiagnosticCategory::InvalidType,
                        format!(
                            "operator is invalid for compile-time native type '{}'",
                            value.ty
                        ),
                        *span,
                    )),
                }
            }
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => {
                let operand_expected = expected.filter(|ty| ty.is_integer());
                let left = self.evaluate_expr(module_id, left, operand_expected)?;
                let right = self.evaluate_expr(module_id, right, Some(&left.ty))?;
                evaluate_constant_binary(module, left, op, right, *span)
            }
            Expr::Call { name, args, span } if matches!(name.as_str(), "size_of" | "align_of") => {
                if args.len() != 1 {
                    return Err(module_diagnostic(
                        module,
                        DiagnosticCategory::WrongArity,
                        format!("{name} expects exactly one type argument"),
                        *span,
                    ));
                }
                let annotation = type_annotation_from_expr(&args[0]).ok_or_else(|| {
                    module_diagnostic(
                        module,
                        DiagnosticCategory::InvalidType,
                        format!("{name} expects a native type name"),
                        args[0].span(),
                    )
                })?;
                let ty =
                    resolve_annotation(self.graph, self.headers, module_id, &annotation, *span)?;
                let layout = self
                    .layouts
                    .layout_of(&ty, *span)
                    .map_err(|error| attach_module_path(error, module))?;
                Ok(ConstantValue {
                    ty: SystemType::Usize,
                    bits: if name == "size_of" {
                        layout.size
                    } else {
                        layout.alignment
                    },
                })
            }
            _ => Err(module_diagnostic(
                module,
                DiagnosticCategory::InvalidType,
                "native compile-time expression is not deterministic or scalar",
                expression.span(),
            )),
        }
    }

    fn evaluate_identifier(
        &mut self,
        module_id: usize,
        name: &str,
        span: Span,
    ) -> NativeResult<ConstantValue> {
        if self.symbols[module_id].constants.contains_key(name) {
            self.evaluate_named(module_id, name)
        } else {
            Err(module_diagnostic(
                &self.graph.modules[module_id],
                DiagnosticCategory::UnknownSymbol,
                format!("unknown compile-time native symbol '{name}'"),
                span,
            ))
        }
    }

    fn import_target(&self, module_id: usize, alias: &str, span: Span) -> NativeResult<usize> {
        self.headers[module_id]
            .imports
            .get(alias)
            .copied()
            .ok_or_else(|| {
                module_diagnostic(
                    &self.graph.modules[module_id],
                    DiagnosticCategory::InvalidModuleImport,
                    format!("unknown native import alias '{alias}'"),
                    span,
                )
            })
    }
}

fn evaluate_constant_binary(
    module: &NativeSourceModule,
    left: ConstantValue,
    operator: &BinOp,
    right: ConstantValue,
    span: Span,
) -> NativeResult<ConstantValue> {
    if left.ty != right.ty {
        return Err(module_diagnostic(
            module,
            DiagnosticCategory::InvalidType,
            format!(
                "compile-time operator '{operator}' requires matching types, found {} and {}",
                left.ty, right.ty
            ),
            span,
        ));
    }
    let ty = left.ty.clone();
    match operator {
        BinOp::Add | BinOp::Sub | BinOp::Mul if ty.is_integer() => {
            let raw = match operator {
                BinOp::Add => left.bits.wrapping_add(right.bits),
                BinOp::Sub => left.bits.wrapping_sub(right.bits),
                BinOp::Mul => left.bits.wrapping_mul(right.bits),
                _ => unreachable!(),
            };
            Ok(ConstantValue {
                bits: normalize_integer_bits(raw, &ty),
                ty,
            })
        }
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor if ty.is_integer() => {
            let raw = match operator {
                BinOp::BitAnd => left.bits & right.bits,
                BinOp::BitOr => left.bits | right.bits,
                BinOp::BitXor => left.bits ^ right.bits,
                _ => unreachable!(),
            };
            Ok(ConstantValue {
                bits: normalize_integer_bits(raw, &ty),
                ty,
            })
        }
        BinOp::Shl | BinOp::Shr if ty.is_integer() => {
            let width = u32::from(ty.bit_width().expect("integer width"));
            if right.bits >= width {
                return Err(module_diagnostic(
                    module,
                    DiagnosticCategory::InvalidType,
                    format!(
                        "shift count {} is outside 0..{} for {ty}",
                        right.bits,
                        width - 1
                    ),
                    span,
                ));
            }
            let raw = if matches!(operator, BinOp::Shl) {
                left.bits.wrapping_shl(right.bits)
            } else if ty.is_signed_integer() {
                (signed_integer_value(left.bits, &ty) >> right.bits) as u32
            } else {
                left.bits >> right.bits
            };
            Ok(ConstantValue {
                bits: normalize_integer_bits(raw, &ty),
                ty,
            })
        }
        BinOp::EqEq | BinOp::NotEq => {
            let equal = left.bits == right.bits;
            Ok(ConstantValue {
                ty: SystemType::Bool,
                bits: u32::from(if matches!(operator, BinOp::EqEq) {
                    equal
                } else {
                    !equal
                }),
            })
        }
        BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq if ty.is_integer() => {
            let ordering = if ty.is_signed_integer() {
                signed_integer_value(left.bits, &ty).cmp(&signed_integer_value(right.bits, &ty))
            } else {
                left.bits.cmp(&right.bits)
            };
            let result = match operator {
                BinOp::Lt => ordering.is_lt(),
                BinOp::LtEq => !ordering.is_gt(),
                BinOp::Gt => ordering.is_gt(),
                BinOp::GtEq => !ordering.is_lt(),
                _ => unreachable!(),
            };
            Ok(ConstantValue {
                ty: SystemType::Bool,
                bits: u32::from(result),
            })
        }
        BinOp::Div => Err(module_diagnostic(
            module,
            DiagnosticCategory::UnsupportedFeature,
            "integer division is unsupported in the native systems foundation",
            span,
        )),
        _ => Err(module_diagnostic(
            module,
            DiagnosticCategory::InvalidType,
            format!("operator '{operator}' is invalid for compile-time native type {ty}"),
            span,
        )),
    }
}

fn normalize_integer_bits(bits: u32, ty: &SystemType) -> u32 {
    match ty.bit_width() {
        Some(8) => {
            let narrowed = bits as u8;
            if ty.is_signed_integer() {
                (narrowed as i8 as i32) as u32
            } else {
                u32::from(narrowed)
            }
        }
        Some(16) => {
            let narrowed = bits as u16;
            if ty.is_signed_integer() {
                (narrowed as i16 as i32) as u32
            } else {
                u32::from(narrowed)
            }
        }
        _ => bits,
    }
}

fn signed_integer_value(bits: u32, ty: &SystemType) -> i64 {
    match ty.bit_width() {
        Some(8) => i64::from(bits as u8 as i8),
        Some(16) => i64::from(bits as u16 as i16),
        _ => i64::from(bits as i32),
    }
}

fn checked_constant_cast_bits(
    source: &ConstantValue,
    target: &SystemType,
    span: Span,
) -> NativeResult<u32> {
    if source.ty.is_integer() && target.is_integer() {
        let value = if source.ty.is_signed_integer() {
            signed_integer_value(source.bits, &source.ty)
        } else {
            i64::from(source.bits)
        };
        checked_literal_bits(value, target, span)
    } else {
        Ok(source.bits)
    }
}

fn signed_literal(expression: &Expr) -> Option<i64> {
    match expression {
        Expr::IntLit(value, _) => Some(*value),
        Expr::Unary {
            op: UnaryOp::Neg,
            expr,
            ..
        } => match expr.as_ref() {
            Expr::IntLit(value, _) => value.checked_neg(),
            _ => None,
        },
        _ => None,
    }
}

fn expression_without_cast(expression: &Expr) -> &Expr {
    expression
}

fn type_annotation_from_expr(expression: &Expr) -> Option<TypeAnnotation> {
    match expression {
        Expr::Ident(name, _) => Some(match name.as_str() {
            "int" => TypeAnnotation::Int,
            "i8" => TypeAnnotation::I8,
            "u8" => TypeAnnotation::U8,
            "i16" => TypeAnnotation::I16,
            "u16" => TypeAnnotation::U16,
            "i32" => TypeAnnotation::I32,
            "u32" => TypeAnnotation::U32,
            "usize" => TypeAnnotation::Usize,
            "bool" => TypeAnnotation::Bool,
            "str" => TypeAnnotation::Str,
            other => TypeAnnotation::Named(other.to_owned()),
        }),
        Expr::Member {
            object, property, ..
        } => match object.as_ref() {
            Expr::Ident(alias, _) => Some(TypeAnnotation::Named(format!("{alias}.{property}"))),
            _ => None,
        },
        Expr::Index { target, index, .. } => match target.as_ref() {
            Expr::Ident(name, _) if name == "ptr" => type_annotation_from_expr(index)
                .map(|inner| TypeAnnotation::Pointer(Box::new(inner))),
            _ => None,
        },
        _ => None,
    }
}

fn compile_module(
    context: &ProjectContext<'_>,
    module: &NativeSourceModule,
) -> NativeResult<SystemsModuleIr> {
    let mut globals = Vec::new();
    for statement in &module.program.stmts {
        if let Stmt::Static { name, .. } = statement {
            let signature = context.symbols[module.id]
                .globals
                .get(name)
                .expect("static header was collected");
            globals.push(compile_global(context, module.id, name, signature)?);
        }
    }

    let mut strings = Vec::new();
    let mut string_ids = BTreeMap::new();
    let mut external_functions = BTreeMap::<String, ExternalFunction>::new();
    let mut external_globals = BTreeMap::<String, ExternalGlobal>::new();
    let mut functions = Vec::new();
    for statement in &module.program.stmts {
        let Stmt::FnDef {
            name, params, body, ..
        } = statement
        else {
            continue;
        };
        let signature = context.symbols[module.id]
            .functions
            .get(name)
            .expect("function header was collected")
            .clone();
        let analyzer = FunctionAnalyzer::new(
            context,
            module.id,
            name,
            signature,
            &mut strings,
            &mut string_ids,
            &mut external_functions,
            &mut external_globals,
        );
        functions.push(analyzer.compile(params, body)?);
    }

    Ok(SystemsModuleIr {
        module_name: module.name.clone(),
        functions,
        globals,
        strings,
        external_functions: external_functions.into_values().collect(),
        external_globals: external_globals.into_values().collect(),
    })
}

fn compile_global(
    context: &ProjectContext<'_>,
    module_id: usize,
    name: &str,
    signature: &GlobalSignature,
) -> NativeResult<SystemsGlobal> {
    let module = &context.graph.modules[module_id];
    let layout = context
        .layouts
        .layout_of(&signature.ty, signature.span)
        .map_err(|error| attach_module_path(error, module))?;
    let (section, bytes) = match &signature.value {
        None => (GlobalSection::Bss, Vec::new()),
        Some(initializer) => {
            let bytes = encode_static_initializer(
                context,
                module_id,
                &signature.ty,
                initializer,
                signature.span,
            )?;
            let section = if signature.mutable {
                GlobalSection::Data
            } else {
                GlobalSection::ReadOnly
            };
            (section, bytes)
        }
    };
    Ok(SystemsGlobal {
        source_name: name.to_owned(),
        symbol: signature.symbol.clone(),
        visibility: visibility(signature.public),
        ty: signature.ty.clone(),
        mutable: signature.mutable,
        section,
        bytes,
        size: layout.size,
        alignment: layout.alignment,
        span: signature.span,
    })
}

fn encode_static_initializer(
    context: &ProjectContext<'_>,
    module_id: usize,
    expected: &SystemType,
    expression: &Expr,
    span: Span,
) -> NativeResult<Vec<u8>> {
    let module = &context.graph.modules[module_id];
    let layout = context
        .layouts
        .layout_of(expected, span)
        .map_err(|error| attach_module_path(error, module))?;
    let mut bytes = vec![0; layout.size as usize];
    encode_static_value(context, module_id, expected, expression, 0, &mut bytes)?;
    Ok(bytes)
}

fn encode_static_value(
    context: &ProjectContext<'_>,
    module_id: usize,
    expected: &SystemType,
    expression: &Expr,
    offset: u32,
    bytes: &mut [u8],
) -> NativeResult<()> {
    let module = &context.graph.modules[module_id];
    match expected {
        SystemType::Array(element, length) => {
            let Expr::List(items, list_span) = expression else {
                return Err(module_diagnostic(
                    module,
                    DiagnosticCategory::InvalidType,
                    format!("array static initializer for {expected} requires an array literal"),
                    expression.span(),
                ));
            };
            if items.len() != *length as usize {
                return Err(module_diagnostic(
                    module,
                    DiagnosticCategory::BadArrayLength,
                    format!(
                        "array initializer for {expected} expects {length} elements, got {}",
                        items.len()
                    ),
                    *list_span,
                ));
            }
            let element_layout = context
                .layouts
                .layout_of(element, *list_span)
                .map_err(|error| attach_module_path(error, module))?;
            for (index, item) in items.iter().enumerate() {
                let item_offset = offset
                    .checked_add(element_layout.size.saturating_mul(index as u32))
                    .ok_or_else(|| {
                        module_diagnostic(
                            module,
                            DiagnosticCategory::LayoutOverflow,
                            "array initializer offset overflow",
                            item.span(),
                        )
                    })?;
                encode_static_value(context, module_id, element, item, item_offset, bytes)?;
            }
            Ok(())
        }
        SystemType::Struct(name) => {
            let args = struct_initializer_args(context, module_id, name, expression)?;
            let layout = context.layouts.structure(name).ok_or_else(|| {
                module_diagnostic(
                    module,
                    DiagnosticCategory::UnknownType,
                    format!("unknown native struct '{name}'"),
                    expression.span(),
                )
            })?;
            if args.len() != layout.fields.len() {
                return Err(module_diagnostic(
                    module,
                    DiagnosticCategory::WrongArity,
                    format!(
                        "struct constructor for {name} expects {} fields, got {}",
                        layout.fields.len(),
                        args.len()
                    ),
                    expression.span(),
                ));
            }
            let fields = layout.fields.clone();
            for (field, argument) in fields.iter().zip(args) {
                let field_offset = offset.checked_add(field.offset).ok_or_else(|| {
                    module_diagnostic(
                        module,
                        DiagnosticCategory::LayoutOverflow,
                        "struct initializer offset overflow",
                        argument.span(),
                    )
                })?;
                encode_static_value(context, module_id, &field.ty, argument, field_offset, bytes)?;
            }
            Ok(())
        }
        SystemType::Str | SystemType::Pointer(_) => Err(module_diagnostic(
            module,
            DiagnosticCategory::UnsupportedFeature,
            format!("static relocation initializer for native type {expected} is not supported"),
            expression.span(),
        )),
        _ => {
            let mut evaluator = ConstantEvaluator {
                graph: context.graph,
                headers: &context.headers,
                symbols: &context.symbols,
                layouts: &context.layouts,
                values: context.constants.clone(),
                visiting: BTreeSet::new(),
            };
            let value = evaluator.evaluate_expr(module_id, expression, Some(expected))?;
            if value.ty != *expected {
                return Err(module_diagnostic(
                    module,
                    DiagnosticCategory::InvalidType,
                    format!("static initializer expects {expected}, found {}", value.ty),
                    expression.span(),
                ));
            }
            let size = expected.scalar_size().expect("scalar initializer size") as usize;
            let start = offset as usize;
            let end = start.checked_add(size).ok_or_else(|| {
                module_diagnostic(
                    module,
                    DiagnosticCategory::LayoutOverflow,
                    "static initializer byte offset overflow",
                    expression.span(),
                )
            })?;
            let destination = bytes.get_mut(start..end).ok_or_else(|| {
                module_diagnostic(
                    module,
                    DiagnosticCategory::LayoutOverflow,
                    "static initializer exceeds object layout",
                    expression.span(),
                )
            })?;
            destination.copy_from_slice(&value.bits.to_le_bytes()[..size]);
            Ok(())
        }
    }
}

fn struct_initializer_args<'a>(
    context: &ProjectContext<'_>,
    module_id: usize,
    expected_name: &str,
    expression: &'a Expr,
) -> NativeResult<&'a [Expr]> {
    let module = &context.graph.modules[module_id];
    let (actual, args) = match expression {
        Expr::Call { name, args, span } => {
            let actual = local_struct_type(context, module_id, name, *span)?;
            (actual, args.as_slice())
        }
        Expr::MemberCall {
            object,
            property,
            args,
            span,
        } => {
            let Expr::Ident(alias, _) = object.as_ref() else {
                return Err(module_diagnostic(
                    module,
                    DiagnosticCategory::InvalidType,
                    "struct constructor namespace must be an import alias",
                    *span,
                ));
            };
            let actual = imported_struct_type(context, module_id, alias, property, *span)?;
            (actual, args.as_slice())
        }
        _ => {
            return Err(module_diagnostic(
                module,
                DiagnosticCategory::InvalidType,
                format!("initializer for {expected_name} requires its positional constructor"),
                expression.span(),
            ))
        }
    };
    if actual != SystemType::Struct(expected_name.to_owned()) {
        return Err(module_diagnostic(
            module,
            DiagnosticCategory::InvalidType,
            format!("initializer expects {expected_name}, found {actual}"),
            expression.span(),
        ));
    }
    Ok(args)
}

fn local_struct_type(
    context: &ProjectContext<'_>,
    module_id: usize,
    name: &str,
    span: Span,
) -> NativeResult<SystemType> {
    if context.headers[module_id].structs.contains_key(name) {
        Ok(SystemType::Struct(canonical_type_name(
            &context.graph.modules[module_id],
            name,
        )))
    } else {
        Err(module_diagnostic(
            &context.graph.modules[module_id],
            DiagnosticCategory::UnresolvedFunction,
            format!("unresolved function or struct constructor '{name}'"),
            span,
        ))
    }
}

fn imported_struct_type(
    context: &ProjectContext<'_>,
    module_id: usize,
    alias: &str,
    name: &str,
    span: Span,
) -> NativeResult<SystemType> {
    let module = &context.graph.modules[module_id];
    let target = import_target(context, module_id, alias, span)?;
    let header = context.headers[target].structs.get(name).ok_or_else(|| {
        module_diagnostic(
            module,
            DiagnosticCategory::UnknownType,
            format!(
                "module '{}' has no native struct '{name}'",
                context.graph.modules[target].name
            ),
            span,
        )
    })?;
    if !header.public {
        return Err(module_diagnostic(
            module,
            DiagnosticCategory::UnknownType,
            format!(
                "native struct '{}.{name}' is module-private",
                context.graph.modules[target].name
            ),
            span,
        ));
    }
    Ok(SystemType::Struct(canonical_type_name(
        &context.graph.modules[target],
        name,
    )))
}

fn import_target(
    context: &ProjectContext<'_>,
    module_id: usize,
    alias: &str,
    span: Span,
) -> NativeResult<usize> {
    context.headers[module_id]
        .imports
        .get(alias)
        .copied()
        .ok_or_else(|| {
            module_diagnostic(
                &context.graph.modules[module_id],
                DiagnosticCategory::InvalidModuleImport,
                format!("unknown native import alias '{alias}'"),
                span,
            )
        })
}

const fn visibility(public: bool) -> Visibility {
    if public {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

#[derive(Debug, Clone)]
struct Binding {
    slot: LocalId,
    ty: SystemType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExprResult {
    Value(SystemType),
    Void,
}

#[derive(Debug, Clone)]
struct LValue {
    ty: SystemType,
    mutable: bool,
    legacy_string_index: bool,
}

struct FunctionAnalyzer<'a> {
    context: &'a ProjectContext<'a>,
    module_id: usize,
    function_name: &'a str,
    signature: FunctionSignature,
    strings: &'a mut Vec<Vec<u8>>,
    string_ids: &'a mut BTreeMap<Vec<u8>, StringId>,
    external_functions: &'a mut BTreeMap<String, ExternalFunction>,
    external_globals: &'a mut BTreeMap<String, ExternalGlobal>,
    scopes: Vec<BTreeMap<String, Binding>>,
    params: Vec<SystemsParam>,
    slots: Vec<SystemsSlot>,
    instructions: Vec<SystemsInst>,
    next_label: u32,
}

impl<'a> FunctionAnalyzer<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        context: &'a ProjectContext<'a>,
        module_id: usize,
        function_name: &'a str,
        signature: FunctionSignature,
        strings: &'a mut Vec<Vec<u8>>,
        string_ids: &'a mut BTreeMap<Vec<u8>, StringId>,
        external_functions: &'a mut BTreeMap<String, ExternalFunction>,
        external_globals: &'a mut BTreeMap<String, ExternalGlobal>,
    ) -> Self {
        Self {
            context,
            module_id,
            function_name,
            signature,
            strings,
            string_ids,
            external_functions,
            external_globals,
            scopes: vec![BTreeMap::new()],
            params: Vec::new(),
            slots: Vec::new(),
            instructions: Vec::new(),
            next_label: 0,
        }
    }

    fn compile(mut self, params: &[Param], body: &[Stmt]) -> NativeResult<SystemsFunction> {
        for (parameter, ty) in params.iter().zip(self.signature.params.clone()) {
            let slot = self.allocate_slot(&parameter.name, ty.clone(), true, parameter.span)?;
            self.params.push(SystemsParam {
                name: parameter.name.clone(),
                ty,
                slot,
            });
        }
        self.compile_block(body)?;
        if !block_always_returns(body) {
            return Err(self.error(
                DiagnosticCategory::InvalidReturn,
                format!(
                    "function '{}' does not return on every control-flow path",
                    self.function_name
                ),
                self.signature.span,
            ));
        }
        Ok(SystemsFunction {
            source_name: self.function_name.to_owned(),
            symbol: self.signature.symbol.clone(),
            visibility: visibility(self.signature.public),
            interrupt: self.signature.interrupt,
            parameter_types: self.signature.params.clone(),
            params: self.params,
            return_type: self.signature.return_type.clone(),
            slots: self.slots,
            instructions: self.instructions,
            span: self.signature.span,
        })
    }

    fn compile_block(&mut self, statements: &[Stmt]) -> NativeResult<()> {
        for statement in statements {
            self.compile_stmt(statement)?;
        }
        Ok(())
    }

    fn compile_scoped_block(&mut self, statements: &[Stmt]) -> NativeResult<()> {
        self.scopes.push(BTreeMap::new());
        let result = self.compile_block(statements);
        self.scopes.pop();
        result
    }

    fn compile_stmt(&mut self, statement: &Stmt) -> NativeResult<()> {
        match statement {
            Stmt::Let {
                is_pub,
                name,
                ty,
                value,
                span,
            } => {
                if *is_pub {
                    return Err(self.error(
                        DiagnosticCategory::UnsupportedFeature,
                        "public local variables are unsupported in native functions",
                        *span,
                    ));
                }
                self.ensure_local_available(name, *span)?;
                if matches!(ty, TypeAnnotation::Inferred) {
                    let inferred = self.compile_value(value, value.span(), "local initializer")?;
                    if !inferred.is_scalar() {
                        return Err(self.error(
                            DiagnosticCategory::InvalidType,
                            format!("aggregate local '{name}' requires an explicit type"),
                            *span,
                        ));
                    }
                    let slot = self.allocate_slot(name, inferred.clone(), false, *span)?;
                    self.instructions.push(SystemsInst::LocalStore {
                        local: slot,
                        ty: inferred,
                    });
                } else {
                    let declared = resolve_annotation(
                        self.context.graph,
                        &self.context.headers,
                        self.module_id,
                        ty,
                        *span,
                    )?;
                    let slot = self.allocate_slot(name, declared.clone(), false, *span)?;
                    if declared.is_scalar() {
                        let actual = self.compile_value_expected(
                            value,
                            Some(&declared),
                            value.span(),
                            "local initializer",
                        )?;
                        self.require_same_type(
                            &declared,
                            &actual,
                            value.span(),
                            format!("local '{name}'"),
                        )?;
                        self.instructions.push(SystemsInst::LocalStore {
                            local: slot,
                            ty: declared,
                        });
                    } else {
                        self.compile_aggregate_initializer(slot, &declared, value, 0)?;
                    }
                }
            }
            Stmt::Assign { name, value, span } => self.compile_assignment(name, value, *span)?,
            Stmt::Store {
                target,
                value,
                span,
            } => {
                let destination = self.compile_lvalue(target)?;
                if !destination.mutable {
                    return Err(self.error(
                        DiagnosticCategory::InvalidPointerOperation,
                        "cannot store through an immutable native lvalue",
                        *span,
                    ));
                }
                if !destination.ty.is_scalar() {
                    return Err(self.error(
                        DiagnosticCategory::UnsupportedFeature,
                        "aggregate assignment is unsupported; initialize fields explicitly",
                        *span,
                    ));
                }
                let actual = self.compile_value_expected(
                    value,
                    Some(&destination.ty),
                    value.span(),
                    "store value",
                )?;
                self.require_same_type(&destination.ty, &actual, *span, "lvalue store")?;
                self.instructions.push(SystemsInst::Store(destination.ty));
            }
            Stmt::Return { value, span } => {
                if self.signature.interrupt.is_some() {
                    if value.is_some() {
                        return Err(self.error(
                            DiagnosticCategory::InvalidReturn,
                            format!(
                                "interrupt function '{}' cannot return a value",
                                self.function_name
                            ),
                            *span,
                        ));
                    }
                    self.instructions.push(SystemsInst::Const {
                        bits: 0,
                        ty: SystemType::I32,
                    });
                    self.instructions.push(SystemsInst::Return);
                    return Ok(());
                }
                let Some(value) = value else {
                    return Err(self.error(
                        DiagnosticCategory::InvalidReturn,
                        format!(
                            "function '{}' must return a {} value",
                            self.function_name, self.signature.return_type
                        ),
                        *span,
                    ));
                };
                let expected = self.signature.return_type.clone();
                let actual = self.compile_value_expected(
                    value,
                    Some(&expected),
                    value.span(),
                    "return value",
                )?;
                if actual != expected {
                    return Err(self.error(
                        DiagnosticCategory::InvalidReturn,
                        format!(
                            "function '{}' must return {}, found {actual}",
                            self.function_name, expected
                        ),
                        *span,
                    ));
                }
                self.instructions.push(SystemsInst::Return);
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
                span,
            } => {
                self.compile_condition(cond, "if")?;
                if let Some(else_body) = else_body {
                    let else_label = self.new_label(*span)?;
                    let end_label = self.new_label(*span)?;
                    self.instructions.push(SystemsInst::JumpIfFalse(else_label));
                    self.compile_scoped_block(then_body)?;
                    self.instructions.push(SystemsInst::Jump(end_label));
                    self.instructions.push(SystemsInst::Label(else_label));
                    self.compile_scoped_block(else_body)?;
                    self.instructions.push(SystemsInst::Label(end_label));
                } else {
                    let end_label = self.new_label(*span)?;
                    self.instructions.push(SystemsInst::JumpIfFalse(end_label));
                    self.compile_scoped_block(then_body)?;
                    self.instructions.push(SystemsInst::Label(end_label));
                }
            }
            Stmt::While { cond, body, span } => {
                let loop_label = self.new_label(*span)?;
                let end_label = self.new_label(*span)?;
                self.instructions.push(SystemsInst::Label(loop_label));
                self.compile_condition(cond, "while")?;
                self.instructions.push(SystemsInst::JumpIfFalse(end_label));
                self.compile_scoped_block(body)?;
                self.instructions.push(SystemsInst::Jump(loop_label));
                self.instructions.push(SystemsInst::Label(end_label));
            }
            Stmt::Expr(expression) => match self.compile_expr(expression)? {
                ExprResult::Value(_) => self.instructions.push(SystemsInst::Pop),
                ExprResult::Void => {}
            },
            Stmt::For { span, .. } => {
                return Err(self.error(
                    DiagnosticCategory::UnsupportedFeature,
                    "for loops are unsupported in native systems functions",
                    *span,
                ))
            }
            Stmt::FnDef { span, .. }
            | Stmt::Import { span, .. }
            | Stmt::StructDef { span, .. }
            | Stmt::Const { span, .. }
            | Stmt::Static { span, .. } => {
                return Err(self.error(
                    DiagnosticCategory::UnsupportedFeature,
                    "module declarations cannot be nested inside native functions",
                    *span,
                ))
            }
        }
        Ok(())
    }

    fn compile_assignment(&mut self, name: &str, value: &Expr, span: Span) -> NativeResult<()> {
        if let Some(binding) = self.lookup(name) {
            if !binding.ty.is_scalar() {
                return Err(self.error(
                    DiagnosticCategory::UnsupportedFeature,
                    "aggregate assignment is unsupported; assign fields or elements",
                    span,
                ));
            }
            let actual = self.compile_value_expected(
                value,
                Some(&binding.ty),
                value.span(),
                "assignment value",
            )?;
            self.require_same_type(&binding.ty, &actual, span, format!("local '{name}'"))?;
            self.instructions.push(SystemsInst::LocalStore {
                local: binding.slot,
                ty: binding.ty,
            });
            return Ok(());
        }
        if let Some(global) = self.context.symbols[self.module_id]
            .globals
            .get(name)
            .cloned()
        {
            if !global.mutable {
                return Err(self.error(
                    DiagnosticCategory::InvalidPointerOperation,
                    format!("cannot assign immutable static '{name}'"),
                    span,
                ));
            }
            if !global.ty.is_scalar() {
                return Err(self.error(
                    DiagnosticCategory::UnsupportedFeature,
                    "aggregate static assignment is unsupported; assign elements or fields",
                    span,
                ));
            }
            self.instructions.push(SystemsInst::GlobalAddress {
                symbol: global.symbol.clone(),
                pointee: global.ty.clone(),
            });
            let actual = self.compile_value_expected(
                value,
                Some(&global.ty),
                value.span(),
                "global assignment value",
            )?;
            self.require_same_type(&global.ty, &actual, span, format!("static '{name}'"))?;
            self.instructions.push(SystemsInst::Store(global.ty));
            return Ok(());
        }
        Err(self.error(
            DiagnosticCategory::UnknownSymbol,
            format!("unknown mutable native symbol '{name}'"),
            span,
        ))
    }

    fn compile_condition(&mut self, expression: &Expr, owner: &str) -> NativeResult<()> {
        let actual = self.compile_value_expected(
            expression,
            Some(&SystemType::Bool),
            expression.span(),
            "condition",
        )?;
        if actual != SystemType::Bool {
            return Err(self.error(
                DiagnosticCategory::InvalidType,
                format!("{owner} condition must be bool, found {actual}"),
                expression.span(),
            ));
        }
        Ok(())
    }

    fn compile_aggregate_initializer(
        &mut self,
        slot: LocalId,
        expected: &SystemType,
        expression: &Expr,
        base_offset: u32,
    ) -> NativeResult<()> {
        match expected {
            SystemType::Array(element, length) => {
                let Expr::List(items, span) = expression else {
                    return Err(self.error(
                        DiagnosticCategory::InvalidType,
                        format!("initializer for {expected} requires an array literal"),
                        expression.span(),
                    ));
                };
                if items.len() != *length as usize {
                    return Err(self.error(
                        DiagnosticCategory::BadArrayLength,
                        format!(
                            "initializer for {expected} expects {length} elements, got {}",
                            items.len()
                        ),
                        *span,
                    ));
                }
                let element_layout = self
                    .context
                    .layouts
                    .layout_of(element, *span)
                    .map_err(|error| self.attach(error))?;
                for (index, item) in items.iter().enumerate() {
                    let offset = base_offset
                        .checked_add(element_layout.size.saturating_mul(index as u32))
                        .ok_or_else(|| {
                            self.error(
                                DiagnosticCategory::LayoutOverflow,
                                "local array initializer offset overflow",
                                item.span(),
                            )
                        })?;
                    self.compile_aggregate_initializer(slot, element, item, offset)?;
                }
                Ok(())
            }
            SystemType::Struct(name) => {
                let args = struct_initializer_args(self.context, self.module_id, name, expression)?
                    .to_vec();
                let layout = self
                    .context
                    .layouts
                    .structure(name)
                    .expect("resolved struct has a layout");
                if args.len() != layout.fields.len() {
                    return Err(self.error(
                        DiagnosticCategory::WrongArity,
                        format!(
                            "struct constructor for {name} expects {} fields, got {}",
                            layout.fields.len(),
                            args.len()
                        ),
                        expression.span(),
                    ));
                }
                let fields = layout.fields.clone();
                for (field, argument) in fields.iter().zip(&args) {
                    let offset = base_offset.checked_add(field.offset).ok_or_else(|| {
                        self.error(
                            DiagnosticCategory::LayoutOverflow,
                            "local struct initializer offset overflow",
                            argument.span(),
                        )
                    })?;
                    self.compile_aggregate_initializer(slot, &field.ty, argument, offset)?;
                }
                Ok(())
            }
            _ if expected.is_scalar() => {
                self.instructions.push(SystemsInst::LocalAddress(slot));
                if base_offset != 0 {
                    self.instructions.push(SystemsInst::FieldAddress {
                        offset: base_offset,
                    });
                }
                let actual = self.compile_value_expected(
                    expression,
                    Some(expected),
                    expression.span(),
                    "aggregate field initializer",
                )?;
                self.require_same_type(expected, &actual, expression.span(), "aggregate field")?;
                self.instructions.push(SystemsInst::Store(expected.clone()));
                Ok(())
            }
            _ => Err(self.error(
                DiagnosticCategory::InvalidType,
                format!("cannot initialize native type {expected}"),
                expression.span(),
            )),
        }
    }

    fn compile_expr(&mut self, expression: &Expr) -> NativeResult<ExprResult> {
        self.compile_expr_expected(expression, None)
    }

    fn compile_expr_expected(
        &mut self,
        expression: &Expr,
        expected: Option<&SystemType>,
    ) -> NativeResult<ExprResult> {
        match expression {
            Expr::IntLit(value, span) => {
                let ty = expected
                    .filter(|ty| ty.is_integer())
                    .cloned()
                    .unwrap_or(SystemType::I32);
                let bits =
                    checked_literal_bits(*value, &ty, *span).map_err(|error| self.attach(error))?;
                self.instructions.push(SystemsInst::Const {
                    bits,
                    ty: ty.clone(),
                });
                Ok(ExprResult::Value(ty))
            }
            Expr::BoolLit(value, span) => {
                if expected.is_some_and(|ty| *ty != SystemType::Bool) {
                    return Err(self.error(
                        DiagnosticCategory::InvalidType,
                        format!(
                            "boolean expression cannot initialize native type {}",
                            expected.expect("checked Some")
                        ),
                        *span,
                    ));
                }
                self.instructions.push(SystemsInst::ConstBool(*value));
                Ok(ExprResult::Value(SystemType::Bool))
            }
            Expr::StrLit(value, span) => {
                if expected.is_some_and(|ty| *ty != SystemType::Str) {
                    return Err(self.error(
                        DiagnosticCategory::InvalidType,
                        format!(
                            "string expression cannot initialize native type {}",
                            expected.expect("checked Some")
                        ),
                        *span,
                    ));
                }
                if !value.is_ascii() {
                    return Err(self.error(
                        DiagnosticCategory::UnsupportedFeature,
                        "non-ASCII strings are unsupported in the freestanding native profile",
                        *span,
                    ));
                }
                let id = self.intern_string(value.as_bytes(), *span)?;
                self.instructions.push(SystemsInst::StringAddress(id));
                Ok(ExprResult::Value(SystemType::Str))
            }
            Expr::Ident(name, span) => self.compile_identifier(name, *span),
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => self.compile_binary(left, op, right, expected, *span),
            Expr::Unary { op, expr, span } => self.compile_unary(op, expr, expected, *span),
            Expr::Cast { expr, ty, span } => self.compile_cast(expr, ty, *span),
            Expr::Call { name, args, span } => self.compile_call(name, args, *span),
            Expr::MemberCall {
                object,
                property,
                args,
                span,
            } => self.compile_member_call(object, property, args, *span),
            Expr::Member {
                object,
                property,
                span,
            } if matches!(object.as_ref(), Expr::Ident(alias, _) if self.context.headers[self.module_id].imports.contains_key(alias)) =>
            {
                let Expr::Ident(alias, _) = object.as_ref() else {
                    unreachable!("guard requires import alias")
                };
                self.compile_imported_value(alias, property, *span)
            }
            Expr::Index { .. } | Expr::Member { .. } => {
                let lvalue = self.compile_lvalue(expression)?;
                if !lvalue.ty.is_scalar() {
                    return Err(self.error(
                        DiagnosticCategory::InvalidType,
                        format!("aggregate value of type {} cannot be copied", lvalue.ty),
                        expression.span(),
                    ));
                }
                let loaded = lvalue.ty.clone();
                self.instructions.push(SystemsInst::Load(loaded.clone()));
                if lvalue.legacy_string_index {
                    self.instructions.push(SystemsInst::Cast {
                        from: SystemType::U8,
                        to: SystemType::I32,
                    });
                    Ok(ExprResult::Value(SystemType::I32))
                } else {
                    Ok(ExprResult::Value(loaded))
                }
            }
            Expr::List(_, span) => Err(self.error(
                DiagnosticCategory::InvalidType,
                "array literals are valid only as destination-directed aggregate initializers",
                *span,
            )),
            Expr::FloatLit(_, span) => Err(self.unsupported("float literals", *span)),
            Expr::BytesLit(_, span) => Err(self.unsupported("byte-string literals", *span)),
            Expr::FStr(_, span) => Err(self.unsupported("formatted strings", *span)),
        }
    }

    fn compile_identifier(&mut self, name: &str, span: Span) -> NativeResult<ExprResult> {
        if let Some(binding) = self.lookup(name) {
            if !binding.ty.is_scalar() {
                return Err(self.error(
                    DiagnosticCategory::InvalidType,
                    format!("aggregate local '{name}' cannot be copied by value"),
                    span,
                ));
            }
            self.instructions.push(SystemsInst::LocalLoad {
                local: binding.slot,
                ty: binding.ty.clone(),
            });
            return Ok(ExprResult::Value(binding.ty));
        }
        if let Some(value) = self
            .context
            .constants
            .get(&(self.module_id, name.to_owned()))
            .cloned()
        {
            self.emit_constant(&value);
            return Ok(ExprResult::Value(value.ty));
        }
        if let Some(global) = self.context.symbols[self.module_id]
            .globals
            .get(name)
            .cloned()
        {
            if !global.ty.is_scalar() {
                return Err(self.error(
                    DiagnosticCategory::InvalidType,
                    format!("aggregate static '{name}' cannot be copied by value"),
                    span,
                ));
            }
            self.instructions.push(SystemsInst::GlobalAddress {
                symbol: global.symbol,
                pointee: global.ty.clone(),
            });
            self.instructions.push(SystemsInst::Load(global.ty.clone()));
            return Ok(ExprResult::Value(global.ty));
        }
        if self.context.headers[self.module_id]
            .imports
            .contains_key(name)
        {
            return Err(self.error(
                DiagnosticCategory::UnknownSymbol,
                format!("module alias '{name}' is not a first-class value"),
                span,
            ));
        }
        Err(self.error(
            DiagnosticCategory::UnknownSymbol,
            format!("unknown native symbol '{name}'"),
            span,
        ))
    }

    fn compile_binary(
        &mut self,
        left: &Expr,
        operator: &BinOp,
        right: &Expr,
        expected: Option<&SystemType>,
        span: Span,
    ) -> NativeResult<ExprResult> {
        if matches!(operator, BinOp::Div) {
            return Err(self.unsupported("integer division", span));
        }
        let operand_expected = if matches!(
            operator,
            BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor
                | BinOp::Shl
                | BinOp::Shr
        ) {
            expected.filter(|ty| ty.is_integer())
        } else {
            None
        };
        let inferred_left = operand_expected.cloned().or_else(|| {
            signed_literal(left)
                .is_some()
                .then(|| self.known_value_type(right))
                .flatten()
                .filter(SystemType::is_integer)
        });
        let left_ty =
            self.compile_value_expected(left, inferred_left.as_ref(), left.span(), "left operand")?;
        let right_ty =
            self.compile_value_expected(right, Some(&left_ty), right.span(), "right operand")?;
        if left_ty != right_ty {
            return Err(self.error(
                DiagnosticCategory::InvalidType,
                format!(
                    "operator '{operator}' requires matching types, found {left_ty} and {right_ty}"
                ),
                span,
            ));
        }
        match operator {
            BinOp::Add
            | BinOp::Sub
            | BinOp::Mul
            | BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::Shl
            | BinOp::Shr => {
                if !left_ty.is_integer() {
                    return Err(self.error(
                        DiagnosticCategory::InvalidType,
                        format!("operator '{operator}' requires integer operands, found {left_ty}"),
                        span,
                    ));
                }
                self.instructions.push(match operator {
                    BinOp::Add => SystemsInst::Add(left_ty.clone()),
                    BinOp::Sub => SystemsInst::Sub(left_ty.clone()),
                    BinOp::Mul => SystemsInst::Mul(left_ty.clone()),
                    BinOp::BitAnd => SystemsInst::BitAnd(left_ty.clone()),
                    BinOp::BitOr => SystemsInst::BitOr(left_ty.clone()),
                    BinOp::BitXor => SystemsInst::BitXor(left_ty.clone()),
                    BinOp::Shl => SystemsInst::ShiftLeft(left_ty.clone()),
                    BinOp::Shr => SystemsInst::ShiftRight(left_ty.clone()),
                    _ => unreachable!(),
                });
                Ok(ExprResult::Value(left_ty))
            }
            BinOp::EqEq | BinOp::NotEq => {
                if !left_ty.is_scalar() {
                    return Err(self.error(
                        DiagnosticCategory::InvalidType,
                        format!("operator '{operator}' requires scalar operands"),
                        span,
                    ));
                }
                self.instructions.push(SystemsInst::Compare {
                    op: if matches!(operator, BinOp::EqEq) {
                        CompareOp::Eq
                    } else {
                        CompareOp::NotEq
                    },
                    operand_type: left_ty,
                });
                Ok(ExprResult::Value(SystemType::Bool))
            }
            BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq => {
                if !left_ty.is_integer() {
                    return Err(self.error(
                        DiagnosticCategory::InvalidType,
                        format!("operator '{operator}' requires integer operands, found {left_ty}"),
                        span,
                    ));
                }
                let op = match operator {
                    BinOp::Lt => CompareOp::Lt,
                    BinOp::LtEq => CompareOp::LtEq,
                    BinOp::Gt => CompareOp::Gt,
                    BinOp::GtEq => CompareOp::GtEq,
                    _ => unreachable!(),
                };
                self.instructions.push(SystemsInst::Compare {
                    op,
                    operand_type: left_ty,
                });
                Ok(ExprResult::Value(SystemType::Bool))
            }
            BinOp::Div => unreachable!(),
        }
    }

    fn compile_unary(
        &mut self,
        operator: &UnaryOp,
        expression: &Expr,
        expected: Option<&SystemType>,
        span: Span,
    ) -> NativeResult<ExprResult> {
        match operator {
            UnaryOp::AddressOf => {
                let lvalue = self.compile_lvalue(expression)?;
                Ok(ExprResult::Value(SystemType::Pointer(Box::new(lvalue.ty))))
            }
            UnaryOp::Deref => {
                let pointer =
                    self.compile_value(expression, expression.span(), "dereference operand")?;
                let SystemType::Pointer(pointee) = pointer else {
                    return Err(self.error(
                        DiagnosticCategory::BadDereference,
                        format!("cannot dereference native type {pointer}"),
                        span,
                    ));
                };
                if !pointee.is_scalar() {
                    return Err(self.error(
                        DiagnosticCategory::InvalidType,
                        format!("aggregate dereference of {pointee} cannot be copied by value"),
                        span,
                    ));
                }
                self.instructions
                    .push(SystemsInst::Load((*pointee).clone()));
                Ok(ExprResult::Value(*pointee))
            }
            UnaryOp::Neg => {
                if let Some(literal) = signed_literal(expression) {
                    if expected.is_some_and(SystemType::is_signed_integer) {
                        let ty = expected.expect("checked Some").clone();
                        let bits = checked_literal_bits(-literal, &ty, span)
                            .map_err(|error| self.attach(error))?;
                        self.instructions.push(SystemsInst::Const {
                            bits,
                            ty: ty.clone(),
                        });
                        return Ok(ExprResult::Value(ty));
                    }
                }
                let operand = self.compile_value_expected(
                    expression,
                    expected,
                    expression.span(),
                    "unary negation operand",
                )?;
                if !operand.is_signed_integer() {
                    return Err(self.error(
                        DiagnosticCategory::InvalidType,
                        format!("unary '-' requires a signed integer, found {operand}"),
                        span,
                    ));
                }
                self.instructions.push(SystemsInst::Neg(operand.clone()));
                Ok(ExprResult::Value(operand))
            }
            UnaryOp::BitNot => {
                let operand = self.compile_value_expected(
                    expression,
                    expected,
                    expression.span(),
                    "bitwise-not operand",
                )?;
                if !operand.is_integer() {
                    return Err(self.error(
                        DiagnosticCategory::InvalidType,
                        format!("unary '~' requires an integer, found {operand}"),
                        span,
                    ));
                }
                self.instructions.push(SystemsInst::BitNot(operand.clone()));
                Ok(ExprResult::Value(operand))
            }
            UnaryOp::Not => {
                let operand = self.compile_value_expected(
                    expression,
                    Some(&SystemType::Bool),
                    expression.span(),
                    "boolean-not operand",
                )?;
                if operand != SystemType::Bool {
                    return Err(self.error(
                        DiagnosticCategory::InvalidType,
                        format!("unary '!' requires bool, found {operand}"),
                        span,
                    ));
                }
                self.instructions.push(SystemsInst::BoolNot);
                Ok(ExprResult::Value(SystemType::Bool))
            }
        }
    }

    fn compile_cast(
        &mut self,
        expression: &Expr,
        annotation: &TypeAnnotation,
        span: Span,
    ) -> NativeResult<ExprResult> {
        let target = resolve_annotation(
            self.context.graph,
            &self.context.headers,
            self.module_id,
            annotation,
            span,
        )?;
        if !target.is_scalar() || matches!(target, SystemType::Bool | SystemType::Str) {
            return Err(self.error(
                DiagnosticCategory::InvalidNarrowing,
                format!("runtime cast target must be an integer or pointer, found {target}"),
                span,
            ));
        }
        if let Some(literal) = signed_literal(expression) {
            if target.is_integer() {
                let bits = checked_literal_bits(literal, &target, span)
                    .map_err(|error| self.attach(error))?;
                self.instructions.push(SystemsInst::Const {
                    bits,
                    ty: target.clone(),
                });
                return Ok(ExprResult::Value(target));
            }
        }
        let source = self.compile_value(expression, expression.span(), "cast operand")?;
        if !source.is_scalar()
            || matches!(source, SystemType::Bool | SystemType::Str)
            || !explicit_cast_allowed(&source, &target)
        {
            return Err(self.error(
                DiagnosticCategory::InvalidNarrowing,
                format!("invalid native cast from {source} to {target}"),
                span,
            ));
        }
        self.instructions.push(SystemsInst::Cast {
            from: source,
            to: target.clone(),
        });
        Ok(ExprResult::Value(target))
    }

    fn compile_call(&mut self, name: &str, args: &[Expr], span: Span) -> NativeResult<ExprResult> {
        if name == "interrupt_address" {
            return self.compile_interrupt_address(args, span);
        }
        if name == "task_address" {
            return self.compile_task_address(args, span);
        }
        if matches!(name, "size_of" | "align_of") {
            if args.len() != 1 {
                return Err(self.error(
                    DiagnosticCategory::WrongArity,
                    format!("{name} expects exactly one type argument"),
                    span,
                ));
            }
            let annotation = type_annotation_from_expr(&args[0]).ok_or_else(|| {
                self.error(
                    DiagnosticCategory::InvalidType,
                    format!("{name} expects a native type name"),
                    args[0].span(),
                )
            })?;
            let ty = resolve_annotation(
                self.context.graph,
                &self.context.headers,
                self.module_id,
                &annotation,
                args[0].span(),
            )?;
            let layout = self
                .context
                .layouts
                .layout_of(&ty, args[0].span())
                .map_err(|error| self.attach(error))?;
            self.instructions.push(SystemsInst::Const {
                bits: if name == "size_of" {
                    layout.size
                } else {
                    layout.alignment
                },
                ty: SystemType::Usize,
            });
            return Ok(ExprResult::Value(SystemType::Usize));
        }
        if self.context.headers[self.module_id]
            .structs
            .contains_key(name)
        {
            return Err(self.error(
                DiagnosticCategory::InvalidType,
                format!("struct constructor '{name}' is valid only as an aggregate initializer"),
                span,
            ));
        }
        let signature = self.context.symbols[self.module_id]
            .functions
            .get(name)
            .cloned()
            .ok_or_else(|| {
                self.error(
                    DiagnosticCategory::UnresolvedFunction,
                    format!("unresolved native function '{name}'"),
                    span,
                )
            })?;
        self.emit_call(name, args, &signature, span, false)
    }

    fn compile_member_call(
        &mut self,
        object: &Expr,
        property: &str,
        args: &[Expr],
        span: Span,
    ) -> NativeResult<ExprResult> {
        let Expr::Ident(namespace, _) = object else {
            return Err(self.error(
                DiagnosticCategory::UnsupportedFeature,
                "native method calls require an import alias or the native namespace",
                span,
            ));
        };
        if namespace == "native" {
            return self.compile_intrinsic(property, args, span);
        }
        let target = import_target(self.context, self.module_id, namespace, span)?;
        if self.context.headers[target].structs.contains_key(property) {
            return Err(self.error(
                DiagnosticCategory::InvalidType,
                format!(
                    "struct constructor '{namespace}.{property}' is valid only as an aggregate initializer"
                ),
                span,
            ));
        }
        let signature = self.context.symbols[target]
            .functions
            .get(property)
            .cloned()
            .ok_or_else(|| {
                self.error(
                    DiagnosticCategory::UnresolvedFunction,
                    format!(
                        "module '{}' has no native function '{property}'",
                        self.context.graph.modules[target].name
                    ),
                    span,
                )
            })?;
        if !signature.public {
            return Err(self.error(
                DiagnosticCategory::UnresolvedFunction,
                format!(
                    "native function '{}.{property}' is module-private",
                    self.context.graph.modules[target].name
                ),
                span,
            ));
        }
        self.emit_call(property, args, &signature, span, true)
    }

    fn emit_call(
        &mut self,
        source_name: &str,
        args: &[Expr],
        signature: &FunctionSignature,
        span: Span,
        external: bool,
    ) -> NativeResult<ExprResult> {
        if signature.interrupt.is_some() {
            return Err(self.error(
                DiagnosticCategory::InvalidType,
                format!("interrupt function '{source_name}' is not directly callable"),
                span,
            ));
        }
        if args.len() != signature.params.len() {
            return Err(self.error(
                DiagnosticCategory::WrongArity,
                format!(
                    "function '{source_name}' expects {} arguments, got {}",
                    signature.params.len(),
                    args.len()
                ),
                span,
            ));
        }
        for (index, (argument, expected)) in args.iter().zip(&signature.params).enumerate() {
            let actual = self.compile_value_expected(
                argument,
                Some(expected),
                argument.span(),
                "function argument",
            )?;
            if actual != *expected {
                return Err(self.error(
                    DiagnosticCategory::InvalidType,
                    format!(
                        "argument {} to '{source_name}' expects {expected}, found {actual}",
                        index + 1
                    ),
                    argument.span(),
                ));
            }
        }
        if external {
            self.external_functions
                .entry(signature.symbol.clone())
                .or_insert_with(|| ExternalFunction {
                    symbol: signature.symbol.clone(),
                    parameter_types: signature.params.clone(),
                    return_type: signature.return_type.clone(),
                });
        }
        self.instructions.push(SystemsInst::Call {
            symbol: signature.symbol.clone(),
            argument_count: args.len(),
            result: signature.return_type.clone(),
        });
        Ok(ExprResult::Value(signature.return_type.clone()))
    }

    fn compile_intrinsic(
        &mut self,
        property: &str,
        args: &[Expr],
        span: Span,
    ) -> NativeResult<ExprResult> {
        let linker_symbol = match property {
            "kernel_image_start" => Some("__dbyte_image_start"),
            "kernel_image_end" => Some("__dbyte_image_end"),
            "bootstrap_stack_start" => Some("__dbyte_stack_bottom"),
            "bootstrap_stack_end" => Some("__dbyte_stack_top"),
            "task_trampoline_address" => Some("__dbyte_task_trampoline"),
            _ => None,
        };
        if let Some(symbol) = linker_symbol {
            if !args.is_empty() {
                return Err(self.error(
                    DiagnosticCategory::WrongArity,
                    format!("native.{property} expects 0 arguments, got {}", args.len()),
                    span,
                ));
            }
            self.instructions.push(SystemsInst::LinkerAddress {
                symbol: symbol.to_owned(),
            });
            return Ok(ExprResult::Value(SystemType::Usize));
        }
        if property == "cpu_reload_segments" {
            if args.len() != 2 {
                return Err(self.error(
                    DiagnosticCategory::WrongArity,
                    format!(
                        "native.cpu_reload_segments expects 2 arguments, got {}",
                        args.len()
                    ),
                    span,
                ));
            }
            let code_selector = self.constant_u16(&args[0], "code selector")?;
            let data_selector = self.constant_u16(&args[1], "data selector")?;
            self.instructions.push(SystemsInst::ReloadSegments {
                code_selector,
                data_selector,
            });
            return Ok(ExprResult::Void);
        }
        let intrinsic = SystemIntrinsic::from_name(property).ok_or_else(|| {
            self.error(
                DiagnosticCategory::InvalidIntrinsic,
                format!("unknown native intrinsic 'native.{property}'"),
                span,
            )
        })?;
        if intrinsic == SystemIntrinsic::InterruptResume
            && self.signature.interrupt == Some(InterruptClass::ErrorCode)
        {
            return Err(self.error(
                DiagnosticCategory::InvalidIntrinsic,
                "native.interrupt_resume is forbidden in error-code interrupt functions",
                span,
            ));
        }
        let expected_types = intrinsic.parameter_types();
        if args.len() != expected_types.len() {
            return Err(self.error(
                DiagnosticCategory::WrongArity,
                format!(
                    "intrinsic '{intrinsic}' expects {} arguments, got {}",
                    expected_types.len(),
                    args.len()
                ),
                span,
            ));
        }
        for (index, (argument, expected)) in args.iter().zip(&expected_types).enumerate() {
            let actual = self.compile_value_expected(
                argument,
                Some(expected),
                argument.span(),
                "intrinsic argument",
            )?;
            if actual != *expected {
                return Err(self.error(
                    DiagnosticCategory::InvalidIntrinsic,
                    format!(
                        "argument {} to '{intrinsic}' expects {expected}, found {actual}",
                        index + 1
                    ),
                    argument.span(),
                ));
            }
        }
        let result = intrinsic.result_type();
        self.instructions.push(SystemsInst::Intrinsic {
            intrinsic,
            argument_count: args.len(),
            result: result.clone(),
        });
        Ok(result.map_or(ExprResult::Void, ExprResult::Value))
    }

    fn constant_u16(&self, expression: &Expr, role: &str) -> NativeResult<u16> {
        let mut evaluator = ConstantEvaluator {
            graph: self.context.graph,
            headers: &self.context.headers,
            symbols: &self.context.symbols,
            layouts: &self.context.layouts,
            values: self.context.constants.clone(),
            visiting: BTreeSet::new(),
        };
        let value = evaluator
            .evaluate_expr(self.module_id, expression, Some(&SystemType::U16))
            .map_err(|_| {
                self.error(
                    DiagnosticCategory::InvalidIntrinsic,
                    format!(
                        "native.cpu_reload_segments {role} must be a compile-time u16 constant"
                    ),
                    expression.span(),
                )
            })?;
        u16::try_from(value.bits).map_err(|_| {
            self.error(
                DiagnosticCategory::IntegerOutOfRange,
                format!("native.cpu_reload_segments {role} exceeds u16"),
                expression.span(),
            )
        })
    }

    fn compile_interrupt_address(&mut self, args: &[Expr], span: Span) -> NativeResult<ExprResult> {
        if args.len() != 2 {
            return Err(self.error(
                DiagnosticCategory::WrongArity,
                format!("interrupt_address expects 2 arguments, got {}", args.len()),
                span,
            ));
        }
        let mut evaluator = ConstantEvaluator {
            graph: self.context.graph,
            headers: &self.context.headers,
            symbols: &self.context.symbols,
            layouts: &self.context.layouts,
            values: self.context.constants.clone(),
            visiting: BTreeSet::new(),
        };
        let vector_value = evaluator
            .evaluate_expr(self.module_id, &args[0], Some(&SystemType::Usize))
            .map_err(|_| {
                self.error(
                    DiagnosticCategory::InvalidIntrinsic,
                    "interrupt_address vector must be a compile-time constant",
                    args[0].span(),
                )
            })?;
        let vector = u8::try_from(vector_value.bits).map_err(|_| {
            self.error(
                DiagnosticCategory::IntegerOutOfRange,
                "interrupt_address vector must be in 0..255",
                args[0].span(),
            )
        })?;
        let (signature, source_name, external) = match &args[1] {
            Expr::Ident(name, _) => (
                self.context.symbols[self.module_id]
                    .functions
                    .get(name)
                    .cloned()
                    .ok_or_else(|| {
                        self.error(
                            DiagnosticCategory::UnresolvedFunction,
                            format!("unresolved interrupt handler '{name}'"),
                            args[1].span(),
                        )
                    })?,
                name.clone(),
                false,
            ),
            Expr::Member {
                object, property, ..
            } => {
                let Expr::Ident(alias, _) = object.as_ref() else {
                    return Err(self.error(
                        DiagnosticCategory::InvalidIntrinsic,
                        "interrupt_address handler must be a function symbol",
                        args[1].span(),
                    ));
                };
                let target = import_target(self.context, self.module_id, alias, args[1].span())?;
                let signature = self.context.symbols[target]
                    .functions
                    .get(property)
                    .cloned()
                    .ok_or_else(|| {
                        self.error(
                            DiagnosticCategory::UnresolvedFunction,
                            format!("module '{alias}' has no handler '{property}'"),
                            args[1].span(),
                        )
                    })?;
                if !signature.public {
                    return Err(self.error(
                        DiagnosticCategory::UnresolvedFunction,
                        format!("interrupt handler '{alias}.{property}' is module-private"),
                        args[1].span(),
                    ));
                }
                (signature, format!("{alias}.{property}"), true)
            }
            _ => {
                return Err(self.error(
                    DiagnosticCategory::InvalidIntrinsic,
                    "interrupt_address handler must be an interrupt function symbol",
                    args[1].span(),
                ))
            }
        };
        let Some(class) = signature.interrupt else {
            return Err(self.error(
                DiagnosticCategory::InvalidIntrinsic,
                format!("'{source_name}' is not an interrupt function"),
                args[1].span(),
            ));
        };
        let requires_error = matches!(vector, 8 | 10 | 11 | 12 | 13 | 14 | 17);
        let required = if requires_error {
            InterruptClass::ErrorCode
        } else {
            InterruptClass::NoErrorCode
        };
        if class != required {
            return Err(self.error(
                DiagnosticCategory::InterruptClassMismatch,
                format!(
                    "vector {vector} requires {required} handler, but {source_name} is {class}"
                ),
                span,
            ));
        }
        if external {
            self.external_functions
                .entry(signature.symbol.clone())
                .or_insert_with(|| ExternalFunction {
                    symbol: signature.symbol.clone(),
                    parameter_types: signature.params.clone(),
                    return_type: signature.return_type.clone(),
                });
        }
        self.instructions.push(SystemsInst::InterruptAddress {
            vector,
            symbol: signature.symbol,
            class,
        });
        Ok(ExprResult::Value(SystemType::Usize))
    }

    fn compile_task_address(&mut self, args: &[Expr], span: Span) -> NativeResult<ExprResult> {
        if args.len() != 1 {
            return Err(self.error(
                DiagnosticCategory::WrongArity,
                format!("task_address expects 1 argument, got {}", args.len()),
                span,
            ));
        }
        let (signature, source_name, external) = match &args[0] {
            Expr::Ident(name, _) => (
                self.context.symbols[self.module_id]
                    .functions
                    .get(name)
                    .cloned()
                    .ok_or_else(|| {
                        self.error(
                            DiagnosticCategory::UnresolvedFunction,
                            format!("unresolved task entry '{name}'"),
                            args[0].span(),
                        )
                    })?,
                name.clone(),
                false,
            ),
            Expr::Member {
                object, property, ..
            } => {
                let Expr::Ident(alias, _) = object.as_ref() else {
                    return Err(self.error(
                        DiagnosticCategory::InvalidIntrinsic,
                        "task_address entry must be a public function symbol",
                        args[0].span(),
                    ));
                };
                let target = import_target(self.context, self.module_id, alias, args[0].span())?;
                let signature = self.context.symbols[target]
                    .functions
                    .get(property)
                    .cloned()
                    .ok_or_else(|| {
                        self.error(
                            DiagnosticCategory::UnresolvedFunction,
                            format!("module '{alias}' has no task entry '{property}'"),
                            args[0].span(),
                        )
                    })?;
                if !signature.public {
                    return Err(self.error(
                        DiagnosticCategory::UnresolvedFunction,
                        format!("task entry '{alias}.{property}' is module-private"),
                        args[0].span(),
                    ));
                }
                (signature, format!("{alias}.{property}"), true)
            }
            _ => {
                return Err(self.error(
                    DiagnosticCategory::InvalidIntrinsic,
                    "task_address entry must be a public function symbol",
                    args[0].span(),
                ))
            }
        };
        if !signature.public
            || signature.interrupt.is_some()
            || !signature.params.is_empty()
            || signature.return_type != SystemType::I32
            || signature.symbol == "kernel_main"
        {
            return Err(self.error(
                DiagnosticCategory::InvalidIntrinsic,
                format!("task entry '{source_name}' must be public non-interrupt fn() -> int and cannot be kernel_main"),
                args[0].span(),
            ));
        }
        if external {
            self.external_functions
                .entry(signature.symbol.clone())
                .or_insert_with(|| ExternalFunction {
                    symbol: signature.symbol.clone(),
                    parameter_types: Vec::new(),
                    return_type: SystemType::I32,
                });
        }
        self.instructions.push(SystemsInst::TaskAddress {
            symbol: signature.symbol,
        });
        Ok(ExprResult::Value(SystemType::Usize))
    }

    fn compile_lvalue(&mut self, expression: &Expr) -> NativeResult<LValue> {
        match expression {
            Expr::Ident(name, span) => {
                if let Some(binding) = self.lookup(name) {
                    self.instructions
                        .push(SystemsInst::LocalAddress(binding.slot));
                    return Ok(LValue {
                        ty: binding.ty,
                        mutable: true,
                        legacy_string_index: false,
                    });
                }
                if let Some(global) = self.context.symbols[self.module_id]
                    .globals
                    .get(name)
                    .cloned()
                {
                    self.instructions.push(SystemsInst::GlobalAddress {
                        symbol: global.symbol,
                        pointee: global.ty.clone(),
                    });
                    return Ok(LValue {
                        ty: global.ty,
                        mutable: global.mutable,
                        legacy_string_index: false,
                    });
                }
                Err(self.error(
                    DiagnosticCategory::UnknownSymbol,
                    format!("unknown native lvalue '{name}'"),
                    *span,
                ))
            }
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
                span,
            } => {
                let pointer = self.compile_value(expr, expr.span(), "dereference operand")?;
                let SystemType::Pointer(pointee) = pointer else {
                    return Err(self.error(
                        DiagnosticCategory::BadDereference,
                        format!("cannot dereference native type {pointer}"),
                        *span,
                    ));
                };
                Ok(LValue {
                    ty: *pointee,
                    mutable: true,
                    legacy_string_index: false,
                })
            }
            Expr::Index {
                target,
                index,
                span,
            } => self.compile_index_lvalue(target, index, *span),
            Expr::Member {
                object,
                property,
                span,
            } => self.compile_member_lvalue(object, property, *span),
            _ => Err(self.error(
                DiagnosticCategory::InvalidPointerOperation,
                "expression is not an addressable native lvalue",
                expression.span(),
            )),
        }
    }

    fn compile_index_lvalue(
        &mut self,
        target: &Expr,
        index: &Expr,
        span: Span,
    ) -> NativeResult<LValue> {
        let known = self.known_storage_type(target);
        let (element, mutable, legacy_string_index, bound) = match known {
            Some(SystemType::Array(element, length)) => {
                let base = self.compile_lvalue(target)?;
                (*element, base.mutable, false, Some(length))
            }
            _ => {
                let target_type = self.compile_value(target, target.span(), "index target")?;
                match target_type {
                    SystemType::Pointer(element) => (*element, true, false, None),
                    SystemType::Str => (SystemType::U8, false, true, None),
                    other => {
                        return Err(self.error(
                            DiagnosticCategory::InvalidPointerOperation,
                            format!(
                                "native indexing requires an array, pointer, or str, found {other}"
                            ),
                            span,
                        ))
                    }
                }
            }
        };
        if let (Some(length), Some(value)) = (bound, signed_literal(index)) {
            if value < 0 || value >= i64::from(length) {
                return Err(self.error(
                    DiagnosticCategory::BadArrayLength,
                    format!("array index {value} is outside 0..{length}"),
                    index.span(),
                ));
            }
        }
        let index_ty = self.compile_value(index, index.span(), "array index")?;
        if !index_ty.is_integer() {
            return Err(self.error(
                DiagnosticCategory::InvalidType,
                format!("native index must be an integer, found {index_ty}"),
                index.span(),
            ));
        }
        let element_size = self
            .context
            .layouts
            .layout_of(&element, span)
            .map_err(|error| self.attach(error))?
            .size;
        self.instructions
            .push(SystemsInst::IndexAddress { element_size });
        Ok(LValue {
            ty: element,
            mutable,
            legacy_string_index,
        })
    }

    fn compile_member_lvalue(
        &mut self,
        object: &Expr,
        property: &str,
        span: Span,
    ) -> NativeResult<LValue> {
        if let Expr::Ident(alias, _) = object {
            if self.context.headers[self.module_id]
                .imports
                .contains_key(alias)
            {
                return self.compile_imported_global_lvalue(alias, property, span);
            }
        }
        let base = self.compile_lvalue(object)?;
        let SystemType::Struct(name) = &base.ty else {
            return Err(self.error(
                DiagnosticCategory::InvalidField,
                format!("native field access requires a struct, found {}", base.ty),
                span,
            ));
        };
        let layout = self.context.layouts.structure(name).ok_or_else(|| {
            self.error(
                DiagnosticCategory::UnknownType,
                format!("unknown native struct '{name}'"),
                span,
            )
        })?;
        let field = layout.field(property).cloned().ok_or_else(|| {
            self.error(
                DiagnosticCategory::InvalidField,
                format!("struct '{name}' has no field '{property}'"),
                span,
            )
        })?;
        self.instructions.push(SystemsInst::FieldAddress {
            offset: field.offset,
        });
        Ok(LValue {
            ty: field.ty,
            mutable: base.mutable,
            legacy_string_index: false,
        })
    }

    fn compile_imported_global_lvalue(
        &mut self,
        alias: &str,
        name: &str,
        span: Span,
    ) -> NativeResult<LValue> {
        let target = import_target(self.context, self.module_id, alias, span)?;
        let global = self.context.symbols[target]
            .globals
            .get(name)
            .cloned()
            .ok_or_else(|| {
                self.error(
                    DiagnosticCategory::UnknownSymbol,
                    format!(
                        "module '{}' has no native static '{name}'",
                        self.context.graph.modules[target].name
                    ),
                    span,
                )
            })?;
        if !global.public {
            return Err(self.error(
                DiagnosticCategory::UnknownSymbol,
                format!(
                    "native static '{}.{name}' is module-private",
                    self.context.graph.modules[target].name
                ),
                span,
            ));
        }
        self.external_globals
            .entry(global.symbol.clone())
            .or_insert_with(|| ExternalGlobal {
                symbol: global.symbol.clone(),
                ty: global.ty.clone(),
                mutable: global.mutable,
            });
        self.instructions.push(SystemsInst::GlobalAddress {
            symbol: global.symbol,
            pointee: global.ty.clone(),
        });
        Ok(LValue {
            ty: global.ty,
            mutable: global.mutable,
            legacy_string_index: false,
        })
    }

    fn compile_imported_value(
        &mut self,
        alias: &str,
        name: &str,
        span: Span,
    ) -> NativeResult<ExprResult> {
        let target = import_target(self.context, self.module_id, alias, span)?;
        if let Some(declaration) = self.context.symbols[target].constants.get(name) {
            if !declaration.public {
                return Err(self.error(
                    DiagnosticCategory::UnknownSymbol,
                    format!(
                        "native constant '{}.{name}' is module-private",
                        self.context.graph.modules[target].name
                    ),
                    span,
                ));
            }
            let value = self.context.constants[&(target, name.to_owned())].clone();
            self.emit_constant(&value);
            return Ok(ExprResult::Value(value.ty));
        }
        let lvalue = self.compile_imported_global_lvalue(alias, name, span)?;
        if !lvalue.ty.is_scalar() {
            return Err(self.error(
                DiagnosticCategory::InvalidType,
                format!("aggregate imported static '{alias}.{name}' cannot be copied by value"),
                span,
            ));
        }
        self.instructions.push(SystemsInst::Load(lvalue.ty.clone()));
        Ok(ExprResult::Value(lvalue.ty))
    }

    fn known_storage_type(&self, expression: &Expr) -> Option<SystemType> {
        match expression {
            Expr::Ident(name, _) => self.lookup(name).map(|binding| binding.ty).or_else(|| {
                self.context.symbols[self.module_id]
                    .globals
                    .get(name)
                    .map(|global| global.ty.clone())
            }),
            Expr::Member {
                object, property, ..
            } => {
                if let Expr::Ident(alias, _) = object.as_ref() {
                    if let Some(target) = self.context.headers[self.module_id].imports.get(alias) {
                        return self.context.symbols[*target]
                            .globals
                            .get(property)
                            .filter(|global| global.public)
                            .map(|global| global.ty.clone());
                    }
                }
                let base = self.known_storage_type(object)?;
                let SystemType::Struct(name) = base else {
                    return None;
                };
                self.context
                    .layouts
                    .structure(&name)?
                    .field(property)
                    .map(|field| field.ty.clone())
            }
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
                ..
            } => {
                let SystemType::Pointer(pointee) = self.known_value_type(expr)? else {
                    return None;
                };
                Some(*pointee)
            }
            Expr::Index { target, .. } => match self.known_storage_type(target) {
                Some(SystemType::Array(element, _)) => Some(*element),
                _ => match self.known_value_type(target) {
                    Some(SystemType::Pointer(element)) => Some(*element),
                    Some(SystemType::Str) => Some(SystemType::U8),
                    _ => None,
                },
            },
            _ => None,
        }
    }

    fn known_value_type(&self, expression: &Expr) -> Option<SystemType> {
        match expression {
            Expr::IntLit(_, _) => Some(SystemType::I32),
            Expr::BoolLit(_, _) => Some(SystemType::Bool),
            Expr::StrLit(_, _) => Some(SystemType::Str),
            Expr::Ident(name, _) => self
                .lookup(name)
                .map(|binding| binding.ty)
                .or_else(|| {
                    self.context
                        .constants
                        .get(&(self.module_id, name.clone()))
                        .map(|value| value.ty.clone())
                })
                .or_else(|| {
                    self.context.symbols[self.module_id]
                        .globals
                        .get(name)
                        .map(|global| global.ty.clone())
                }),
            Expr::Cast { ty, span, .. } => resolve_annotation(
                self.context.graph,
                &self.context.headers,
                self.module_id,
                ty,
                *span,
            )
            .ok(),
            Expr::Unary {
                op: UnaryOp::AddressOf,
                expr,
                ..
            } => self
                .known_storage_type(expr)
                .map(|ty| SystemType::Pointer(Box::new(ty))),
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
                ..
            } => match self.known_value_type(expr) {
                Some(SystemType::Pointer(pointee)) => Some(*pointee),
                _ => None,
            },
            Expr::Index { target, .. } => match self.known_storage_type(target) {
                Some(SystemType::Array(element, _)) => Some(*element),
                _ => match self.known_value_type(target) {
                    Some(SystemType::Pointer(element)) => Some(*element),
                    Some(SystemType::Str) => Some(SystemType::I32),
                    _ => None,
                },
            },
            Expr::Member {
                object, property, ..
            } => {
                if let Expr::Ident(alias, _) = object.as_ref() {
                    if let Some(target) = self.context.headers[self.module_id].imports.get(alias) {
                        if let Some(value) = self.context.symbols[*target].constants.get(property) {
                            return value.public.then(|| value.ty.clone());
                        }
                    }
                }
                self.known_storage_type(expression)
            }
            Expr::Call { name, .. } => self.context.symbols[self.module_id]
                .functions
                .get(name)
                .map(|signature| signature.return_type.clone()),
            Expr::MemberCall {
                object, property, ..
            } => {
                let Expr::Ident(alias, _) = object.as_ref() else {
                    return None;
                };
                if alias == "native" {
                    return SystemIntrinsic::from_name(property)
                        .and_then(|item| item.result_type());
                }
                let target = *self.context.headers[self.module_id].imports.get(alias)?;
                self.context.symbols[target]
                    .functions
                    .get(property)
                    .filter(|signature| signature.public)
                    .map(|signature| signature.return_type.clone())
            }
            _ => None,
        }
    }

    fn emit_constant(&mut self, value: &ConstantValue) {
        if value.ty == SystemType::Bool {
            self.instructions
                .push(SystemsInst::ConstBool(value.bits != 0));
        } else {
            self.instructions.push(SystemsInst::Const {
                bits: value.bits,
                ty: value.ty.clone(),
            });
        }
    }

    fn allocate_slot(
        &mut self,
        name: &str,
        ty: SystemType,
        is_parameter: bool,
        span: Span,
    ) -> NativeResult<LocalId> {
        self.ensure_local_available(name, span)?;
        let id = LocalId(u32::try_from(self.slots.len()).map_err(|_| {
            self.error(
                DiagnosticCategory::UnsupportedFeature,
                "native function exceeds the u32 local-slot limit",
                span,
            )
        })?);
        let layout = self
            .context
            .layouts
            .layout_of(&ty, span)
            .map_err(|error| self.attach(error))?;
        self.slots.push(SystemsSlot {
            id,
            name: name.to_owned(),
            ty: ty.clone(),
            storage_size: layout.size,
            alignment: layout.alignment,
            is_parameter,
        });
        self.scopes
            .last_mut()
            .expect("function always has a lexical scope")
            .insert(name.to_owned(), Binding { slot: id, ty });
        Ok(id)
    }

    fn ensure_local_available(&self, name: &str, span: Span) -> NativeResult<()> {
        if name == "native"
            || self.context.headers[self.module_id]
                .imports
                .contains_key(name)
        {
            return Err(self.error(
                DiagnosticCategory::DuplicateLocal,
                format!("local '{name}' conflicts with a native namespace"),
                span,
            ));
        }
        if self
            .scopes
            .last()
            .is_some_and(|scope| scope.contains_key(name))
        {
            return Err(self.error(
                DiagnosticCategory::DuplicateLocal,
                format!("duplicate local '{name}'"),
                span,
            ));
        }
        Ok(())
    }

    fn lookup(&self, name: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).cloned())
    }

    fn new_label(&mut self, span: Span) -> NativeResult<LabelId> {
        let label = LabelId(self.next_label);
        self.next_label = self.next_label.checked_add(1).ok_or_else(|| {
            self.error(
                DiagnosticCategory::UnsupportedFeature,
                "native function exceeds the u32 label limit",
                span,
            )
        })?;
        Ok(label)
    }

    fn intern_string(&mut self, bytes: &[u8], span: Span) -> NativeResult<StringId> {
        if let Some(id) = self.string_ids.get(bytes) {
            return Ok(*id);
        }
        let id = StringId(u32::try_from(self.strings.len()).map_err(|_| {
            self.error(
                DiagnosticCategory::UnsupportedFeature,
                "native module exceeds the u32 static-string limit",
                span,
            )
        })?);
        let owned = bytes.to_vec();
        self.strings.push(owned.clone());
        self.string_ids.insert(owned, id);
        Ok(id)
    }

    fn compile_value(
        &mut self,
        expression: &Expr,
        span: Span,
        context: &str,
    ) -> NativeResult<SystemType> {
        self.compile_value_expected(expression, None, span, context)
    }

    fn compile_value_expected(
        &mut self,
        expression: &Expr,
        expected: Option<&SystemType>,
        span: Span,
        context: &str,
    ) -> NativeResult<SystemType> {
        let result = self.compile_expr_expected(expression, expected)?;
        self.require_value(result, span, context)
    }

    fn require_value(
        &self,
        result: ExprResult,
        span: Span,
        context: &str,
    ) -> NativeResult<SystemType> {
        match result {
            ExprResult::Value(ty) => Ok(ty),
            ExprResult::Void => Err(self.error(
                DiagnosticCategory::InvalidType,
                format!("{context} requires a value, found void intrinsic result"),
                span,
            )),
        }
    }

    fn require_same_type(
        &self,
        expected: &SystemType,
        actual: &SystemType,
        span: Span,
        owner: impl std::fmt::Display,
    ) -> NativeResult<()> {
        if expected == actual {
            Ok(())
        } else {
            Err(self.error(
                DiagnosticCategory::InvalidType,
                format!("{owner} expects {expected}, found {actual}"),
                span,
            ))
        }
    }

    fn error(
        &self,
        category: DiagnosticCategory,
        message: impl Into<String>,
        span: Span,
    ) -> Diagnostic {
        module_diagnostic(
            &self.context.graph.modules[self.module_id],
            category,
            message,
            span,
        )
    }

    fn unsupported(&self, feature: &str, span: Span) -> Diagnostic {
        self.error(
            DiagnosticCategory::UnsupportedFeature,
            format!("{feature} are unsupported in the freestanding native systems profile"),
            span,
        )
    }

    fn attach(&self, error: Diagnostic) -> Diagnostic {
        attach_module_path(error, &self.context.graph.modules[self.module_id])
    }
}

fn block_always_returns(statements: &[Stmt]) -> bool {
    statements.iter().any(statement_always_returns)
}

fn statement_always_returns(statement: &Stmt) -> bool {
    match statement {
        Stmt::Return { .. } => true,
        Stmt::Expr(Expr::MemberCall {
            object, property, ..
        }) => {
            matches!(object.as_ref(), Expr::Ident(name, _) if name == "native")
                && property == "interrupt_resume"
        }
        Stmt::If {
            then_body,
            else_body: Some(else_body),
            ..
        } => block_always_returns(then_body) && block_always_returns(else_body),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::NativeImport;
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;
    use std::path::{Path, PathBuf};

    fn test_graph(sources: &[(&str, &str)], root_name: &str) -> NativeModuleGraph {
        let ids: BTreeMap<&str, usize> = sources
            .iter()
            .enumerate()
            .map(|(id, (name, _))| (*name, id))
            .collect();
        let mut modules = Vec::new();
        for (id, (name, source)) in sources.iter().enumerate() {
            let tokens = Lexer::new(source).tokenize().expect("test source lexes");
            let program = Parser::new(tokens)
                .parse_program()
                .expect("test source parses");
            let imports = program
                .stmts
                .iter()
                .filter_map(|statement| {
                    let Stmt::Import { path, alias, span } = statement else {
                        return None;
                    };
                    let target_name = Path::new(path)
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .expect("test import has a UTF-8 stem");
                    Some(NativeImport {
                        source_path: path.clone(),
                        alias: alias.clone(),
                        target: ids[target_name],
                        span: *span,
                    })
                })
                .collect();
            modules.push(NativeSourceModule {
                id,
                name: (*name).to_owned(),
                path: PathBuf::from(format!("{name}.dby")),
                display_path: format!("tests/{name}.dby"),
                program,
                imports,
            });
        }
        NativeModuleGraph {
            root: ids[root_name],
            modules,
        }
    }

    fn one_module(source: &str) -> NativeModuleGraph {
        test_graph(&[("kernel", source)], "kernel")
    }

    fn proof_graph() -> NativeModuleGraph {
        test_graph(
            &[
                (
                    "arch_x86",
                    include_str!("../../../examples/native_kernel_multimodule/arch_x86.dby"),
                ),
                (
                    "kernel",
                    include_str!("../../../examples/native_kernel_multimodule/kernel.dby"),
                ),
                (
                    "memory",
                    include_str!("../../../examples/native_kernel_multimodule/memory.dby"),
                ),
                (
                    "numbers",
                    include_str!("../../../examples/native_kernel_multimodule/numbers.dby"),
                ),
                (
                    "serial",
                    include_str!("../../../examples/native_kernel_multimodule/serial.dby"),
                ),
            ],
            "kernel",
        )
    }

    #[test]
    fn five_module_proof_covers_systems_ir_and_global_sections() {
        let project = analyze_systems_project(&proof_graph()).expect("proof project analyzes");
        assert_eq!(project.modules.len(), 5);
        assert_eq!(project.root_module().module_name, "kernel");

        let memory = project
            .modules
            .iter()
            .find(|module| module.module_name == "memory")
            .unwrap();
        let build_id = memory
            .globals
            .iter()
            .find(|global| global.source_name == "BUILD_ID")
            .unwrap();
        assert_eq!(build_id.section, GlobalSection::ReadOnly);
        assert_eq!(build_id.bytes, 11u32.to_le_bytes());
        let cells = memory
            .globals
            .iter()
            .find(|global| global.source_name == "CELLS")
            .unwrap();
        assert_eq!(cells.section, GlobalSection::Data);
        assert_eq!(cells.bytes, [1, 2, 3, 4]);
        let total = memory
            .globals
            .iter()
            .find(|global| global.source_name == "TOTAL")
            .unwrap();
        assert_eq!(total.section, GlobalSection::Bss);
        assert!(total.bytes.is_empty());

        let dump = project.dump();
        for required in [
            "address.index scale=1",
            "address.field",
            "store.u8",
            "shl.u8",
            "xor.u16",
            "cast u8 -> u32",
            "native.io_out8",
            "extern fn compute(u32) -> u32",
        ] {
            assert!(
                dump.contains(required),
                "missing IR proof: {required}\n{dump}"
            );
        }
        let record = project.layouts.structure("memory::SystemRecord").unwrap();
        let packed = project.layouts.structure("memory::PackedProof").unwrap();
        assert_eq!((record.size, record.alignment), (8, 4));
        assert_eq!((packed.size, packed.alignment), (3, 1));
    }

    #[test]
    fn analysis_is_byte_for_byte_deterministic() {
        let graph = proof_graph();
        let first = analyze_systems_project(&graph).unwrap();
        let second = analyze_systems_project(&graph).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.dump(), second.dump());
    }

    #[test]
    fn standalone_object_analysis_skips_only_integrated_entry_validation() {
        let graph = test_graph(
            &[(
                "arch_x86",
                include_str!("../../../examples/native_kernel_multimodule/arch_x86.dby"),
            )],
            "arch_x86",
        );
        let object = analyze_systems_objects(&graph).unwrap();
        assert!(object.root_module().function("out8").is_some());

        let error = analyze_systems_project(&graph).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::InvalidEntryPoint);
        assert!(error.message.contains("kernel_main"));
    }

    #[test]
    fn contextual_integer_boundaries_are_checked() {
        let valid = one_module(
            "const LO: i8 = -128 as i8\n\
             const HI: u8 = 255 as u8\n\
             static WORD: u16 = 65535 as u16\n\
             fn kernel_main() -> int:\n    let byte: u8 = HI\n    if 255 == byte:\n        return LO as i32\n    return 0\n",
        );
        let project = analyze_systems_project(&valid).unwrap();
        assert_eq!(project.root_module().globals[0].bytes, [255, 255]);

        for source in [
            "fn kernel_main() -> int:\n    let bad: u8 = 256 as u8\n    return 0\n",
            "fn kernel_main() -> int:\n    let bad: i8 = -129 as i8\n    return 0\n",
        ] {
            assert_eq!(
                analyze_systems_project(&one_module(source))
                    .unwrap_err()
                    .category,
                DiagnosticCategory::IntegerOutOfRange
            );
        }
    }

    #[test]
    fn visibility_and_duplicate_exports_are_project_wide() {
        let private = test_graph(
            &[
                (
                    "dep",
                    "fn secret() -> int:\n    return 1\n",
                ),
                (
                    "kernel",
                    "import \"dep.dby\" as dep\nfn kernel_main() -> int:\n    return dep.secret()\n",
                ),
            ],
            "kernel",
        );
        assert_eq!(
            analyze_systems_project(&private).unwrap_err().category,
            DiagnosticCategory::UnresolvedFunction
        );

        let duplicate = test_graph(
            &[
                ("alpha", "pub fn clash() -> int:\n    return 1\n"),
                ("beta", "pub fn clash() -> int:\n    return 2\n"),
                (
                    "kernel",
                    "import \"alpha.dby\" as alpha\nimport \"beta.dby\" as beta\nfn kernel_main() -> int:\n    return 0\n",
                ),
            ],
            "kernel",
        );
        assert_eq!(
            analyze_systems_project(&duplicate).unwrap_err().category,
            DiagnosticCategory::DuplicateModuleSymbol
        );

        let entry_collision = test_graph(
            &[
                (
                    "dependency",
                    "pub fn kernel_main() -> int:\n    return 1\n",
                ),
                (
                    "kernel",
                    "import \"dependency.dby\" as dependency\nfn kernel_main() -> int:\n    return 0\n",
                ),
            ],
            "kernel",
        );
        let error = analyze_systems_project(&entry_collision).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::DuplicateModuleSymbol);
        assert!(error.message.contains("duplicate emitted native symbol"));
    }

    #[test]
    fn invalid_systems_operations_have_stable_categories() {
        let cases = [
            (
                "fn kernel_main() -> int:\n    let value: i32 = 1\n    return *value\n",
                DiagnosticCategory::BadDereference,
            ),
            (
                "struct Pair:\n    left: i32\nfn kernel_main() -> int:\n    let pair: Pair = Pair(1)\n    return pair.right\n",
                DiagnosticCategory::InvalidField,
            ),
            (
                "fn kernel_main() -> int:\n    let values: array[u8, 2] = [1 as u8, 2 as u8]\n    return values[2] as i32\n",
                DiagnosticCategory::BadArrayLength,
            ),
            (
                "fn kernel_main() -> int:\n    let value: i32 = 1\n    return value[0]\n",
                DiagnosticCategory::InvalidPointerOperation,
            ),
            (
                "fn kernel_main() -> int:\n    return missing.call()\n",
                DiagnosticCategory::InvalidModuleImport,
            ),
            (
                "fn kernel_main(value: i32) -> int:\n    return value\n",
                DiagnosticCategory::InvalidEntryPoint,
            ),
        ];
        for (source, category) in cases {
            let error = analyze_systems_project(&one_module(source)).unwrap_err();
            assert_eq!(error.category, category, "{error}");
            assert_eq!(error.path.as_deref(), Some("tests/kernel.dby"));
            assert!(error.span.is_some());
        }
    }

    #[test]
    fn malformed_public_graph_input_is_diagnosed_without_indexing_panics() {
        let source = "fn kernel_main() -> int:\n    return 0\n";
        let mut invalid_root = one_module(source);
        invalid_root.root = 7;
        assert_eq!(
            analyze_systems_project(&invalid_root).unwrap_err().category,
            DiagnosticCategory::InvalidModuleImport
        );

        let mut invalid_id = one_module(source);
        invalid_id.modules[0].id = 7;
        let error = analyze_systems_project(&invalid_id).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::InvalidModuleImport);
        assert!(error.message.contains("expected deterministic index"));
    }

    #[test]
    fn multiboot_entry_and_memory_intrinsics_are_typed_without_host_fallback() {
        let source = "fn kernel_main(multiboot_magic: u32, multiboot_info: usize) -> int:\n    let cr0: u32 = native.cpu_read_cr0()\n    let cr2: usize = native.cpu_read_cr2()\n    let cr3: usize = native.cpu_read_cr3()\n    native.cpu_write_cr3(cr3)\n    native.cpu_write_cr0(cr0)\n    native.cpu_invlpg(cr2 as ptr[u8])\n    let start: usize = native.kernel_image_start()\n    let end: usize = native.kernel_image_end()\n    let stack_start: usize = native.bootstrap_stack_start()\n    let stack_end: usize = native.bootstrap_stack_end()\n    return (multiboot_magic as i32) + (multiboot_info as i32) + (start as i32) + (end as i32) + (stack_start as i32) + (stack_end as i32)\n";
        let project = analyze_systems_project(&one_module(source)).unwrap();
        let kernel = project.modules[0].function("kernel_main").unwrap();
        assert_eq!(
            kernel.parameter_types,
            vec![SystemType::U32, SystemType::Usize]
        );
        assert!(kernel.instructions.iter().any(|instruction| matches!(
            instruction,
            SystemsInst::LinkerAddress { symbol } if symbol == "__dbyte_image_start"
        )));
        assert!(kernel.instructions.iter().any(|instruction| matches!(
            instruction,
            SystemsInst::Intrinsic {
                intrinsic: SystemIntrinsic::CpuInvlpg,
                ..
            }
        )));
    }

    #[test]
    fn interrupt_address_preserves_class_and_rejects_direct_calls() {
        let source = "struct InterruptFrame:\n    eip: u32\n    cs: u32\n    eflags: u32\ninterrupt(no_error_code) fn breakpoint(frame: ptr[InterruptFrame]):\n    return\nfn kernel_main() -> int:\n    interrupt_address(3, breakpoint)\n    return 0\n";
        let project = analyze_systems_project(&one_module(source)).unwrap();
        let kernel = project.modules[0].function("kernel_main").unwrap();
        assert!(kernel.instructions.iter().any(|instruction| matches!(
            instruction,
            SystemsInst::InterruptAddress {
                vector: 3,
                class: InterruptClass::NoErrorCode,
                ..
            }
        )));

        let direct = source.replace(
            "interrupt_address(3, breakpoint)",
            "breakpoint(0 as usize as ptr[InterruptFrame])",
        );
        assert_eq!(
            analyze_systems_project(&one_module(&direct))
                .unwrap_err()
                .category,
            DiagnosticCategory::InvalidType
        );
    }

    #[test]
    fn interrupt_vector_class_mismatch_is_compile_time_error() {
        let source = "struct InterruptFrame:\n    eip: u32\n    cs: u32\n    eflags: u32\ninterrupt(no_error_code) fn page_fault(frame: ptr[InterruptFrame]):\n    return\nfn kernel_main() -> int:\n    interrupt_address(14, page_fault)\n    return 0\n";
        let error = analyze_systems_project(&one_module(source)).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::InterruptClassMismatch);
        assert!(error.message.contains("vector 14 requires error_code"));
    }

    #[test]
    fn checked_task_addresses_and_terminal_resume_are_typed_narrowly() {
        let source = "pub fn worker() -> int:\n    return 0\nfn switch(resume: usize) -> int:\n    native.interrupt_resume(resume)\nfn kernel_main() -> int:\n    let entry: usize = task_address(worker)\n    let trampoline: usize = native.task_trampoline_address()\n    return (entry + trampoline) as i32\n";
        let project = analyze_systems_project(&one_module(source)).unwrap();
        let kernel = project.modules[0].function("kernel_main").unwrap();
        assert!(kernel.instructions.iter().any(|instruction| matches!(
            instruction,
            SystemsInst::TaskAddress { symbol } if symbol == "worker"
        )));
        assert!(kernel.instructions.iter().any(|instruction| matches!(
            instruction,
            SystemsInst::LinkerAddress { symbol } if symbol == "__dbyte_task_trampoline"
        )));
        let switch = project.modules[0]
            .functions
            .iter()
            .find(|function| function.source_name == "switch")
            .unwrap();
        assert!(switch.instructions.iter().any(|instruction| matches!(
            instruction,
            SystemsInst::Intrinsic {
                intrinsic: SystemIntrinsic::InterruptResume,
                ..
            }
        )));

        for invalid in [
            "fn worker() -> int:\n    return 0\nfn kernel_main() -> int:\n    task_address(worker)\n    return 0\n",
            "pub fn worker(value: usize) -> int:\n    return value as i32\nfn kernel_main() -> int:\n    task_address(worker)\n    return 0\n",
            "fn kernel_main() -> int:\n    task_address(kernel_main)\n    return 0\n",
        ] {
            assert_eq!(
                analyze_systems_project(&one_module(invalid))
                    .unwrap_err()
                    .category,
                DiagnosticCategory::InvalidIntrinsic
            );
        }

        let error_code = "struct InterruptFrame:\n    eip: u32\n    cs: u32\n    eflags: u32\ninterrupt(error_code) fn fault(frame: ptr[InterruptFrame], error: u32):\n    native.interrupt_resume(frame as usize)\nfn kernel_main() -> int:\n    return 0\n";
        assert_eq!(
            analyze_systems_project(&one_module(error_code))
                .unwrap_err()
                .category,
            DiagnosticCategory::InvalidIntrinsic
        );
    }
}
