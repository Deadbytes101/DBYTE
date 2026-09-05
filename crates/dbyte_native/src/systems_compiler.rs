//! End-to-end systems project compilation through real ELF32 objects and the
//! in-process DByte static linker.

use crate::diagnostics::{Diagnostic, NativeResult};
use crate::linker::{link_object_bytes, link_user_object_bytes, LinkArtifact, LinkInput};
use crate::modules::load_native_module_graph;
use crate::object::{
    write_relocatable, ObjectSection, ObjectSymbol, SymbolBinding, SymbolDefinition, SymbolKind,
    DROP_METADATA_PREFIX, GENERIC_METADATA_PREFIX,
};
use crate::systems_lower::lower_systems_module;
#[cfg(test)]
use crate::systems_semantic::{
    analyze_systems_objects, analyze_systems_project, analyze_user_objects, analyze_user_project,
    SystemsProfile,
};
use std::path::{Path, PathBuf};

pub fn compile_canonical_legacy_single_image(
    root: &Path,
) -> NativeResult<Option<crate::NativeArtifact>> {
    let graph = load_native_module_graph(root)?;
    if graph.modules.len() != 1 || !graph.modules[0].imports.is_empty() {
        return Ok(None);
    }
    let input = dbyte_hir::ProjectInput {
        root: dbyte_hir::ModuleId(graph.root as u32),
        modules: graph
            .modules
            .iter()
            .map(|module| dbyte_hir::ProjectModuleInput {
                id: dbyte_hir::ModuleId(module.id as u32),
                name: module.name.clone(),
                program: module.program.clone(),
                imports: Vec::new(),
            })
            .collect(),
    };
    let analyzed = dbyte_hir::analyze_project(
        &input,
        dbyte_hir::Profile::I686None,
        dbyte_hir::TargetDataLayout::I686,
    )
    .map_err(|errors| canonical_project_diagnostics(errors, &graph))?;
    let concrete = dbyte_hir::monomorphize_project(&analyzed)
        .map_err(|error| canonical_project_diagnostic(error, &graph))?;
    validate_canonical_entry(&concrete, dbyte_hir::Profile::I686None)?;
    let Some(ir) = crate::legacy_hir::lower_legacy_single_image(&analyzed, &concrete.executable)?
    else {
        return Ok(None);
    };
    let ir_dump = ir.dump();
    let lowered = crate::lower::lower_program(&ir)?;
    let elf = crate::elf32::write_executable(&ir, &lowered)?;
    let mut machine_dump = lowered.dump;
    if !machine_dump.ends_with('\n') {
        machine_dump.push('\n');
    }
    machine_dump.push_str(&elf.dump);
    Ok(Some(crate::NativeArtifact {
        elf: elf.elf,
        bin: elf.bin,
        machine_code: elf.machine_code,
        layout: elf.layout,
        ir_dump,
        machine_dump,
    }))
}

/// Compile one already-loaded module through the canonical executable HIR
/// seam. This is the staging entry used while project-wide canonical module
/// ownership is brought online; it deliberately has no legacy fallback.
pub fn compile_canonical_module_object(
    module_name: &str,
    program: &dbyte_ast::Program,
    profile: dbyte_hir::Profile,
) -> NativeResult<CompiledModuleObject> {
    let executable =
        dbyte_hir::analyze_executable(program, profile, dbyte_hir::TargetDataLayout::I686)
            .map_err(canonical_diagnostics)?;
    let concrete = dbyte_hir::monomorphize(&executable).map_err(canonical_diagnostic)?;
    let ir_module = crate::hir_native::lower_hir_module_to_native_ir(module_name, &concrete)?;
    let lowered = lower_systems_module(&ir_module)?;
    let object = write_relocatable(&lowered.image)?;
    Ok(CompiledModuleObject {
        module_id: 0,
        module_name: module_name.to_owned(),
        source_path: PathBuf::new(),
        display_path: module_name.to_owned(),
        bytes: object.bytes,
        ir_dump: ir_module.dump(),
        machine_dump: lowered.machine_dump,
        object_dump: object.dump,
    })
}

pub fn compile_canonical_project_objects(
    root: &Path,
    profile: dbyte_hir::Profile,
) -> NativeResult<CompiledProjectObjects> {
    compile_canonical_project_objects_with_entry(root, profile, false)
}

fn compile_canonical_project_objects_with_entry(
    root: &Path,
    profile: dbyte_hir::Profile,
    require_entry: bool,
) -> NativeResult<CompiledProjectObjects> {
    let graph = load_native_module_graph(root)?;
    let input = dbyte_hir::ProjectInput {
        root: dbyte_hir::ModuleId(graph.root as u32),
        modules: graph
            .modules
            .iter()
            .map(|module| dbyte_hir::ProjectModuleInput {
                id: dbyte_hir::ModuleId(module.id as u32),
                name: module.name.clone(),
                program: module.program.clone(),
                imports: module
                    .imports
                    .iter()
                    .map(|import| {
                        (
                            import.alias.clone(),
                            dbyte_hir::ModuleId(import.target as u32),
                        )
                    })
                    .collect(),
            })
            .collect(),
    };
    let analyzed = dbyte_hir::analyze_project(&input, profile, dbyte_hir::TargetDataLayout::I686)
        .map_err(|errors| canonical_project_diagnostics(errors, &graph))?;
    let concrete = dbyte_hir::monomorphize_project(&analyzed)
        .map_err(|error| canonical_project_diagnostic(error, &graph))?;
    if require_entry {
        validate_canonical_entry(&concrete, profile)?;
    }
    let names = graph
        .modules
        .iter()
        .map(|module| (dbyte_hir::ModuleId(module.id as u32), module.name.clone()))
        .collect();
    let ir_modules =
        crate::hir_native::lower_hir_project_to_native_ir(&analyzed, &concrete.executable, &names)?;
    let mut modules = Vec::with_capacity(ir_modules.len());
    for (module_id, ir_module) in ir_modules {
        let source_module = &graph.modules[module_id.0 as usize];
        let mut lowered = lower_systems_module(&ir_module)
            .map_err(|error| attach_path(error, &source_module.display_path))?;
        for marker in generic_metadata_markers(&analyzed, module_id) {
            lowered.image.symbols.push(ObjectSymbol {
                name: marker,
                binding: SymbolBinding::Local,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Text,
                    value: 0,
                },
                size: 0,
            });
        }
        for marker in drop_metadata_markers(&analyzed, module_id) {
            lowered.image.symbols.push(ObjectSymbol {
                name: marker,
                binding: SymbolBinding::Local,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Text,
                    value: 0,
                },
                size: 0,
            });
        }
        let object = write_relocatable(&lowered.image)
            .map_err(|error| attach_path(error, &source_module.display_path))?;
        modules.push(CompiledModuleObject {
            module_id: source_module.id,
            module_name: source_module.name.clone(),
            source_path: source_module.path.clone(),
            display_path: source_module.display_path.clone(),
            bytes: object.bytes,
            ir_dump: ir_module.dump(),
            machine_dump: lowered.machine_dump,
            object_dump: object.dump,
        });
    }
    Ok(CompiledProjectObjects {
        root_module: graph.root,
        modules,
    })
}

fn generic_metadata_markers(
    project: &dbyte_hir::CanonicalHirProject,
    module: dbyte_hir::ModuleId,
) -> Vec<String> {
    let mut markers = Vec::new();
    for function in &project.executable.semantic.functions {
        if let Some(generic) = function.generic.filter(|_| {
            function.public && project.symbol_owners.get(&function.symbol) == Some(&module)
        }) {
            markers.push(format!(
                "{GENERIC_METADATA_PREFIX}fn:{}:{}:{}",
                project
                    .link_names
                    .get(&function.symbol)
                    .unwrap_or(&function.name),
                generic.0,
                function.generic_params.len()
            ));
        }
    }
    for definition in &project.executable.semantic.definitions {
        if let Some(generic) = definition.generic.filter(|_| {
            definition.public && project.symbol_owners.get(&definition.symbol) == Some(&module)
        }) {
            markers.push(format!(
                "{GENERIC_METADATA_PREFIX}type:{}:{}:{}",
                project
                    .link_names
                    .get(&definition.symbol)
                    .unwrap_or(&definition.name),
                generic.0,
                definition.generic_params.len()
            ));
        }
    }
    markers.sort();
    markers.dedup();
    markers
}

fn drop_metadata_markers(
    project: &dbyte_hir::CanonicalHirProject,
    module: dbyte_hir::ModuleId,
) -> Vec<String> {
    let mut markers = Vec::new();
    for definition in &project.executable.semantic.definitions {
        if !definition.public || project.symbol_owners.get(&definition.symbol) != Some(&module) {
            continue;
        }
        let Some(drop_function) = project
            .executable
            .semantic
            .functions
            .iter()
            .find(|function| {
                function.kind == dbyte_ast::FunctionKind::Drop
                    && function.drop_target == Some(definition.symbol)
            })
        else {
            continue;
        };
        let owner = project
            .link_names
            .get(&definition.symbol)
            .unwrap_or(&definition.name);
        let function = project
            .link_names
            .get(&drop_function.symbol)
            .unwrap_or(&drop_function.name);
        markers.push(format!(
            "{DROP_METADATA_PREFIX}{owner}:{function}:{}:{}",
            definition.symbol.0,
            definition.generic_params.len()
        ));
    }
    markers.sort();
    markers.dedup();
    markers
}

pub(crate) fn object_metadata_markers_for_module(
    project: &dbyte_hir::CanonicalHirProject,
    module: dbyte_hir::ModuleId,
) -> Vec<String> {
    let mut markers = generic_metadata_markers(project, module);
    markers.extend(drop_metadata_markers(project, module));
    markers.sort();
    markers.dedup();
    markers
}

fn validate_canonical_entry(
    project: &dbyte_hir::ConcreteHirProject,
    profile: dbyte_hir::Profile,
) -> NativeResult<()> {
    let concrete = &project.executable;
    let name = if profile == dbyte_hir::Profile::I686User {
        "user_main"
    } else {
        "kernel_main"
    };
    let Some(function) = concrete.functions.iter().find(|function| {
        project.symbol_owners.get(&function.key.symbol) == Some(&project.root)
            && project
                .link_names
                .get(&function.key.symbol)
                .is_some_and(|value| value == name)
    }) else {
        return Err(Diagnostic::without_span(
            crate::diagnostics::DiagnosticCategory::InvalidEntryPoint,
            format!("canonical project is missing `{name}`"),
        ));
    };
    let result_ok = matches!(
        concrete.semantic.type_of(function.result),
        dbyte_hir::CanonicalType::Integer(
            dbyte_hir::IntegerType::LegacyInt | dbyte_hir::IntegerType::I32
        )
    );
    let params_ok = if profile == dbyte_hir::Profile::I686User {
        function.parameters.is_empty()
    } else {
        function.parameters.is_empty()
            || matches!(
                function.parameters.as_slice(),
                [magic, info]
                    if matches!(
                        concrete.semantic.type_of(magic.ty),
                        dbyte_hir::CanonicalType::Integer(dbyte_hir::IntegerType::U32)
                    ) && matches!(
                        concrete.semantic.type_of(info.ty),
                        dbyte_hir::CanonicalType::Integer(dbyte_hir::IntegerType::Usize)
                    )
            )
    };
    if !result_ok || !params_ok || function.interrupt.is_some() {
        return Err(Diagnostic::new(
            crate::diagnostics::DiagnosticCategory::InvalidEntryPoint,
            format!("`{name}` has an invalid canonical entry signature"),
            function.span,
        ));
    }
    Ok(())
}

fn canonical_diagnostics(errors: Vec<dbyte_hir::Diagnostic>) -> Diagnostic {
    canonical_diagnostic(errors.into_iter().next().unwrap_or(dbyte_hir::Diagnostic {
        code: "E_HIR_ANALYSIS",
        message: "canonical analysis failed without a diagnostic".into(),
        span: dbyte_ast::Span::zero(),
    }))
}

fn canonical_project_diagnostics(
    errors: Vec<dbyte_hir::Diagnostic>,
    graph: &crate::modules::NativeModuleGraph,
) -> Diagnostic {
    canonical_project_diagnostic(
        errors.into_iter().next().unwrap_or(dbyte_hir::Diagnostic {
            code: "E_HIR_ANALYSIS",
            message: "canonical analysis failed without a diagnostic".into(),
            span: dbyte_ast::Span::zero(),
        }),
        graph,
    )
}

fn canonical_project_diagnostic(
    mut error: dbyte_hir::Diagnostic,
    graph: &crate::modules::NativeModuleGraph,
) -> Diagnostic {
    let module = error.span.line / 1_000_000;
    if module < graph.modules.len() && error.span.line >= 1_000_000 {
        error.span.line %= 1_000_000;
    }
    let diagnostic = canonical_diagnostic(error);
    graph
        .modules
        .get(module)
        .map_or(diagnostic.clone(), |source| {
            attach_path(diagnostic, &source.display_path)
        })
}

fn canonical_diagnostic(error: dbyte_hir::Diagnostic) -> Diagnostic {
    Diagnostic::new(
        crate::diagnostics::DiagnosticCategory::InvalidType,
        format!("{}: {}", error.code, error.message),
        error.span,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledModuleObject {
    pub module_id: usize,
    pub module_name: String,
    pub source_path: PathBuf,
    pub display_path: String,
    pub bytes: Vec<u8>,
    pub ir_dump: String,
    pub machine_dump: String,
    pub object_dump: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledProjectObjects {
    pub root_module: usize,
    pub modules: Vec<CompiledModuleObject>,
}

impl CompiledProjectObjects {
    pub fn root_object(&self) -> &CompiledModuleObject {
        &self.modules[self.root_module]
    }

    pub fn link_inputs(&self) -> Vec<LinkInput> {
        self.modules
            .iter()
            .map(|module| LinkInput {
                name: format!("{}.o", module.module_name),
                bytes: module.bytes.clone(),
            })
            .collect()
    }

    pub fn combined_ir_dump(&self) -> String {
        let mut dump = String::new();
        for module in &self.modules {
            dump.push_str(&module.ir_dump);
            if !dump.ends_with('\n') {
                dump.push('\n');
            }
        }
        dump
    }

    pub fn combined_machine_dump(&self) -> String {
        let mut dump = String::new();
        for module in &self.modules {
            dump.push_str(&module.machine_dump);
            if !dump.ends_with('\n') {
                dump.push('\n');
            }
            dump.push_str(&module.object_dump);
            if !dump.ends_with('\n') {
                dump.push('\n');
            }
        }
        dump
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemsBuildArtifact {
    pub objects: CompiledProjectObjects,
    pub linked: LinkArtifact,
    pub ir_dump: String,
    pub machine_dump: String,
}

pub fn compile_systems_objects(root: &Path) -> NativeResult<CompiledProjectObjects> {
    compile_canonical_project_objects(root, dbyte_hir::Profile::I686None)
}

pub fn compile_user_objects(root: &Path) -> NativeResult<CompiledProjectObjects> {
    compile_canonical_project_objects(root, dbyte_hir::Profile::I686User)
}

#[cfg(test)]
fn compile_legacy_systems_objects_with_mode(
    root: &Path,
    require_entry: bool,
    profile: SystemsProfile,
) -> NativeResult<CompiledProjectObjects> {
    let graph = load_native_module_graph(root)?;
    let project = match (profile, require_entry) {
        (SystemsProfile::Kernel, true) => analyze_systems_project(&graph)?,
        (SystemsProfile::Kernel, false) => analyze_systems_objects(&graph)?,
        (SystemsProfile::User, true) => analyze_user_project(&graph)?,
        (SystemsProfile::User, false) => analyze_user_objects(&graph)?,
    };
    if project.modules.len() != graph.modules.len() {
        return Err(Diagnostic::without_span(
            crate::diagnostics::DiagnosticCategory::InvalidModuleImport,
            "systems semantic output module count differs from module graph",
        ));
    }

    let mut modules = Vec::with_capacity(graph.modules.len());
    for source_module in &graph.modules {
        let ir_module = project.module(source_module.id).ok_or_else(|| {
            Diagnostic::without_span(
                crate::diagnostics::DiagnosticCategory::InvalidModuleImport,
                format!(
                    "systems semantic output is missing module '{}'",
                    source_module.name
                ),
            )
        })?;
        let lowered = lower_systems_module(ir_module)
            .map_err(|error| attach_path(error, &source_module.display_path))?;
        let object = write_relocatable(&lowered.image)
            .map_err(|error| attach_path(error, &source_module.display_path))?;
        modules.push(CompiledModuleObject {
            module_id: source_module.id,
            module_name: source_module.name.clone(),
            source_path: source_module.path.clone(),
            display_path: source_module.display_path.clone(),
            bytes: object.bytes,
            ir_dump: ir_module.dump(),
            machine_dump: lowered.machine_dump,
            object_dump: object.dump,
        });
    }
    Ok(CompiledProjectObjects {
        root_module: graph.root,
        modules,
    })
}

pub fn link_compiled_project(objects: &CompiledProjectObjects) -> NativeResult<LinkArtifact> {
    link_object_bytes(&objects.link_inputs())
}

pub fn compile_systems_project(root: &Path) -> NativeResult<SystemsBuildArtifact> {
    let objects =
        compile_canonical_project_objects_with_entry(root, dbyte_hir::Profile::I686None, true)?;
    let ir_dump = objects.combined_ir_dump();
    let mut machine_dump = objects.combined_machine_dump();
    let linked = link_compiled_project(&objects)?;
    machine_dump.push_str(&linked.dump);
    if !machine_dump.ends_with('\n') {
        machine_dump.push('\n');
    }
    Ok(SystemsBuildArtifact {
        objects,
        linked,
        ir_dump,
        machine_dump,
    })
}

pub fn compile_user_project(root: &Path) -> NativeResult<SystemsBuildArtifact> {
    let objects =
        compile_canonical_project_objects_with_entry(root, dbyte_hir::Profile::I686User, true)?;
    let ir_dump = objects.combined_ir_dump();
    let mut machine_dump = objects.combined_machine_dump();
    let linked = link_user_object_bytes(&objects.link_inputs())?;
    machine_dump.push_str(&linked.dump);
    if !machine_dump.ends_with('\n') {
        machine_dump.push('\n');
    }
    Ok(SystemsBuildArtifact {
        objects,
        linked,
        ir_dump,
        machine_dump,
    })
}

fn attach_path(error: Diagnostic, path: &str) -> Diagnostic {
    if error.path.is_some() {
        error
    } else {
        error.with_path(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    #[test]
    fn staged_canonical_pipeline_emits_a_linkable_real_object() {
        let source = "fn id[T](value: T) -> T:\n    return value\n\npub fn kernel_main() -> int:\n    native.io_out8(1016 as u16, id[u8](65 as u8))\n    return 0\n";
        let tokens = Lexer::new(source).tokenize().unwrap();
        let program = Parser::new(tokens).parse_program().unwrap();
        let object =
            compile_canonical_module_object("kernel", &program, dbyte_hir::Profile::I686None)
                .unwrap();
        assert!(object.bytes.starts_with(b"\x7fELF"));
        assert!(object.ir_dump.contains("__dbyte_mono_id"));
        let linked = link_object_bytes(&[LinkInput {
            name: "kernel.o".into(),
            bytes: object.bytes,
        }])
        .unwrap();
        assert!(linked.elf.starts_with(b"\x7fELF"));
    }

    #[test]
    fn canonical_project_pipeline_resolves_and_links_real_module_graph() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_multimodule/kernel.dby");
        let objects =
            compile_canonical_project_objects(&root, dbyte_hir::Profile::I686None).unwrap();
        assert_eq!(objects.modules.len(), 5);
        let linked = link_compiled_project(&objects).unwrap();
        assert!(linked.elf.starts_with(b"\x7fELF"));
        assert!(objects.combined_ir_dump().contains("call add 2 -> u32"));
        let legacy =
            compile_legacy_systems_objects_with_mode(&root, false, SystemsProfile::Kernel).unwrap();
        for (canonical, legacy) in objects.modules.iter().zip(&legacy.modules) {
            assert_eq!(
                canonical.bytes, legacy.bytes,
                "canonical object drifted for {}",
                canonical.module_name
            );
        }
    }

    #[test]
    fn canonical_project_preserves_frozen_interrupt_core_objects() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_core/kernel.dby");
        let canonical =
            compile_canonical_project_objects(&root, dbyte_hir::Profile::I686None).unwrap();
        assert_eq!(canonical.modules.len(), 8);
        assert!(canonical
            .modules
            .iter()
            .any(|module| module.object_dump.contains(".dbyte.interrupts")));
        let linked = link_compiled_project(&canonical).unwrap();
        assert!(linked.elf.starts_with(b"\x7fELF"));
        let idt = canonical
            .modules
            .iter()
            .find(|module| module.module_name == "idt")
            .expect("IDT object");
        assert_eq!(
            crate::dbup::sha256(&idt.bytes),
            [
                0x63, 0x58, 0x9f, 0x4b, 0x77, 0x33, 0x54, 0x1e, 0x24, 0xa4, 0xae, 0x96, 0x2d, 0x44,
                0xc1, 0x4a, 0x2a, 0x69, 0x11, 0xd3, 0xdd, 0x8a, 0x75, 0x18, 0x46, 0x62, 0x2d, 0x48,
                0xef, 0x59, 0x83, 0x2b,
            ],
            "canonical IDT object must retain the frozen v11.2 bytes"
        );
    }

    #[test]
    fn canonical_project_accepts_every_frozen_kernel_root() {
        for directory in [
            "native_memory_core",
            "native_runtime_core",
            "native_scheduler_core",
            "native_blocking_core",
            "native_irq_io_core",
            "native_block_device_core",
            "native_filesystem_core",
            "native_filesystem_runtime",
            "native_user_process_core",
        ] {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples")
                .join(directory)
                .join("kernel.dby");
            let objects = compile_canonical_project_objects(&root, dbyte_hir::Profile::I686None)
                .unwrap_or_else(|error| panic!("canonical {directory} failed: {error}"));
            let linked = link_compiled_project(&objects)
                .unwrap_or_else(|error| panic!("canonical {directory} link failed: {error}"));
            assert!(linked.elf.starts_with(b"\x7fELF"), "{directory}");
        }
    }

    #[test]
    fn canonical_i686_project_links_v123_collection_helpers_across_modules() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_v12_3_collections/none.dby");
        let objects = compile_canonical_project_objects(&root, dbyte_hir::Profile::I686None)
            .unwrap_or_else(|error| panic!("collection project failed: {error}"));
        let dump = objects.combined_ir_dump();
        for helper in [
            "__dbyte_buffer_usize_max",
            "__dbyte_deque_usize_max",
            "__dbyte_hash_map_usize_max",
        ] {
            assert!(
                dump.contains(&format!("fn {helper}")),
                "missing {helper}\n{dump}"
            );
        }
        link_compiled_project(&objects)
            .unwrap_or_else(|error| panic!("collection project link failed: {error}"));
    }

    #[test]
    fn real_project_emits_one_object_per_module_and_links_same_bytes_twice() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_multimodule/kernel.dby");
        let objects = compile_systems_objects(&root).unwrap();
        assert_eq!(objects.modules.len(), 5);
        assert!(objects
            .modules
            .iter()
            .all(|module| module.bytes.starts_with(b"\x7fELF")));
        let first = link_compiled_project(&objects).unwrap();
        let second = link_compiled_project(&objects).unwrap();
        assert_eq!(first.elf, second.elf);
        assert_eq!(first.bin, second.bin);
    }

    #[test]
    fn object_and_integrated_paths_share_exact_object_bytes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_multimodule/kernel.dby");
        let direct = compile_systems_objects(&root).unwrap();
        let integrated = compile_systems_project(&root).unwrap();
        let direct_bytes: Vec<&[u8]> = direct
            .modules
            .iter()
            .map(|module| module.bytes.as_slice())
            .collect();
        let integrated_bytes: Vec<&[u8]> = integrated
            .objects
            .modules
            .iter()
            .map(|module| module.bytes.as_slice())
            .collect();
        assert_eq!(direct_bytes, integrated_bytes);
    }

    #[test]
    fn standalone_object_does_not_require_entry_but_integrated_project_does() {
        let module = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_multimodule/arch_x86.dby");
        let objects = compile_systems_objects(&module).unwrap();
        assert_eq!(objects.modules.len(), 1);
        assert!(objects.root_object().bytes.starts_with(b"\x7fELF"));

        let error = compile_systems_project(&module).unwrap_err();
        assert_eq!(
            error.category,
            crate::diagnostics::DiagnosticCategory::InvalidEntryPoint
        );
        assert!(error.message.contains("kernel_main"));
    }
}
