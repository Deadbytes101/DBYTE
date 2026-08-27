//! Canonical, target-parameterized semantic model for DByte.
//!
//! This crate deliberately contains no interpreter values, VM opcodes, x86
//! registers, relocation kinds, or host-sized layout decisions.

use dbyte_ast::{
    BinOp, CallConvention, CaptureMode, Expr, FunctionKind, MatchMode, Pattern, Program, Repr,
    Span, Stmt, TypeAnnotation, UnaryOp,
};
use std::collections::{BTreeMap, BTreeSet};

mod executable;
pub use executable::*;
mod project;
mod source;
pub use source::{load_source_project, load_source_project_from_program};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);
    };
}

id_type!(TypeId);
id_type!(SymbolId);
id_type!(ModuleId);
id_type!(GenericDefId);
id_type!(VariantId);
id_type!(FieldId);
id_type!(ClosureId);
id_type!(AllocatorOriginId);
id_type!(BorrowOriginId);

#[derive(Debug, Clone, PartialEq)]
pub enum AllocatorOrigin {
    StaticSystem,
    Parameter { function: SymbolId, index: u32 },
    Lexical { function: SymbolId, span: Span },
}

#[derive(Debug, Clone, PartialEq)]
pub enum BorrowOrigin {
    Static,
    Parameter { function: SymbolId, index: u32 },
    Lexical { function: SymbolId, span: Span },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Hosted,
    Vm,
    I686None,
    I686User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetDataLayout {
    pub pointer_size: u32,
    pub pointer_align: u32,
    pub usize_size: u32,
    pub usize_align: u32,
}

impl TargetDataLayout {
    pub const I686: Self = Self {
        pointer_size: 4,
        pointer_align: 4,
        usize_size: 4,
        usize_align: 4,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub size: u32,
    pub align: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntegerType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    Usize,
    LegacyInt,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CanonicalType {
    Unit,
    Integer(IntegerType),
    Float,
    Bool,
    LegacyStr,
    LegacyBytes,
    LegacyBuffer,
    LegacyList(TypeId),
    Pointer {
        mutable: bool,
        pointee: TypeId,
    },
    Borrow {
        mutable: bool,
        pointee: TypeId,
    },
    Slice {
        mutable: bool,
        element: TypeId,
    },
    StringView,
    Array {
        element: TypeId,
        length: u32,
    },
    Struct {
        symbol: SymbolId,
        args: Vec<TypeId>,
    },
    Enum {
        symbol: SymbolId,
        args: Vec<TypeId>,
    },
    Function {
        convention: CallConvention,
        params: Vec<TypeId>,
        result: TypeId,
    },
    ClosureShape {
        params: Vec<TypeId>,
        result: TypeId,
    },
    GenericParam {
        owner: GenericDefId,
        index: u32,
    },
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} at {}: {}",
            self.code, self.span, self.message
        )
    }
}

impl std::error::Error for Diagnostic {}

#[derive(Debug, Clone)]
pub struct FieldDef {
    pub id: FieldId,
    pub name: String,
    pub annotation: TypeAnnotation,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct VariantDef {
    pub id: VariantId,
    pub name: String,
    pub payload: Vec<TypeAnnotation>,
    pub discriminant: u32,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypeDefKind {
    Struct { repr: Repr, fields: Vec<FieldDef> },
    Enum { variants: Vec<VariantDef> },
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub symbol: SymbolId,
    pub generic: Option<GenericDefId>,
    pub name: String,
    pub public: bool,
    pub generic_params: Vec<String>,
    pub kind: TypeDefKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub symbol: SymbolId,
    pub generic: Option<GenericDefId>,
    pub name: String,
    pub public: bool,
    pub kind: dbyte_ast::FunctionKind,
    pub drop_target: Option<SymbolId>,
    pub convention: CallConvention,
    pub interrupt: Option<dbyte_ast::InterruptClass>,
    pub declaration_only: bool,
    pub generic_params: Vec<String>,
    pub params: Vec<TypeAnnotation>,
    pub result: TypeAnnotation,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct GlobalDef {
    pub symbol: SymbolId,
    pub name: String,
    pub public: bool,
    pub mutable: bool,
    pub constant: bool,
    pub annotation: TypeAnnotation,
    pub initializer: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpecializationKey {
    pub definition: GenericDefId,
    pub arguments: Vec<TypeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BuiltinCapability {
    Numeric,
    Equality,
    Ordering,
    Copy,
}

/// A capability resolved by canonical analysis. Backends consume this identity
/// and never recover an operation by matching source-level names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HirIntrinsic {
    VolatileLoad8,
    VolatileStore8,
    VolatileLoad16,
    VolatileStore16,
    VolatileLoad32,
    VolatileStore32,
    IoOut8,
    IoIn8,
    IoOut16,
    IoIn16,
    CpuHlt,
    CpuCli,
    CpuSti,
    CpuInt3,
    CpuReadEflags,
    CpuReadCr0,
    CpuWriteCr0,
    CpuReadCr2,
    CpuReadCr3,
    CpuWriteCr3,
    CpuInvlpg,
    CpuLgdt,
    CpuLidt,
    CpuLtr,
    CpuReloadSegments,
    TaskBlockTrap,
    InterruptResume,
    KernelImageStart,
    KernelImageEnd,
    BootstrapStackStart,
    BootstrapStackEnd,
    TaskTrampolineAddress,
    UserSyscall0,
    UserSyscall1,
    UserSyscall2,
    UserSyscall3,
    RuntimeSliceFromRaw,
    RuntimeMutSliceFromRaw,
    RuntimeStringViewFromRaw,
    RuntimeMemoryAlloc,
    RuntimeMemoryResize,
    RuntimeMemoryFree,
    RuntimeMemoryRead8,
    RuntimeMemoryWrite8,
}

impl HirIntrinsic {
    pub fn resolve(namespace: &str, name: &str) -> Option<Self> {
        let value = match (namespace, name) {
            ("native", "volatile_load8") => Self::VolatileLoad8,
            ("native", "volatile_store8") => Self::VolatileStore8,
            ("native", "volatile_load16") => Self::VolatileLoad16,
            ("native", "volatile_store16") => Self::VolatileStore16,
            ("native", "volatile_load32") => Self::VolatileLoad32,
            ("native", "volatile_store32") => Self::VolatileStore32,
            ("native", "io_out8") => Self::IoOut8,
            ("native", "io_in8") => Self::IoIn8,
            ("native", "io_out16") => Self::IoOut16,
            ("native", "io_in16") => Self::IoIn16,
            ("native", "cpu_hlt") => Self::CpuHlt,
            ("native", "cpu_cli") => Self::CpuCli,
            ("native", "cpu_sti") => Self::CpuSti,
            ("native", "cpu_int3") => Self::CpuInt3,
            ("native", "cpu_read_eflags") => Self::CpuReadEflags,
            ("native", "cpu_read_cr0") => Self::CpuReadCr0,
            ("native", "cpu_write_cr0") => Self::CpuWriteCr0,
            ("native", "cpu_read_cr2") => Self::CpuReadCr2,
            ("native", "cpu_read_cr3") => Self::CpuReadCr3,
            ("native", "cpu_write_cr3") => Self::CpuWriteCr3,
            ("native", "cpu_invlpg") => Self::CpuInvlpg,
            ("native", "cpu_lgdt") => Self::CpuLgdt,
            ("native", "cpu_lidt") => Self::CpuLidt,
            ("native", "cpu_ltr") => Self::CpuLtr,
            ("native", "cpu_reload_segments") => Self::CpuReloadSegments,
            ("native", "task_block_trap") => Self::TaskBlockTrap,
            ("native", "interrupt_resume") => Self::InterruptResume,
            ("native", "kernel_image_start") => Self::KernelImageStart,
            ("native", "kernel_image_end") => Self::KernelImageEnd,
            ("native", "bootstrap_stack_start") => Self::BootstrapStackStart,
            ("native", "bootstrap_stack_end") => Self::BootstrapStackEnd,
            ("native", "task_trampoline_address") => Self::TaskTrampolineAddress,
            ("user", "syscall0") => Self::UserSyscall0,
            ("user", "syscall1") => Self::UserSyscall1,
            ("user", "syscall2") => Self::UserSyscall2,
            ("user", "syscall3") => Self::UserSyscall3,
            ("__dbyte_runtime", "slice_from_raw") => Self::RuntimeSliceFromRaw,
            ("__dbyte_runtime", "mut_slice_from_raw") => Self::RuntimeMutSliceFromRaw,
            ("__dbyte_runtime", "string_view_from_raw") => Self::RuntimeStringViewFromRaw,
            ("__dbyte_runtime", "memory_alloc") => Self::RuntimeMemoryAlloc,
            ("__dbyte_runtime", "memory_resize") => Self::RuntimeMemoryResize,
            ("__dbyte_runtime", "memory_free") => Self::RuntimeMemoryFree,
            ("__dbyte_runtime", "memory_read8") => Self::RuntimeMemoryRead8,
            ("__dbyte_runtime", "memory_write8") => Self::RuntimeMemoryWrite8,
            _ => return None,
        };
        Some(value)
    }

    pub const fn is_user(self) -> bool {
        matches!(
            self,
            Self::UserSyscall0 | Self::UserSyscall1 | Self::UserSyscall2 | Self::UserSyscall3
        )
    }

    pub const fn is_runtime_view(self) -> bool {
        matches!(
            self,
            Self::RuntimeSliceFromRaw
                | Self::RuntimeMutSliceFromRaw
                | Self::RuntimeStringViewFromRaw
        )
    }

    pub const fn is_runtime_host_memory(self) -> bool {
        matches!(
            self,
            Self::RuntimeMemoryAlloc
                | Self::RuntimeMemoryResize
                | Self::RuntimeMemoryFree
                | Self::RuntimeMemoryRead8
                | Self::RuntimeMemoryWrite8
        )
    }
}

#[derive(Debug, Clone)]
pub struct EnumLayout {
    pub layout: Layout,
    pub payload_offset: u32,
    pub payload: Layout,
}

pub type EnumVariantFieldLayout = (u32, u32, Vec<(TypeId, u32)>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueCategory {
    LValue,
    RValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipAction {
    Copy,
    Move,
    Borrow,
    BorrowMut,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirExpression {
    pub span: Span,
    pub ty: TypeId,
    pub category: ValueCategory,
    pub ownership: OwnershipAction,
    pub allocator_origin: Option<AllocatorOriginId>,
    pub borrow_origin: Option<BorrowOriginId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedAddress {
    Interrupt {
        vector: u8,
        handler: SymbolId,
        class: dbyte_ast::InterruptClass,
    },
    Task {
        function: SymbolId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupExit {
    Fallthrough,
    Return,
    Break,
    Continue,
    PropagateError,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CleanupEdge {
    pub defer_span: Span,
    pub exit_span: Span,
    pub exit: CleanupExit,
    pub lexical_depth: u32,
}

#[derive(Debug, Clone)]
pub struct HirProgram {
    pub module: ModuleId,
    pub profile: Profile,
    pub layout: TargetDataLayout,
    pub types: Vec<CanonicalType>,
    pub definitions: Vec<TypeDef>,
    pub functions: Vec<FunctionDef>,
    pub globals: Vec<GlobalDef>,
    pub specializations: BTreeSet<SpecializationKey>,
    pub generic_capabilities: BTreeMap<(GenericDefId, u32), BTreeSet<BuiltinCapability>>,
    pub closure_count: u32,
    pub expressions: Vec<HirExpression>,
    pub allocator_origins: Vec<AllocatorOrigin>,
    pub borrow_origins: Vec<BorrowOrigin>,
    pub function_allocator_origins: BTreeMap<SymbolId, AllocatorOriginId>,
    pub function_borrow_origins: BTreeMap<SymbolId, BorrowOriginId>,
    pub cleanup_edges: Vec<CleanupEdge>,
    pub resolved_addresses: BTreeMap<(usize, usize), ResolvedAddress>,
    pub interface_fingerprint: u64,
}

impl HirProgram {
    pub fn type_of(&self, id: TypeId) -> &CanonicalType {
        &self.types[id.0 as usize]
    }

    pub fn dump(&self) -> String {
        let mut out = String::new();
        out.push_str("HIR v12\n");
        out.push_str(&format!(
            "module m{} profile {:?}\n",
            self.module.0, self.profile
        ));
        out.push_str(&format!(
            "layout pointer={}/{} usize={}/{}\n",
            self.layout.pointer_size,
            self.layout.pointer_align,
            self.layout.usize_size,
            self.layout.usize_align
        ));
        for (index, ty) in self.types.iter().enumerate() {
            out.push_str(&format!("type t{index} = {ty:?}\n"));
        }
        for definition in &self.definitions {
            out.push_str(&format!(
                "def s{} {} generic={:?} public={}\n",
                definition.symbol.0, definition.name, definition.generic, definition.public
            ));
        }
        for function in &self.functions {
            out.push_str(&format!(
                "fn s{} {} convention={} generic={:?}\n",
                function.symbol.0, function.name, function.convention, function.generic
            ));
        }
        for global in &self.globals {
            out.push_str(&format!(
                "global s{} {} public={} mutable={} constant={}\n",
                global.symbol.0, global.name, global.public, global.mutable, global.constant
            ));
        }
        for specialization in &self.specializations {
            out.push_str(&format!(
                "specialize g{} {:?}\n",
                specialization.definition.0, specialization.arguments
            ));
        }
        for ((generic, parameter), capabilities) in &self.generic_capabilities {
            out.push_str(&format!(
                "capabilities g{}.{} {:?}\n",
                generic.0, parameter, capabilities
            ));
        }
        out.push_str(&format!("closures {}\n", self.closure_count));
        for expression in &self.expressions {
            out.push_str(&format!(
                "expr {} t{} {:?} {:?} alloc={:?} borrow={:?}\n",
                expression.span,
                expression.ty.0,
                expression.category,
                expression.ownership,
                expression.allocator_origin,
                expression.borrow_origin
            ));
        }
        for (index, origin) in self.allocator_origins.iter().enumerate() {
            out.push_str(&format!("allocator-origin a{index} = {origin:?}\n"));
        }
        for (index, origin) in self.borrow_origins.iter().enumerate() {
            out.push_str(&format!("borrow-origin b{index} = {origin:?}\n"));
        }
        for (function, origin) in &self.function_allocator_origins {
            out.push_str(&format!(
                "fn-origin s{} allocator=a{}\n",
                function.0, origin.0
            ));
        }
        for (function, origin) in &self.function_borrow_origins {
            out.push_str(&format!("fn-origin s{} borrow=b{}\n", function.0, origin.0));
        }
        for edge in &self.cleanup_edges {
            out.push_str(&format!(
                "cleanup {} -> {} {:?} depth={}\n",
                edge.defer_span, edge.exit_span, edge.exit, edge.lexical_depth
            ));
        }
        out.push_str(&format!("interface {:016x}\n", self.interface_fingerprint));
        out
    }
}

#[derive(Default)]
struct Interner {
    values: Vec<CanonicalType>,
    ids: BTreeMap<CanonicalType, TypeId>,
}

impl Interner {
    fn intern(&mut self, value: CanonicalType) -> TypeId {
        if let Some(id) = self.ids.get(&value) {
            return *id;
        }
        let id = TypeId(self.values.len() as u32);
        self.values.push(value.clone());
        self.ids.insert(value, id);
        id
    }
}

pub fn analyze(
    program: &Program,
    profile: Profile,
    layout: TargetDataLayout,
) -> Result<HirProgram, Vec<Diagnostic>> {
    Analyzer::new(profile, layout).analyze(program)
}

struct Analyzer {
    profile: Profile,
    layout: TargetDataLayout,
    types: Interner,
    names: BTreeMap<String, SymbolId>,
    definitions: Vec<TypeDef>,
    functions: Vec<FunctionDef>,
    globals: Vec<GlobalDef>,
    specializations: BTreeSet<SpecializationKey>,
    generic_capabilities: BTreeMap<(GenericDefId, u32), BTreeSet<BuiltinCapability>>,
    closure_count: u32,
    next_symbol: u32,
    next_generic: u32,
    next_field: u32,
    next_variant: u32,
    current_return: Option<TypeId>,
    current_interrupt: Option<dbyte_ast::InterruptClass>,
    expressions: Vec<HirExpression>,
    allocator_origins: Vec<AllocatorOrigin>,
    borrow_origins: Vec<BorrowOrigin>,
    function_allocator_origins: BTreeMap<SymbolId, AllocatorOriginId>,
    function_borrow_origins: BTreeMap<SymbolId, BorrowOriginId>,
    allocator_bindings: BTreeMap<String, AllocatorOriginId>,
    borrow_bindings: BTreeMap<String, BorrowOriginId>,
    current_function: Option<SymbolId>,
    current_function_source_name: Option<String>,
    current_parameters: BTreeMap<String, u32>,
    cleanup_edges: Vec<CleanupEdge>,
    resolved_addresses: BTreeMap<(usize, usize), ResolvedAddress>,
    active_defers: Vec<(Span, u32)>,
    lexical_depth: u32,
    borrowed_parameters: BTreeSet<String>,
    current_drop_parameter: Option<String>,
    current_generic: Option<GenericDefId>,
    host_modules: BTreeMap<String, String>,
    diagnostics: Vec<Diagnostic>,
}

impl Analyzer {
    fn new(profile: Profile, layout: TargetDataLayout) -> Self {
        let mut types = Interner::default();
        types.intern(CanonicalType::Unit);
        let mut analyzer = Self {
            profile,
            layout,
            types,
            names: BTreeMap::new(),
            definitions: Vec::new(),
            functions: Vec::new(),
            globals: Vec::new(),
            specializations: BTreeSet::new(),
            generic_capabilities: BTreeMap::new(),
            closure_count: 0,
            next_symbol: 0,
            next_generic: 0,
            next_field: 0,
            next_variant: 0,
            current_return: None,
            current_interrupt: None,
            expressions: Vec::new(),
            allocator_origins: Vec::new(),
            borrow_origins: Vec::new(),
            function_allocator_origins: BTreeMap::new(),
            function_borrow_origins: BTreeMap::new(),
            allocator_bindings: BTreeMap::new(),
            borrow_bindings: BTreeMap::new(),
            current_function: None,
            current_function_source_name: None,
            current_parameters: BTreeMap::new(),
            cleanup_edges: Vec::new(),
            resolved_addresses: BTreeMap::new(),
            active_defers: Vec::new(),
            lexical_depth: 0,
            borrowed_parameters: BTreeSet::new(),
            current_drop_parameter: None,
            current_generic: None,
            host_modules: BTreeMap::new(),
            diagnostics: Vec::new(),
        };
        analyzer.install_prelude_enum("Option", &["T"], &[("None", &[]), ("Some", &["T"])]);
        analyzer.install_prelude_enum("Result", &["T", "E"], &[("Ok", &["T"]), ("Err", &["E"])]);
        analyzer
    }

    fn install_prelude_enum(
        &mut self,
        name: &str,
        generic_names: &[&str],
        variants: &[(&str, &[&str])],
    ) {
        let symbol = SymbolId(self.next_symbol);
        self.next_symbol += 1;
        self.names.insert(name.to_owned(), symbol);
        let generic = GenericDefId(self.next_generic);
        self.next_generic += 1;
        let variants = variants
            .iter()
            .enumerate()
            .map(|(index, (variant_name, payload))| {
                let id = VariantId(self.next_variant);
                self.next_variant += 1;
                VariantDef {
                    id,
                    name: (*variant_name).to_owned(),
                    payload: payload
                        .iter()
                        .map(|name| TypeAnnotation::Named((*name).to_owned()))
                        .collect(),
                    discriminant: index as u32,
                    span: Span::zero(),
                }
            })
            .collect();
        self.definitions.push(TypeDef {
            symbol,
            generic: Some(generic),
            name: name.to_owned(),
            public: true,
            generic_params: generic_names
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            kind: TypeDefKind::Enum { variants },
            span: Span::zero(),
        });
    }

    fn diagnostic(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diagnostics.push(Diagnostic {
            code,
            message: message.into(),
            span,
        });
    }

    fn symbol(&mut self, name: &str, span: Span) -> Option<SymbolId> {
        if self.names.contains_key(name) {
            self.diagnostic(
                "E_DUPLICATE_SYMBOL",
                format!("duplicate module symbol `{name}`"),
                span,
            );
            return None;
        }
        let id = SymbolId(self.next_symbol);
        self.next_symbol += 1;
        self.names.insert(name.to_owned(), id);
        Some(id)
    }

    fn generic(&mut self, params: &[String]) -> Option<GenericDefId> {
        if params.is_empty() {
            None
        } else {
            let id = GenericDefId(self.next_generic);
            self.next_generic += 1;
            Some(id)
        }
    }

    fn analyze(mut self, program: &Program) -> Result<HirProgram, Vec<Diagnostic>> {
        self.collect(program);
        self.validate_declarations();
        self.validate_bodies(program);
        self.validate_specializations();
        self.validate_profile(program);
        if !self.diagnostics.is_empty() {
            self.diagnostics.sort_by_key(|diagnostic| {
                (diagnostic.span.line, diagnostic.span.col, diagnostic.code)
            });
            return Err(self.diagnostics);
        }
        let interface_fingerprint = self.interface_fingerprint();
        Ok(HirProgram {
            module: ModuleId(0),
            profile: self.profile,
            layout: self.layout,
            types: self.types.values,
            definitions: self.definitions,
            functions: self.functions,
            globals: self.globals,
            specializations: self.specializations,
            generic_capabilities: self.generic_capabilities,
            closure_count: self.closure_count,
            expressions: self.expressions,
            allocator_origins: self.allocator_origins,
            borrow_origins: self.borrow_origins,
            function_allocator_origins: self.function_allocator_origins,
            function_borrow_origins: self.function_borrow_origins,
            cleanup_edges: self.cleanup_edges,
            resolved_addresses: self.resolved_addresses,
            interface_fingerprint,
        })
    }

    /// Profile checking is deliberately a final semantic pass.  Resolution,
    /// type identity, ownership, and generic specialization therefore do not
    /// change depending on the selected backend; a profile can only remove a
    /// capability from an otherwise canonical program.
    fn validate_profile(&mut self, program: &Program) {
        for statement in &program.stmts {
            self.validate_profile_statement(statement);
        }
    }

    fn validate_profile_statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::FnDef {
                interrupt,
                call_convention,
                body,
                span,
                ..
            } => {
                if interrupt.is_some()
                    && matches!(
                        self.profile,
                        Profile::Hosted | Profile::Vm | Profile::I686User
                    )
                {
                    self.diagnostic(
                        "E_PROFILE_CAPABILITY",
                        format!(
                            "{:?} profile does not admit interrupt functions",
                            self.profile
                        ),
                        *span,
                    );
                }
                if *call_convention == CallConvention::C
                    && !matches!(self.profile, Profile::I686None)
                {
                    self.diagnostic(
                        "E_PROFILE_CAPABILITY",
                        format!("{:?} profile does not admit extern C linkage", self.profile),
                        *span,
                    );
                }
                for child in body {
                    self.validate_profile_statement(child);
                }
            }
            Stmt::Let { value, .. }
            | Stmt::Assign { value, .. }
            | Stmt::Const { value, .. }
            | Stmt::Expr(value) => self.validate_profile_expression(value),
            Stmt::Static { value, .. } => {
                if let Some(value) = value {
                    self.validate_profile_expression(value);
                }
            }
            Stmt::Store { target, value, .. } => {
                self.validate_profile_expression(target);
                self.validate_profile_expression(value);
            }
            Stmt::Return { value, .. } => {
                if let Some(value) = value {
                    self.validate_profile_expression(value);
                }
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                self.validate_profile_expression(cond);
                for child in then_body {
                    self.validate_profile_statement(child);
                }
                if let Some(body) = else_body {
                    for child in body {
                        self.validate_profile_statement(child);
                    }
                }
            }
            Stmt::While { cond, body, .. } => {
                self.validate_profile_expression(cond);
                for child in body {
                    self.validate_profile_statement(child);
                }
            }
            Stmt::For { iterable, body, .. } => {
                self.validate_profile_expression(iterable);
                for child in body {
                    self.validate_profile_statement(child);
                }
            }
            Stmt::Match { value, arms, .. } => {
                self.validate_profile_expression(value);
                for arm in arms {
                    for child in &arm.body {
                        self.validate_profile_statement(child);
                    }
                }
            }
            Stmt::Defer { body, .. } => {
                for child in body {
                    self.validate_profile_statement(child);
                }
            }
            Stmt::Import { .. }
            | Stmt::StructDef { .. }
            | Stmt::EnumDef { .. }
            | Stmt::Break { .. }
            | Stmt::Continue { .. } => {}
        }
    }

    fn validate_profile_expression(&mut self, expression: &Expr) {
        if self.profile == Profile::I686User {
            match expression {
                Expr::Call { name, span, .. }
                    if name == "interrupt_address"
                        || name == "task_address"
                        || name.starts_with("native.") =>
                {
                    self.diagnostic(
                        "E_PROFILE_CAPABILITY",
                        format!("i686 user profile does not admit `{name}`"),
                        *span,
                    );
                }
                Expr::MemberCall {
                    object,
                    property,
                    span,
                    ..
                } if matches!(object.as_ref(), Expr::Ident(name, _) if name == "native") => {
                    self.diagnostic(
                        "E_PROFILE_CAPABILITY",
                        format!("i686 user profile does not admit `native.{property}`"),
                        *span,
                    );
                }
                Expr::Cast { expr, ty, span }
                    if matches!(
                        ty,
                        TypeAnnotation::Pointer(_) | TypeAnnotation::ConstPointer(_)
                    ) && constant_integer_expression(expr)
                        .is_some_and(|value| value >= 0xc000_0000) =>
                {
                    self.diagnostic(
                        "E_PROFILE_CAPABILITY",
                        "i686 user profile rejects constant pointers into the kernel range",
                        *span,
                    );
                }
                _ => {}
            }
        }

        match expression {
            Expr::Binary { left, right, .. } => {
                self.validate_profile_expression(left);
                self.validate_profile_expression(right);
            }
            Expr::Unary { expr, .. }
            | Expr::Borrow { expr, .. }
            | Expr::Try { expr, .. }
            | Expr::Drop { expr, .. }
            | Expr::Cast { expr, .. } => self.validate_profile_expression(expr),
            Expr::List(values, _) => {
                for value in values {
                    self.validate_profile_expression(value);
                }
            }
            Expr::Call { args, .. } | Expr::MemberCall { args, .. } => {
                for argument in args {
                    self.validate_profile_expression(argument);
                }
                if let Expr::MemberCall { object, .. } = expression {
                    self.validate_profile_expression(object);
                }
            }
            Expr::Index { target, index, .. } => {
                self.validate_profile_expression(target);
                self.validate_profile_expression(index);
            }
            Expr::Member { object, .. } | Expr::GenericApply { target: object, .. } => {
                self.validate_profile_expression(object);
            }
            Expr::Invoke {
                callee,
                args,
                named_args,
                ..
            } => {
                self.validate_profile_expression(callee);
                for argument in args {
                    self.validate_profile_expression(argument);
                }
                for argument in named_args {
                    self.validate_profile_expression(&argument.value);
                }
            }
            Expr::Closure { body, .. } => {
                for statement in body {
                    self.validate_profile_statement(statement);
                }
            }
            Expr::UnitLit(..)
            | Expr::IntLit(..)
            | Expr::FloatLit(..)
            | Expr::BoolLit(..)
            | Expr::StrLit(..)
            | Expr::BytesLit(..)
            | Expr::FStr(..)
            | Expr::NullLit(..)
            | Expr::Ident(..) => {}
        }
    }

    fn collect(&mut self, program: &Program) {
        for statement in &program.stmts {
            match statement {
                Stmt::Import { path, alias, span } if path.starts_with("std.") => {
                    if dbyte_module::stdlib_exports(path).is_none() {
                        self.diagnostic(
                            "E_HOST_MODULE",
                            format!("unknown standard module `{path}`"),
                            *span,
                        );
                    } else if self
                        .host_modules
                        .insert(alias.clone(), path.clone())
                        .is_some()
                    {
                        self.diagnostic(
                            "E_DUPLICATE_IMPORT_ALIAS",
                            format!("duplicate import alias `{alias}`"),
                            *span,
                        );
                    }
                }
                Stmt::StructDef {
                    is_pub,
                    is_packed,
                    repr,
                    name,
                    generic_params,
                    fields,
                    span,
                } => {
                    let Some(symbol) = self.symbol(name, *span) else {
                        continue;
                    };
                    let generic = self.generic(generic_params);
                    let repr = if *is_packed && *repr == Repr::DByte {
                        Repr::CPacked
                    } else {
                        *repr
                    };
                    let fields = fields
                        .iter()
                        .map(|field| {
                            let id = FieldId(self.next_field);
                            self.next_field += 1;
                            FieldDef {
                                id,
                                name: field.name.clone(),
                                annotation: field.ty.clone(),
                                span: field.span,
                            }
                        })
                        .collect();
                    self.definitions.push(TypeDef {
                        symbol,
                        generic,
                        name: name.clone(),
                        public: *is_pub,
                        generic_params: generic_params.clone(),
                        kind: TypeDefKind::Struct { repr, fields },
                        span: *span,
                    });
                }
                Stmt::EnumDef {
                    is_pub,
                    name,
                    generic_params,
                    variants,
                    span,
                } => {
                    let Some(symbol) = self.symbol(name, *span) else {
                        continue;
                    };
                    let generic = self.generic(generic_params);
                    let variants = variants
                        .iter()
                        .enumerate()
                        .map(|(index, variant)| {
                            let id = VariantId(self.next_variant);
                            self.next_variant += 1;
                            VariantDef {
                                id,
                                name: variant.name.clone(),
                                payload: variant.payload.clone(),
                                discriminant: index as u32,
                                span: variant.span,
                            }
                        })
                        .collect();
                    self.definitions.push(TypeDef {
                        symbol,
                        generic,
                        name: name.clone(),
                        public: *is_pub,
                        generic_params: generic_params.clone(),
                        kind: TypeDefKind::Enum { variants },
                        span: *span,
                    });
                }
                Stmt::FnDef {
                    is_pub,
                    kind,
                    interrupt,
                    call_convention,
                    name,
                    generic_params,
                    params,
                    ret_ty,
                    body,
                    span,
                } => {
                    let Some(symbol) = self.symbol(name, *span) else {
                        continue;
                    };
                    if interrupt.is_some() && *call_convention == CallConvention::C {
                        self.diagnostic(
                            "E_CALL_CONVENTION",
                            "interrupt functions cannot use the C calling convention",
                            *span,
                        );
                    }
                    let generic = self.generic(generic_params);
                    self.functions.push(FunctionDef {
                        symbol,
                        generic,
                        name: name.clone(),
                        public: *is_pub,
                        kind: *kind,
                        drop_target: None,
                        convention: *call_convention,
                        interrupt: *interrupt,
                        declaration_only: body.is_empty(),
                        generic_params: generic_params.clone(),
                        params: params.iter().map(|param| param.ty.clone()).collect(),
                        result: if matches!(ret_ty, TypeAnnotation::Inferred) {
                            infer_legacy_return_annotation(body, params)
                                .unwrap_or(TypeAnnotation::Unit)
                        } else {
                            ret_ty.clone()
                        },
                        span: *span,
                    });
                }
                Stmt::Const {
                    is_pub,
                    name,
                    ty,
                    span,
                    value,
                } => {
                    if let Some(symbol) = self.symbol(name, *span) {
                        let annotation = if matches!(ty, TypeAnnotation::Inferred) {
                            infer_legacy_global_annotation(value).unwrap_or_else(|| ty.clone())
                        } else {
                            ty.clone()
                        };
                        self.globals.push(GlobalDef {
                            symbol,
                            name: name.clone(),
                            public: *is_pub,
                            mutable: false,
                            constant: true,
                            annotation,
                            initializer: Some(value.clone()),
                            span: *span,
                        });
                    }
                }
                Stmt::Static {
                    is_pub,
                    is_mut,
                    name,
                    ty,
                    span,
                    value,
                } => {
                    if let Some(symbol) = self.symbol(name, *span) {
                        let annotation = if matches!(ty, TypeAnnotation::Inferred) {
                            value
                                .as_ref()
                                .and_then(infer_legacy_global_annotation)
                                .unwrap_or_else(|| ty.clone())
                        } else {
                            ty.clone()
                        };
                        self.globals.push(GlobalDef {
                            symbol,
                            name: name.clone(),
                            public: *is_pub,
                            mutable: *is_mut,
                            constant: false,
                            annotation,
                            initializer: value.clone(),
                            span: *span,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn validate_declarations(&mut self) {
        for index in 0..self.definitions.len() {
            let definition = self.definitions[index].clone();
            let mut generics = BTreeMap::new();
            if let Some(owner) = definition.generic {
                for (position, name) in definition.generic_params.iter().enumerate() {
                    let id = self.types.intern(CanonicalType::GenericParam {
                        owner,
                        index: position as u32,
                    });
                    generics.insert(name.clone(), id);
                }
            }
            match definition.kind {
                TypeDefKind::Struct { fields, .. } => {
                    let mut seen = BTreeSet::new();
                    for field in fields {
                        if !seen.insert(field.name.clone()) {
                            self.diagnostic(
                                "E_DUPLICATE_FIELD",
                                format!("duplicate field `{}`", field.name),
                                field.span,
                            );
                        }
                        self.resolve_type(&field.annotation, &generics, field.span);
                    }
                }
                TypeDefKind::Enum { variants } => {
                    let mut seen = BTreeSet::new();
                    for variant in variants {
                        if !seen.insert(variant.name.clone()) {
                            self.diagnostic(
                                "E_DUPLICATE_VARIANT",
                                format!("duplicate variant `{}`", variant.name),
                                variant.span,
                            );
                        }
                        for payload in variant.payload {
                            self.resolve_type(&payload, &generics, variant.span);
                        }
                    }
                }
            }
        }
        for index in 0..self.functions.len() {
            let function = self.functions[index].clone();
            let mut generics = BTreeMap::new();
            if let Some(owner) = function.generic {
                for (position, name) in function.generic_params.iter().enumerate() {
                    let id = self.types.intern(CanonicalType::GenericParam {
                        owner,
                        index: position as u32,
                    });
                    generics.insert(name.clone(), id);
                }
            }
            for param in &function.params {
                self.resolve_type(param, &generics, function.span);
            }
            let result = self.resolve_type(&function.result, &generics, function.span);
            if function.kind == FunctionKind::Drop {
                let mut target = None;
                if function.public {
                    self.diagnostic(
                        "E_DROP_PUBLIC",
                        "drop functions are compiler-owned and cannot be public",
                        function.span,
                    );
                }
                if function.convention != CallConvention::DByte || function.interrupt.is_some() {
                    self.diagnostic(
                        "E_DROP_SIGNATURE",
                        "drop function must use the ordinary DByte convention",
                        function.span,
                    );
                }
                if !matches!(
                    function.result,
                    TypeAnnotation::Unit | TypeAnnotation::Inferred
                ) {
                    self.diagnostic(
                        "E_DROP_SIGNATURE",
                        "drop function must return unit",
                        function.span,
                    );
                }
                if function.params.len() != 1 {
                    self.diagnostic(
                        "E_DROP_SIGNATURE",
                        "drop function requires exactly one `borrow mut T` parameter",
                        function.span,
                    );
                } else if let Some(parameter) = function.params.first() {
                    if let Some(parameter_ty) =
                        self.resolve_type(parameter, &generics, function.span)
                    {
                        match self.types.values[parameter_ty.0 as usize] {
                            CanonicalType::Borrow {
                                mutable: true,
                                pointee,
                            } => match self.types.values[pointee.0 as usize] {
                                CanonicalType::Struct { symbol, .. }
                                | CanonicalType::Enum { symbol, .. } => target = Some(symbol),
                                _ => self.diagnostic(
                                    "E_DROP_TARGET",
                                    "drop target must be a nominal struct or enum",
                                    function.span,
                                ),
                            },
                            _ => self.diagnostic(
                                "E_DROP_SIGNATURE",
                                "drop parameter must be `borrow mut T`",
                                function.span,
                            ),
                        }
                    }
                }
                if let Some(symbol) = target {
                    if let Some(owner) = self
                        .definitions
                        .iter()
                        .find(|definition| definition.symbol == symbol)
                        .cloned()
                    {
                        if owner.span.line / 1_000_000 != function.span.line / 1_000_000 {
                            self.diagnostic(
                                "E_DROP_FOREIGN_TYPE",
                                "drop function and nominal type must have the same module owner",
                                function.span,
                            );
                        }
                        if owner.generic_params != function.generic_params {
                            self.diagnostic(
                                "E_DROP_GENERIC_MISMATCH",
                                "generic drop parameters must exactly match the owning type",
                                function.span,
                            );
                        }
                    }
                    if self.functions[..index].iter().any(|candidate| {
                        candidate.kind == FunctionKind::Drop
                            && candidate.drop_target == Some(symbol)
                    }) {
                        self.diagnostic(
                            "E_DUPLICATE_DROP",
                            "nominal type has more than one drop function",
                            function.span,
                        );
                    } else {
                        self.functions[index].drop_target = Some(symbol);
                    }
                }
            }
            if function.convention == CallConvention::C {
                for param in &function.params {
                    if let Some(ty) = self.resolve_type(param, &generics, function.span) {
                        self.validate_c_abi_type(ty, function.span);
                    }
                }
                if let Some(result) = result {
                    self.validate_c_abi_type(result, function.span);
                }
                if function.generic.is_some() {
                    self.diagnostic(
                        "E_C_ABI_GENERIC",
                        "open generic functions cannot use extern C",
                        function.span,
                    );
                }
            }
        }
        for global in self.globals.clone() {
            let expected = self.resolve_type(&global.annotation, &BTreeMap::new(), global.span);
            if let Some(initializer) = &global.initializer {
                let mut ownership = Ownership::default();
                ownership.push_scope();
                let actual = self.check_expr_expected(
                    initializer,
                    expected,
                    &BTreeMap::new(),
                    &mut ownership,
                    true,
                );
                if let (Some(actual), Some(expected)) = (actual, expected) {
                    if !self.types_compatible(actual, expected) {
                        self.diagnostic(
                            "E_TYPE_MISMATCH",
                            format!("global `{}` initializer has the wrong type", global.name),
                            global.span,
                        );
                    }
                }
            }
        }
        self.validate_recursive_layouts();
    }

    fn validate_recursive_layouts(&mut self) {
        for definition in self.definitions.clone() {
            let mut active = BTreeSet::new();
            if self.definition_contains_by_value_cycle(definition.symbol, &mut active) {
                self.diagnostic(
                    "E_RECURSIVE_LAYOUT",
                    format!(
                        "type `{}` is recursively embedded by value",
                        definition.name
                    ),
                    definition.span,
                );
            }
        }
    }

    fn definition_contains_by_value_cycle(
        &self,
        symbol: SymbolId,
        active: &mut BTreeSet<SymbolId>,
    ) -> bool {
        if !active.insert(symbol) {
            return true;
        }
        let Some(definition) = self
            .definitions
            .iter()
            .find(|definition| definition.symbol == symbol)
        else {
            return false;
        };
        let annotations: Vec<_> = match &definition.kind {
            TypeDefKind::Struct { fields, .. } => {
                fields.iter().map(|field| &field.annotation).collect()
            }
            TypeDefKind::Enum { variants } => variants
                .iter()
                .flat_map(|variant| variant.payload.iter())
                .collect(),
        };
        let cycle = annotations
            .into_iter()
            .any(|annotation| self.annotation_contains_cycle(annotation, active));
        active.remove(&symbol);
        cycle
    }

    fn annotation_contains_cycle(
        &self,
        annotation: &TypeAnnotation,
        active: &mut BTreeSet<SymbolId>,
    ) -> bool {
        match annotation {
            TypeAnnotation::Array { element, .. } => {
                self.annotation_contains_cycle(element, active)
            }
            TypeAnnotation::Named(name) | TypeAnnotation::Generic { base: name, .. } => self
                .definitions
                .iter()
                .find(|definition| definition.name == *name)
                .is_some_and(|definition| {
                    self.definition_contains_by_value_cycle(definition.symbol, active)
                }),
            TypeAnnotation::Pointer(_)
            | TypeAnnotation::ConstPointer(_)
            | TypeAnnotation::Slice(_)
            | TypeAnnotation::MutSlice(_)
            | TypeAnnotation::Function { .. }
            | TypeAnnotation::Closure { .. } => false,
            _ => false,
        }
    }

    fn resolve_type(
        &mut self,
        annotation: &TypeAnnotation,
        generics: &BTreeMap<String, TypeId>,
        span: Span,
    ) -> Option<TypeId> {
        let canonical = match annotation {
            TypeAnnotation::Unit => CanonicalType::Unit,
            TypeAnnotation::Int => CanonicalType::Integer(IntegerType::LegacyInt),
            TypeAnnotation::I8 => CanonicalType::Integer(IntegerType::I8),
            TypeAnnotation::U8 => CanonicalType::Integer(IntegerType::U8),
            TypeAnnotation::I16 => CanonicalType::Integer(IntegerType::I16),
            TypeAnnotation::U16 => CanonicalType::Integer(IntegerType::U16),
            TypeAnnotation::I32 => CanonicalType::Integer(IntegerType::I32),
            TypeAnnotation::U32 => CanonicalType::Integer(IntegerType::U32),
            TypeAnnotation::Usize => CanonicalType::Integer(IntegerType::Usize),
            TypeAnnotation::Float => CanonicalType::Float,
            TypeAnnotation::Bool => CanonicalType::Bool,
            TypeAnnotation::Str => CanonicalType::LegacyStr,
            TypeAnnotation::Bytes => CanonicalType::LegacyBytes,
            TypeAnnotation::Buffer => CanonicalType::LegacyBuffer,
            TypeAnnotation::StringView => CanonicalType::StringView,
            TypeAnnotation::List(inner) => {
                let inner = self.resolve_type(inner, generics, span)?;
                CanonicalType::LegacyList(inner)
            }
            TypeAnnotation::Pointer(inner) => {
                let pointee = self.resolve_type(inner, generics, span)?;
                CanonicalType::Pointer {
                    mutable: true,
                    pointee,
                }
            }
            TypeAnnotation::ConstPointer(inner) => {
                let pointee = self.resolve_type(inner, generics, span)?;
                CanonicalType::Pointer {
                    mutable: false,
                    pointee,
                }
            }
            TypeAnnotation::Slice(inner) => {
                let element = self.resolve_type(inner, generics, span)?;
                CanonicalType::Slice {
                    mutable: false,
                    element,
                }
            }
            TypeAnnotation::MutSlice(inner) => {
                let element = self.resolve_type(inner, generics, span)?;
                CanonicalType::Slice {
                    mutable: true,
                    element,
                }
            }
            TypeAnnotation::Borrow { mutable, pointee } => {
                let pointee = self.resolve_type(pointee, generics, span)?;
                CanonicalType::Borrow {
                    mutable: *mutable,
                    pointee,
                }
            }
            TypeAnnotation::Array { element, length } => {
                let element = self.resolve_type(element, generics, span)?;
                CanonicalType::Array {
                    element,
                    length: *length,
                }
            }
            TypeAnnotation::Function {
                convention,
                params,
                ret,
            } => {
                let params = params
                    .iter()
                    .filter_map(|param| self.resolve_type(param, generics, span))
                    .collect();
                let result = self.resolve_type(ret, generics, span)?;
                CanonicalType::Function {
                    convention: *convention,
                    params,
                    result,
                }
            }
            TypeAnnotation::Closure { params, ret } => {
                let params = params
                    .iter()
                    .filter_map(|param| self.resolve_type(param, generics, span))
                    .collect();
                let result = self.resolve_type(ret, generics, span)?;
                CanonicalType::ClosureShape { params, result }
            }
            TypeAnnotation::Named(name) => {
                if let Some(id) = generics.get(name) {
                    return Some(*id);
                }
                let Some(symbol) = self.names.get(name).copied() else {
                    self.diagnostic("E_UNKNOWN_TYPE", format!("unknown type `{name}`"), span);
                    return None;
                };
                let Some(definition) = self
                    .definitions
                    .iter()
                    .find(|definition| definition.symbol == symbol)
                else {
                    self.diagnostic("E_NOT_A_TYPE", format!("`{name}` is not a type"), span);
                    return None;
                };
                if !definition.generic_params.is_empty() {
                    self.diagnostic(
                        "E_GENERIC_ARGUMENT_COUNT",
                        format!("generic type `{name}` requires type arguments"),
                        span,
                    );
                    return None;
                }
                match definition.kind {
                    TypeDefKind::Struct { .. } => CanonicalType::Struct {
                        symbol,
                        args: Vec::new(),
                    },
                    TypeDefKind::Enum { .. } => CanonicalType::Enum {
                        symbol,
                        args: Vec::new(),
                    },
                }
            }
            TypeAnnotation::Generic { base, args } => {
                let Some(symbol) = self.names.get(base).copied() else {
                    self.diagnostic("E_UNKNOWN_TYPE", format!("unknown type `{base}`"), span);
                    return None;
                };
                let Some(definition) = self
                    .definitions
                    .iter()
                    .find(|definition| definition.symbol == symbol)
                    .cloned()
                else {
                    self.diagnostic("E_NOT_A_TYPE", format!("`{base}` is not a type"), span);
                    return None;
                };
                if args.len() != definition.generic_params.len() {
                    self.diagnostic(
                        "E_GENERIC_ARGUMENT_COUNT",
                        format!(
                            "generic type `{base}` expects {} arguments, found {}",
                            definition.generic_params.len(),
                            args.len()
                        ),
                        span,
                    );
                    return None;
                }
                let resolved: Vec<_> = args
                    .iter()
                    .filter_map(|arg| self.resolve_type(arg, generics, span))
                    .collect();
                if let Some(generic) = definition.generic {
                    self.specializations.insert(SpecializationKey {
                        definition: generic,
                        arguments: resolved.clone(),
                    });
                }
                match definition.kind {
                    TypeDefKind::Struct { .. } => CanonicalType::Struct {
                        symbol,
                        args: resolved,
                    },
                    TypeDefKind::Enum { .. } => CanonicalType::Enum {
                        symbol,
                        args: resolved,
                    },
                }
            }
            TypeAnnotation::Inferred => return Some(TypeId(0)),
        };
        Some(self.types.intern(canonical))
    }

    fn validate_c_abi_type(&mut self, ty: TypeId, span: Span) {
        match self.types.values[ty.0 as usize].clone() {
            CanonicalType::Unit
            | CanonicalType::Integer(_)
            | CanonicalType::Pointer { .. }
            | CanonicalType::Function {
                convention: CallConvention::C,
                ..
            } => {}
            other => self.diagnostic(
                "E_C_ABI_TYPE",
                format!("type `{other:?}` is outside the DByte i686 C ABI subset"),
                span,
            ),
        }
    }

    fn type_has_nominal_name(&self, ty: TypeId, expected: &str) -> bool {
        match self.types.values.get(ty.0 as usize) {
            Some(CanonicalType::Struct { symbol, .. } | CanonicalType::Enum { symbol, .. }) => {
                self.definitions.iter().any(|definition| {
                    definition.symbol == *symbol && definition.name.ends_with(expected)
                })
            }
            _ => false,
        }
    }

    fn validate_bodies(&mut self, program: &Program) {
        let top_level: Vec<_> = program
            .stmts
            .iter()
            .filter(|statement| {
                !matches!(
                    statement,
                    Stmt::FnDef { .. }
                        | Stmt::StructDef { .. }
                        | Stmt::EnumDef { .. }
                        | Stmt::Import { .. }
                        | Stmt::Const { .. }
                        | Stmt::Static { .. }
                )
            })
            .cloned()
            .collect();
        if !top_level.is_empty() {
            let mut ownership = Ownership::default();
            ownership.push_scope();
            self.current_return = None;
            self.current_generic = None;
            self.check_block(&top_level, &BTreeMap::new(), &mut ownership, false);
            ownership.pop_scope();
        }

        for statement in &program.stmts {
            let Stmt::FnDef {
                name,
                generic_params,
                params,
                body,
                span,
                ..
            } = statement
            else {
                continue;
            };
            if body.is_empty() {
                continue;
            }
            let function = self
                .functions
                .iter()
                .find(|function| {
                    function.name == *name
                        || function.name.rsplit('.').next() == Some(name.as_str())
                })
                .cloned();
            let mut generics = BTreeMap::new();
            if let Some(owner) = function.as_ref().and_then(|function| function.generic) {
                for (index, parameter) in generic_params.iter().enumerate() {
                    let ty = self.types.intern(CanonicalType::GenericParam {
                        owner,
                        index: index as u32,
                    });
                    generics.insert(parameter.clone(), ty);
                }
            }
            let mut ownership = Ownership::default();
            ownership.push_scope();
            self.current_function = function.as_ref().map(|function| function.symbol);
            self.current_function_source_name = Some(name.clone());
            self.current_parameters = params
                .iter()
                .enumerate()
                .map(|(index, parameter)| (parameter.name.clone(), index as u32))
                .collect();
            self.allocator_bindings.clear();
            self.borrow_bindings.clear();
            for (index, parameter) in params.iter().enumerate() {
                if let Some(ty) = self.resolve_type(&parameter.ty, &generics, parameter.span) {
                    if self.type_has_nominal_name(ty, "AllocatorRef") {
                        if let Some(function) = self.current_function {
                            let origin = self.intern_allocator_origin(AllocatorOrigin::Parameter {
                                function,
                                index: index as u32,
                            });
                            self.allocator_bindings
                                .insert(parameter.name.clone(), origin);
                        }
                    }
                    if matches!(
                        self.types.values[ty.0 as usize],
                        CanonicalType::Slice { .. }
                            | CanonicalType::StringView
                            | CanonicalType::Borrow { .. }
                    ) {
                        self.borrowed_parameters.insert(parameter.name.clone());
                        if let Some(function) = self.current_function {
                            let origin = self.intern_borrow_origin(BorrowOrigin::Parameter {
                                function,
                                index: index as u32,
                            });
                            self.borrow_bindings.insert(parameter.name.clone(), origin);
                        }
                    }
                    ownership.define(parameter.name.clone(), ty, parameter.span);
                }
            }
            self.current_return = function
                .as_ref()
                .and_then(|function| self.resolve_type(&function.result, &generics, function.span));
            self.current_interrupt = function.as_ref().and_then(|function| function.interrupt);
            self.current_generic = function.as_ref().and_then(|function| function.generic);
            self.current_drop_parameter = function
                .as_ref()
                .filter(|function| function.kind == FunctionKind::Drop)
                .and_then(|_| params.first().map(|parameter| parameter.name.clone()));
            self.check_block(body, &generics, &mut ownership, false);
            self.current_return = None;
            self.current_interrupt = None;
            self.current_generic = None;
            self.current_drop_parameter = None;
            self.current_function = None;
            self.current_function_source_name = None;
            self.current_parameters.clear();
            self.allocator_bindings.clear();
            self.borrow_bindings.clear();
            self.borrowed_parameters.clear();
            ownership.pop_scope();
            if name == "kernel_main" && self.profile == Profile::I686User {
                self.diagnostic(
                    "E_PROFILE_CAPABILITY",
                    "i686-dbyte-user does not admit kernel_main",
                    *span,
                );
            }
        }
    }

    fn check_block(
        &mut self,
        statements: &[Stmt],
        generics: &BTreeMap<String, TypeId>,
        ownership: &mut Ownership,
        in_loop: bool,
    ) {
        let defer_base = self.active_defers.len();
        self.lexical_depth += 1;
        ownership.push_scope();
        for statement in statements {
            match statement {
                Stmt::Let {
                    name,
                    ty,
                    value,
                    span,
                    ..
                } => {
                    let explicit = if matches!(ty, TypeAnnotation::Inferred) {
                        None
                    } else {
                        self.resolve_type(ty, generics, *span)
                    };
                    let inferred =
                        self.check_expr_expected(value, explicit, generics, ownership, true);
                    let declared = explicit.or(inferred);
                    if let (Some(actual), Some(expected)) = (inferred, declared) {
                        if !self.types_compatible(actual, expected) {
                            self.diagnostic(
                                "E_TYPE_MISMATCH",
                                format!(
                                    "initializer has type t{}, expected t{}",
                                    actual.0, expected.0
                                ),
                                *span,
                            );
                        }
                    }
                    if let Some(ty) = declared {
                        ownership.define(name.clone(), ty, *span);
                        let (allocator, borrow) = self.expression_origins(value);
                        if let Some(origin) = allocator {
                            self.allocator_bindings.insert(name.clone(), origin);
                        }
                        if let Some(origin) = borrow {
                            self.borrow_bindings.insert(name.clone(), origin);
                        }
                    }
                }
                Stmt::Assign { name, value, span } => {
                    let expected = ownership.lookup(name).map(|binding| binding.ty);
                    let actual =
                        self.check_expr_expected(value, expected, generics, ownership, true);
                    let (allocator_origin, borrow_origin) = self.expression_origins(value);
                    if let Some(binding) = ownership.lookup_mut(name) {
                        if binding.immut_borrows != 0 || binding.mut_borrowed {
                            self.diagnostic(
                                "E_ASSIGN_WHILE_BORROWED",
                                format!("cannot assign `{name}` while borrowed"),
                                *span,
                            );
                        } else if actual
                            .is_some_and(|actual| !self.types_compatible(actual, binding.ty))
                        {
                            self.diagnostic(
                                "E_TYPE_MISMATCH",
                                format!(
                                    "TypeError: cannot assign `{}` to variable of type `{}`",
                                    self.type_name(actual.unwrap()),
                                    self.type_name(binding.ty)
                                ),
                                *span,
                            );
                        } else {
                            binding.moved = false;
                            if let Some(origin) = allocator_origin {
                                self.allocator_bindings.insert(name.clone(), origin);
                            } else {
                                self.allocator_bindings.remove(name);
                            }
                            if let Some(origin) = borrow_origin {
                                self.borrow_bindings.insert(name.clone(), origin);
                            } else {
                                self.borrow_bindings.remove(name);
                            }
                        }
                    } else {
                        if let Some(global) = self
                            .globals
                            .iter()
                            .find(|global| global.name == *name)
                            .cloned()
                        {
                            let expected = self.resolve_type(
                                &global.annotation,
                                &BTreeMap::new(),
                                global.span,
                            );
                            self.check_expr_expected(value, expected, generics, ownership, true);
                            if !global.mutable {
                                self.diagnostic(
                                    "E_IMMUTABLE_GLOBAL",
                                    format!("global `{name}` is immutable"),
                                    *span,
                                );
                            }
                        } else {
                            self.diagnostic(
                                "E_UNKNOWN_SYMBOL",
                                format!("TypeError: undefined variable `{name}`"),
                                *span,
                            );
                        }
                    }
                }
                Stmt::Return { value, span } => {
                    if self.current_return.is_none() {
                        self.diagnostic(
                            "E_RETURN_OUTSIDE_FUNCTION",
                            "TypeError: return outside function",
                            *span,
                        );
                    }
                    self.record_cleanup_edges(CleanupExit::Return, *span, 0);
                    if let Some(value) = value {
                        let (allocator_origin, borrow_origin) = self.expression_origins(value);
                        if let Some(origin) = allocator_origin {
                            if matches!(
                                self.allocator_origins.get(origin.0 as usize),
                                Some(AllocatorOrigin::Lexical { .. })
                            ) {
                                self.diagnostic(
                                    "E_ALLOCATOR_LIFETIME_ESCAPE",
                                    "returned owner outlives its lexical allocator",
                                    value.span(),
                                );
                            }
                            if let Some(function) = self.current_function {
                                self.function_allocator_origins.insert(function, origin);
                            }
                        }
                        if let Some(origin) = borrow_origin {
                            if matches!(
                                self.borrow_origins.get(origin.0 as usize),
                                Some(BorrowOrigin::Lexical { .. })
                            ) {
                                self.diagnostic(
                                    "E_BORROW_ESCAPE",
                                    "returned view outlives its lexical owner",
                                    value.span(),
                                );
                            }
                            if let Some(function) = self.current_function {
                                self.function_borrow_origins.insert(function, origin);
                            }
                        }
                        if self.current_return.is_some_and(|return_type| {
                            matches!(
                                self.types.values[return_type.0 as usize],
                                CanonicalType::Slice { .. }
                                    | CanonicalType::StringView
                                    | CanonicalType::Borrow { .. }
                            )
                        }) {
                            let direct_source = borrow_origin.is_some_and(|origin| {
                                matches!(
                                    self.borrow_origins.get(origin.0 as usize),
                                    Some(BorrowOrigin::Parameter { function, .. })
                                        if Some(*function) == self.current_function
                                )
                            });
                            if !direct_source {
                                self.diagnostic(
                                    "E_BORROW_ESCAPE",
                                    "borrowed return must derive directly from one borrowed parameter",
                                    value.span(),
                                );
                            } else if self.borrowed_parameters.len() != 1 {
                                self.diagnostic(
                                    "E_AMBIGUOUS_BORROWED_RETURN",
                                    "borrowed return source is ambiguous; exactly one borrowed parameter is required",
                                    value.span(),
                                );
                            }
                        }
                        if matches!(value, Expr::Closure { captures, .. } if !captures.is_empty()) {
                            self.diagnostic(
                                "E_CLOSURE_ESCAPE",
                                "capturing closures cannot escape in v12.0",
                                value.span(),
                            );
                        }
                        let actual = self.check_expr_expected(
                            value,
                            self.current_return,
                            generics,
                            ownership,
                            true,
                        );
                        if let (Some(actual), Some(expected)) = (actual, self.current_return) {
                            if !self.types_compatible(actual, expected) {
                                self.diagnostic(
                                    "E_RETURN_TYPE",
                                    format!(
                                        "return expected {}, found {}",
                                        self.type_name(expected),
                                        self.type_name(actual)
                                    ),
                                    *span,
                                );
                            }
                        }
                    } else if self.current_return != Some(TypeId(0)) {
                        self.diagnostic(
                            "E_RETURN_TYPE",
                            "value-returning function requires a return value",
                            *span,
                        );
                    }
                }
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                    span,
                } => {
                    ownership.push_scope();
                    let condition = self.check_expr(cond, generics, ownership, false);
                    ownership.pop_scope();
                    if condition.is_some_and(|ty| {
                        !matches!(self.types.values[ty.0 as usize], CanonicalType::Bool)
                    }) {
                        self.diagnostic(
                            "E_CONDITION_TYPE",
                            "if condition must be bool",
                            cond.span(),
                        );
                    }
                    let entry = ownership.clone();
                    let mut then_state = entry.clone();
                    self.check_block(then_body, generics, &mut then_state, in_loop);
                    let mut else_state = entry.clone();
                    if let Some(body) = else_body {
                        self.check_block(body, generics, &mut else_state, in_loop);
                    }
                    if then_state != else_state {
                        self.diagnostic(
                            "E_OWNERSHIP_MAYBE_MOVED",
                            "if branches leave an owned local in different move or borrow states",
                            *span,
                        );
                        *ownership = entry;
                    } else {
                        *ownership = then_state;
                    }
                }
                Stmt::While { cond, body, span } => {
                    ownership.push_scope();
                    let condition = self.check_expr(cond, generics, ownership, false);
                    ownership.pop_scope();
                    if condition.is_some_and(|ty| {
                        !matches!(self.types.values[ty.0 as usize], CanonicalType::Bool)
                    }) {
                        self.diagnostic(
                            "E_CONDITION_TYPE",
                            format!(
                                "TypeError: while condition must be `bool`, found `{}`",
                                condition.map_or("unknown".into(), |ty| self.type_name(ty))
                            ),
                            cond.span(),
                        );
                    }
                    let entry = ownership.clone();
                    let mut body_state = entry.clone();
                    self.check_block(body, generics, &mut body_state, true);
                    if body_state != entry {
                        self.diagnostic(
                            "E_OWNERSHIP_MAYBE_MOVED",
                            "loop body changes ownership state across a loop join",
                            *span,
                        );
                    }
                    *ownership = entry;
                }
                Stmt::For {
                    var,
                    iterable,
                    body,
                    span,
                } => {
                    let iterable_ty = self.check_expr(iterable, generics, ownership, false);
                    let element_ty =
                        iterable_ty.and_then(|ty| match self.types.values[ty.0 as usize] {
                            CanonicalType::LegacyList(element)
                            | CanonicalType::Array { element, .. }
                            | CanonicalType::Slice { element, .. } => Some(element),
                            CanonicalType::LegacyStr => Some(ty),
                            _ => None,
                        });
                    if element_ty.is_none() {
                        self.diagnostic(
                            "E_FOR_TYPE",
                            format!(
                                "TypeError: `{}` is not iterable",
                                iterable_ty.map_or("unknown".into(), |ty| self.type_name(ty))
                            ),
                            *span,
                        );
                    }
                    let entry = ownership.clone();
                    let mut body_state = entry.clone();
                    body_state.push_scope();
                    if let Some(element_ty) = element_ty {
                        body_state.define(var.clone(), element_ty, *span);
                    }
                    self.check_block(body, generics, &mut body_state, true);
                    body_state.pop_scope();
                    if body_state != entry {
                        self.diagnostic(
                            "E_OWNERSHIP_MAYBE_MOVED",
                            "for body changes ownership state across a loop join",
                            *span,
                        );
                    }
                    *ownership = entry;
                }
                Stmt::Break { span } | Stmt::Continue { span } if !in_loop => self.diagnostic(
                    "E_LOOP_CONTROL",
                    "loop control statement used outside a loop",
                    *span,
                ),
                Stmt::Match {
                    mode,
                    value,
                    arms,
                    span,
                } => self.check_match(*mode, value, arms, generics, ownership, *span),
                Stmt::Defer {
                    captures,
                    body,
                    span,
                } => {
                    self.check_captures(captures, ownership, *span);
                    let mut deferred = self.capture_environment(captures, ownership);
                    self.check_block(body, generics, &mut deferred, in_loop);
                    self.active_defers.push((*span, self.lexical_depth));
                }
                Stmt::Break { span } => {
                    self.record_cleanup_edges(CleanupExit::Break, *span, defer_base)
                }
                Stmt::Continue { span } => {
                    self.record_cleanup_edges(CleanupExit::Continue, *span, defer_base)
                }
                Stmt::Expr(expression) => {
                    ownership.push_scope();
                    self.check_expr(expression, generics, ownership, false);
                    ownership.pop_scope();
                }
                Stmt::Store { target, value, .. } => {
                    ownership.push_scope();
                    self.check_expr(target, generics, ownership, false);
                    self.check_expr(value, generics, ownership, true);
                    ownership.pop_scope();
                }
                Stmt::FnDef { span, .. }
                | Stmt::StructDef { span, .. }
                | Stmt::EnumDef { span, .. } => self.diagnostic(
                    "E_NESTED_DECLARATION",
                    "type and function declarations are module-level only",
                    *span,
                ),
                Stmt::Import { .. } | Stmt::Const { .. } | Stmt::Static { .. } => {}
            }
        }
        for (defer_span, depth) in self.active_defers[defer_base..].iter().rev().copied() {
            self.cleanup_edges.push(CleanupEdge {
                defer_span,
                exit_span: statements.last().map_or(defer_span, Stmt::span),
                exit: CleanupExit::Fallthrough,
                lexical_depth: depth,
            });
        }
        self.active_defers.truncate(defer_base);
        ownership.pop_scope();
        self.lexical_depth -= 1;
    }

    fn record_cleanup_edges(&mut self, exit: CleanupExit, exit_span: Span, base: usize) {
        for (defer_span, depth) in self.active_defers[base..].iter().rev().copied() {
            self.cleanup_edges.push(CleanupEdge {
                defer_span,
                exit_span,
                exit,
                lexical_depth: depth,
            });
        }
    }

    fn intern_allocator_origin(&mut self, origin: AllocatorOrigin) -> AllocatorOriginId {
        if let Some(index) = self
            .allocator_origins
            .iter()
            .position(|candidate| *candidate == origin)
        {
            return AllocatorOriginId(index as u32);
        }
        let id = AllocatorOriginId(self.allocator_origins.len() as u32);
        self.allocator_origins.push(origin);
        id
    }

    fn intern_borrow_origin(&mut self, origin: BorrowOrigin) -> BorrowOriginId {
        if let Some(index) = self
            .borrow_origins
            .iter()
            .position(|candidate| *candidate == origin)
        {
            return BorrowOriginId(index as u32);
        }
        let id = BorrowOriginId(self.borrow_origins.len() as u32);
        self.borrow_origins.push(origin);
        id
    }

    fn expression_origins(
        &mut self,
        expression: &Expr,
    ) -> (Option<AllocatorOriginId>, Option<BorrowOriginId>) {
        let function = self.current_function.unwrap_or(SymbolId(u32::MAX));
        match expression {
            Expr::Ident(name, span) => {
                let allocator = self.allocator_bindings.get(name).copied();
                let borrow = self.borrow_bindings.get(name).copied();
                let _ = span;
                (allocator, borrow)
            }
            Expr::Borrow { expr, .. } => {
                let allocator = self.expression_origins(expr).0;
                let borrow = match expr.as_ref() {
                    Expr::Ident(name, _) => {
                        self.borrow_bindings.get(name).copied().unwrap_or_else(|| {
                            if self.globals.iter().any(|global| global.name == *name) {
                                self.intern_borrow_origin(BorrowOrigin::Static)
                            } else {
                                self.intern_borrow_origin(BorrowOrigin::Lexical {
                                    function,
                                    span: expression.span(),
                                })
                            }
                        })
                    }
                    _ => self.intern_borrow_origin(BorrowOrigin::Lexical {
                        function,
                        span: expression.span(),
                    }),
                };
                (allocator, Some(borrow))
            }
            Expr::Call { name, args, .. } => self.call_origins(name, args, expression.span()),
            Expr::Invoke {
                callee,
                args,
                named_args,
                ..
            } => {
                let name = match callee.as_ref() {
                    Expr::Ident(name, _) => name.as_str(),
                    _ => return (None, None),
                };
                let mut values = args.clone();
                values.extend(named_args.iter().map(|argument| argument.value.clone()));
                self.call_origins(name, &values, expression.span())
            }
            Expr::MemberCall { property, args, .. } => {
                self.call_origins(property, args, expression.span())
            }
            _ => (None, None),
        }
    }

    fn call_origins(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> (Option<AllocatorOriginId>, Option<BorrowOriginId>) {
        let function = self.current_function.unwrap_or(SymbolId(u32::MAX));
        if matches!(
            name,
            "slice_from_raw" | "mut_slice_from_raw" | "string_view_from_raw"
        ) {
            let mut origins = self.borrow_bindings.values().copied().filter(|origin| {
                matches!(
                    self.borrow_origins.get(origin.0 as usize),
                    Some(BorrowOrigin::Parameter { function: owner, .. }) if *owner == function
                )
            });
            let origin = origins.next();
            if origin.is_some() && origins.next().is_none() {
                return (None, origin);
            }
        }
        if ["arena_initialize", "fixed_initialize", "kernel_initialize"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        {
            let is_static = args.first().is_some_and(|argument| {
                matches!(
                    argument,
                    Expr::Unary { op: UnaryOp::AddressOf, expr, .. }
                        if matches!(expr.as_ref(), Expr::Ident(name, _) if self.globals.iter().any(|global| global.name == *name))
                )
            });
            return (
                Some(self.intern_allocator_origin(if is_static {
                    AllocatorOrigin::StaticSystem
                } else {
                    AllocatorOrigin::Lexical { function, span }
                })),
                None,
            );
        }
        if let Some(symbol) = self
            .functions
            .iter()
            .find(|candidate| candidate.name == name)
            .map(|candidate| candidate.symbol)
        {
            let allocator = self
                .function_allocator_origins
                .get(&symbol)
                .copied()
                .and_then(|origin| self.translate_allocator_origin(origin, args));
            let borrow = self
                .function_borrow_origins
                .get(&symbol)
                .copied()
                .and_then(|origin| self.translate_borrow_origin(origin, args));
            if allocator.is_some() || borrow.is_some() {
                return (allocator, borrow);
            }
        }
        let allocator_argument = if name.ends_with("clone") {
            args.first()
        } else if name.ends_with("with_allocator")
            || name.ends_with("with_capacity")
            || name.ends_with("from_view")
            || name.ends_with("clone_in")
        {
            args.last()
        } else {
            None
        };
        let allocator = allocator_argument.and_then(|argument| self.expression_origins(argument).0);
        let borrow_argument = if name.ends_with("as_view")
            || name.ends_with("as_slice")
            || name.ends_with("as_mut_slice")
            || name.ends_with("get_mut")
        {
            args.first()
        } else {
            None
        };
        let borrow = borrow_argument.map(|argument| self.borrow_origin_from_argument(argument));
        (allocator, borrow)
    }

    fn translate_allocator_origin(
        &mut self,
        origin: AllocatorOriginId,
        args: &[Expr],
    ) -> Option<AllocatorOriginId> {
        match self.allocator_origins.get(origin.0 as usize)?.clone() {
            AllocatorOrigin::StaticSystem => Some(origin),
            AllocatorOrigin::Parameter { index, .. } => args
                .get(index as usize)
                .and_then(|argument| self.expression_origins(argument).0),
            AllocatorOrigin::Lexical { .. } => None,
        }
    }

    fn translate_borrow_origin(
        &mut self,
        origin: BorrowOriginId,
        args: &[Expr],
    ) -> Option<BorrowOriginId> {
        match self.borrow_origins.get(origin.0 as usize)?.clone() {
            BorrowOrigin::Static => Some(origin),
            BorrowOrigin::Parameter { index, .. } => args
                .get(index as usize)
                .map(|argument| self.borrow_origin_from_argument(argument)),
            BorrowOrigin::Lexical { .. } => None,
        }
    }

    fn borrow_origin_from_argument(&mut self, argument: &Expr) -> BorrowOriginId {
        if let Some(origin) = self.expression_origins(argument).1 {
            return origin;
        }
        let function = self.current_function.unwrap_or(SymbolId(u32::MAX));
        if let Expr::Ident(name, _) = argument {
            if self.globals.iter().any(|global| global.name == *name) {
                return self.intern_borrow_origin(BorrowOrigin::Static);
            }
        }
        self.intern_borrow_origin(BorrowOrigin::Lexical {
            function,
            span: argument.span(),
        })
    }

    fn check_expr(
        &mut self,
        expression: &Expr,
        generics: &BTreeMap<String, TypeId>,
        ownership: &mut Ownership,
        consume: bool,
    ) -> Option<TypeId> {
        let ty = self.check_expr_inner(expression, generics, ownership, consume);
        if let Some(ty) = ty {
            let category = if matches!(
                expression,
                Expr::Ident(..) | Expr::Index { .. } | Expr::Member { .. }
            ) {
                ValueCategory::LValue
            } else {
                ValueCategory::RValue
            };
            let action = match expression {
                Expr::Borrow { mutable: true, .. } => OwnershipAction::BorrowMut,
                Expr::Borrow { mutable: false, .. } => OwnershipAction::Borrow,
                _ if consume && !self.is_copy(ty) => OwnershipAction::Move,
                _ => OwnershipAction::Copy,
            };
            if action == OwnershipAction::Move
                && self
                    .current_drop_parameter
                    .as_deref()
                    .is_some_and(|parameter| drop_parameter_projection(expression, parameter))
            {
                self.diagnostic(
                    "E_DROP_FIELD_MOVE",
                    "drop function cannot move a non-Copy field out of the value being destroyed",
                    expression.span(),
                );
            }
            let (allocator_origin, borrow_origin) = self.expression_origins(expression);
            self.expressions.push(HirExpression {
                span: expression.span(),
                ty,
                category,
                ownership: action,
                allocator_origin,
                borrow_origin,
            });
        }
        ty
    }

    fn check_expr_expected(
        &mut self,
        expression: &Expr,
        expected: Option<TypeId>,
        generics: &BTreeMap<String, TypeId>,
        ownership: &mut Ownership,
        consume: bool,
    ) -> Option<TypeId> {
        if let Some(expected) = expected {
            if matches!(
                self.types.values[expected.0 as usize],
                CanonicalType::Integer(_)
            ) {
                if let Expr::Binary {
                    left,
                    op:
                        BinOp::Add
                        | BinOp::Sub
                        | BinOp::Mul
                        | BinOp::Div
                        | BinOp::Rem
                        | BinOp::BitAnd
                        | BinOp::BitOr
                        | BinOp::BitXor
                        | BinOp::Shl
                        | BinOp::Shr,
                    ..
                } = expression
                {
                    self.check_expr_expected(left, Some(expected), generics, ownership, false);
                    self.check_expr_expected(
                        match expression {
                            Expr::Binary { right, .. } => right,
                            _ => unreachable!(),
                        },
                        Some(expected),
                        generics,
                        ownership,
                        false,
                    );
                    self.expressions.push(HirExpression {
                        span: expression.span(),
                        ty: expected,
                        category: ValueCategory::RValue,
                        ownership: OwnershipAction::Copy,
                        allocator_origin: None,
                        borrow_origin: None,
                    });
                    return Some(expected);
                }
            }
            if let CanonicalType::Integer(integer) = self.types.values[expected.0 as usize] {
                let literal = match expression {
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
                };
                if let Some(value) = literal {
                    if !integer_literal_fits(integer, value, self.layout) {
                        self.diagnostic(
                            "E_INTEGER_LITERAL_RANGE",
                            format!("integer literal `{value}` is outside `{integer:?}`"),
                            expression.span(),
                        );
                    }
                    self.expressions.push(HirExpression {
                        span: expression.span(),
                        ty: expected,
                        category: ValueCategory::RValue,
                        ownership: OwnershipAction::Copy,
                        allocator_origin: None,
                        borrow_origin: None,
                    });
                    return Some(expected);
                }
            }
            let contextual = matches!(
                (&self.types.values[expected.0 as usize], expression),
                (CanonicalType::Pointer { .. }, Expr::NullLit(_))
                    | (
                        CanonicalType::LegacyStr | CanonicalType::StringView,
                        Expr::StrLit(..)
                    )
            );
            if contextual {
                self.expressions.push(HirExpression {
                    span: expression.span(),
                    ty: expected,
                    category: ValueCategory::RValue,
                    ownership: OwnershipAction::Copy,
                    allocator_origin: None,
                    borrow_origin: None,
                });
                return Some(expected);
            }
            if let (CanonicalType::Array { element, length }, Expr::List(values, span)) =
                (self.types.values[expected.0 as usize].clone(), expression)
            {
                if values.len() != length as usize {
                    self.diagnostic(
                        "E_ARRAY_LENGTH",
                        format!(
                            "array initializer expects {length} elements, found {}",
                            values.len()
                        ),
                        *span,
                    );
                }
                for value in values {
                    self.check_expr_expected(value, Some(element), generics, ownership, true);
                }
                self.expressions.push(HirExpression {
                    span: expression.span(),
                    ty: expected,
                    category: ValueCategory::RValue,
                    ownership: OwnershipAction::Copy,
                    allocator_origin: None,
                    borrow_origin: None,
                });
                return Some(expected);
            }
            if let (CanonicalType::LegacyList(element), Expr::List(values, _)) =
                (self.types.values[expected.0 as usize].clone(), expression)
            {
                for value in values {
                    self.check_expr_expected(value, Some(element), generics, ownership, true);
                }
                self.expressions.push(HirExpression {
                    span: expression.span(),
                    ty: expected,
                    category: ValueCategory::RValue,
                    ownership: OwnershipAction::Copy,
                    allocator_origin: None,
                    borrow_origin: None,
                });
                return Some(expected);
            }
        }
        let actual = self.check_expr(expression, generics, ownership, consume)?;
        if let Some(expected) = expected {
            let slice_view = matches!(
                (
                    &self.types.values[actual.0 as usize],
                    &self.types.values[expected.0 as usize]
                ),
                (
                    CanonicalType::Borrow {
                        mutable: actual_mutable,
                        pointee
                    },
                    CanonicalType::Slice {
                        mutable: expected_mutable,
                        element: expected_element
                    }
                ) if matches!(
                    &self.types.values[pointee.0 as usize],
                    CanonicalType::Array { element, .. } if element == expected_element
                ) && (!*expected_mutable || *actual_mutable)
            );
            if slice_view {
                self.expressions.push(HirExpression {
                    span: expression.span(),
                    ty: expected,
                    category: ValueCategory::RValue,
                    ownership: OwnershipAction::Copy,
                    allocator_origin: None,
                    borrow_origin: None,
                });
                return Some(expected);
            }
        }
        if let (
            Expr::Closure { captures, .. },
            CanonicalType::ClosureShape {
                params: closure_params,
                result: closure_result,
            },
            Some(CanonicalType::Function {
                convention: CallConvention::DByte,
                params,
                result,
            }),
        ) = (
            expression,
            self.types.values[actual.0 as usize].clone(),
            expected.map(|expected| self.types.values[expected.0 as usize].clone()),
        ) {
            if captures.is_empty() && closure_params == params && closure_result == result {
                return expected;
            }
        }
        Some(actual)
    }

    fn check_expr_inner(
        &mut self,
        expression: &Expr,
        generics: &BTreeMap<String, TypeId>,
        ownership: &mut Ownership,
        consume: bool,
    ) -> Option<TypeId> {
        match expression {
            Expr::UnitLit(..) => Some(self.types.intern(CanonicalType::Unit)),
            Expr::IntLit(..) => Some(self.types.intern(CanonicalType::Integer(IntegerType::I32))),
            Expr::FloatLit(..) => Some(self.types.intern(CanonicalType::Float)),
            Expr::BoolLit(..) => Some(self.types.intern(CanonicalType::Bool)),
            Expr::StrLit(..) => Some(self.types.intern(
                if matches!(self.profile, Profile::Hosted | Profile::Vm) {
                    CanonicalType::LegacyStr
                } else {
                    CanonicalType::StringView
                },
            )),
            Expr::BytesLit(..) => Some(self.types.intern(CanonicalType::LegacyBytes)),
            Expr::FStr(_, span) => {
                if !matches!(self.profile, Profile::Hosted | Profile::Vm) {
                    self.diagnostic(
                        "E_LEGACY_TEXT",
                        "formatted strings retain legacy str semantics and are hosted-only",
                        *span,
                    );
                }
                Some(self.types.intern(CanonicalType::LegacyStr))
            }
            Expr::NullLit(span) => {
                self.diagnostic(
                    "E_NULL_INFERENCE",
                    "null requires an expected raw-pointer type",
                    *span,
                );
                None
            }
            Expr::Drop { expr, .. } => {
                self.check_expr(expr, generics, ownership, true)?;
                Some(self.types.intern(CanonicalType::Unit))
            }
            Expr::Ident(name, span) => {
                let Some(binding) = ownership.lookup_mut(name) else {
                    if let Some(global) = self
                        .globals
                        .iter()
                        .find(|global| global.name == *name)
                        .cloned()
                    {
                        return self.resolve_type(
                            &global.annotation,
                            &BTreeMap::new(),
                            global.span,
                        );
                    }
                    if let Some(function) = self
                        .functions
                        .iter()
                        .find(|function| function.name == *name)
                        .cloned()
                    {
                        if function.generic.is_some() {
                            self.diagnostic(
                                "E_GENERIC_FUNCTION_POINTER",
                                "generic functions require specialization before address use",
                                *span,
                            );
                            return None;
                        }
                        let params = function
                            .params
                            .iter()
                            .filter_map(|annotation| {
                                self.resolve_type(annotation, &BTreeMap::new(), *span)
                            })
                            .collect();
                        let result =
                            self.resolve_type(&function.result, &BTreeMap::new(), function.span)?;
                        return Some(self.types.intern(CanonicalType::Function {
                            convention: function.convention,
                            params,
                            result,
                        }));
                    }
                    self.diagnostic(
                        "E_UNKNOWN_SYMBOL",
                        format!("TypeError: undefined variable `{name}`"),
                        *span,
                    );
                    return None;
                };
                if binding.moved {
                    if let CanonicalType::GenericParam { owner, index } =
                        self.types.values[binding.ty.0 as usize]
                    {
                        self.generic_capabilities
                            .entry((owner, index))
                            .or_default()
                            .insert(BuiltinCapability::Copy);
                        binding.moved = false;
                        return Some(binding.ty);
                    }
                    self.diagnostic(
                        "E_USE_AFTER_MOVE",
                        format!("binding `{name}` was moved"),
                        *span,
                    );
                    return Some(binding.ty);
                }
                let ty = binding.ty;
                if consume && !self.is_copy(ty) {
                    if binding.immut_borrows != 0 || binding.mut_borrowed {
                        self.diagnostic(
                            "E_MOVE_WHILE_BORROWED",
                            format!("cannot move `{name}` while borrowed"),
                            *span,
                        );
                    } else {
                        binding.moved = true;
                    }
                }
                Some(ty)
            }
            Expr::Borrow {
                mutable,
                expr,
                span,
            } => {
                if matches!(
                    expr.as_ref(),
                    Expr::Unary {
                        op: UnaryOp::Deref,
                        ..
                    } | Expr::Member { .. }
                        | Expr::Index { .. }
                ) {
                    let ty = self.check_expr(expr, generics, ownership, false)?;
                    return Some(self.types.intern(CanonicalType::Borrow {
                        mutable: *mutable,
                        pointee: ty,
                    }));
                }
                let Expr::Ident(name, _) = expr.as_ref() else {
                    self.diagnostic(
                        "E_BORROW_TEMPORARY",
                        "v12.0 does not extend temporary lifetimes",
                        *span,
                    );
                    return None;
                };
                let Some(binding) = ownership.lookup_mut(name) else {
                    self.diagnostic(
                        "E_UNKNOWN_SYMBOL",
                        format!("TypeError: undefined variable `{name}`"),
                        *span,
                    );
                    return None;
                };
                let ty = binding.ty;
                let mut record = None;
                if binding.moved {
                    self.diagnostic(
                        "E_USE_AFTER_MOVE",
                        format!("cannot borrow moved binding `{name}`"),
                        *span,
                    );
                } else if *mutable {
                    if binding.mut_borrowed || binding.immut_borrows != 0 {
                        self.diagnostic(
                            "E_BORROW_CONFLICT",
                            format!("mutable borrow of `{name}` is not exclusive"),
                            *span,
                        );
                    } else {
                        binding.mut_borrowed = true;
                        record = Some(true);
                    }
                } else if binding.mut_borrowed {
                    self.diagnostic(
                        "E_BORROW_CONFLICT",
                        format!("immutable borrow conflicts with mutable borrow of `{name}`"),
                        *span,
                    );
                } else {
                    binding.immut_borrows += 1;
                    record = Some(false);
                }
                if let Some(mutable) = record {
                    ownership.record_borrow(name.clone(), mutable);
                }
                Some(self.types.intern(CanonicalType::Borrow {
                    mutable: *mutable,
                    pointee: ty,
                }))
            }
            Expr::List(values, _) => {
                let first = values
                    .first()
                    .and_then(|value| self.check_expr(value, generics, ownership, true))?;
                for value in values.iter().skip(1) {
                    let actual = self.check_expr(value, generics, ownership, true);
                    if actual.is_some_and(|actual| actual != first) {
                        self.diagnostic(
                            "E_TYPE_MISMATCH",
                            "aggregate initializer element types differ",
                            value.span(),
                        );
                    }
                }
                Some(self.types.intern(CanonicalType::LegacyList(first)))
            }
            Expr::Binary {
                left, op, right, ..
            } => {
                let left_is_contextual_literal = is_integer_literal_expression(left)
                    || matches!(left.as_ref(), Expr::StrLit(..));
                let (left, right) = if left_is_contextual_literal {
                    let right = self.check_expr(right, generics, ownership, false);
                    let left = self.check_expr_expected(left, right, generics, ownership, false);
                    (left, right)
                } else {
                    let left = self.check_expr(left, generics, ownership, false);
                    let right = self.check_expr_expected(right, left, generics, ownership, false);
                    (left, right)
                };
                let pointer_arithmetic = matches!(
                    (left, right, op),
                    (
                        Some(pointer),
                        Some(index),
                        BinOp::Add | BinOp::Sub
                    ) if matches!(self.types.values[pointer.0 as usize], CanonicalType::Pointer { .. })
                        && matches!(self.types.values[index.0 as usize], CanonicalType::Integer(IntegerType::Usize))
                );
                if !left.zip(right).is_some_and(|(left, right)| {
                    self.types_compatible(left, right) || self.types_compatible(right, left)
                }) && !pointer_arithmetic
                {
                    self.diagnostic(
                        "E_TYPE_MISMATCH",
                        "binary operands require identical canonical types",
                        expression.span(),
                    );
                }
                if let Some(ty) = left {
                    if let CanonicalType::GenericParam { owner, index } =
                        self.types.values[ty.0 as usize]
                    {
                        let capability = match op {
                            BinOp::Add
                            | BinOp::Sub
                            | BinOp::Mul
                            | BinOp::Div
                            | BinOp::Rem
                            | BinOp::BitAnd
                            | BinOp::BitOr
                            | BinOp::BitXor
                            | BinOp::Shl
                            | BinOp::Shr => BuiltinCapability::Numeric,
                            BinOp::EqEq | BinOp::NotEq => BuiltinCapability::Equality,
                            BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq => {
                                BuiltinCapability::Ordering
                            }
                        };
                        self.generic_capabilities
                            .entry((owner, index))
                            .or_default()
                            .insert(capability);
                    }
                }
                if matches!(
                    op,
                    BinOp::EqEq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq
                ) {
                    Some(self.types.intern(CanonicalType::Bool))
                } else {
                    left
                }
            }
            Expr::Unary { op, expr, span } => {
                let operand = self.check_expr(expr, generics, ownership, false)?;
                match op {
                    UnaryOp::Deref => match self.types.values[operand.0 as usize] {
                        CanonicalType::Pointer { pointee, .. }
                        | CanonicalType::Borrow { pointee, .. } => Some(pointee),
                        _ => {
                            self.diagnostic(
                                "E_DEREFERENCE_TYPE",
                                "dereference requires a raw pointer or borrow",
                                *span,
                            );
                            None
                        }
                    },
                    UnaryOp::AddressOf => Some(self.types.intern(CanonicalType::Pointer {
                        mutable: true,
                        pointee: operand,
                    })),
                    UnaryOp::Neg => {
                        if !matches!(
                            self.types.values[operand.0 as usize],
                            CanonicalType::Integer(_) | CanonicalType::Float
                        ) {
                            self.diagnostic(
                                "E_UNARY_TYPE",
                                "numeric negation requires a numeric operand",
                                *span,
                            );
                        }
                        Some(operand)
                    }
                    UnaryOp::Not => {
                        if !matches!(self.types.values[operand.0 as usize], CanonicalType::Bool) {
                            self.diagnostic("E_UNARY_TYPE", "logical not requires bool", *span);
                        }
                        Some(self.types.intern(CanonicalType::Bool))
                    }
                    UnaryOp::BitNot => {
                        if !matches!(
                            self.types.values[operand.0 as usize],
                            CanonicalType::Integer(_)
                        ) {
                            self.diagnostic(
                                "E_UNARY_TYPE",
                                "bitwise not requires an integer",
                                *span,
                            );
                        }
                        Some(operand)
                    }
                }
            }
            Expr::Try { expr, span } => {
                self.record_cleanup_edges(CleanupExit::PropagateError, *span, 0);
                let value = self.check_expr(expr, generics, ownership, true)?;
                let CanonicalType::Enum { symbol, args } =
                    self.types.values[value.0 as usize].clone()
                else {
                    self.diagnostic("E_TRY_TYPE", "`?` requires Result[T, E]", *span);
                    return None;
                };
                let is_result = self
                    .definitions
                    .iter()
                    .any(|definition| definition.symbol == symbol && definition.name == "Result");
                if !is_result || args.len() != 2 {
                    self.diagnostic("E_TRY_TYPE", "`?` requires canonical Result[T, E]", *span);
                    return None;
                }
                let compatible_return = self.current_return.is_some_and(|return_type| {
                    matches!(
                        &self.types.values[return_type.0 as usize],
                        CanonicalType::Enum {
                            symbol: return_symbol,
                            args: return_args
                        } if *return_symbol == symbol
                            && return_args.len() == 2
                            && return_args[1] == args[1]
                    )
                });
                if !compatible_return {
                    self.diagnostic(
                        "E_TRY_RETURN_TYPE",
                        "`?` requires the enclosing function to return Result with the identical error type",
                        *span,
                    );
                }
                Some(args[0])
            }
            Expr::Closure {
                captures,
                params,
                ret_ty,
                body,
                span,
            } => {
                self.check_captures(captures, ownership, *span);
                let mut closure_ownership = self.capture_environment(captures, ownership);
                let parameter_types: Vec<_> = params
                    .iter()
                    .filter_map(|param| self.resolve_type(&param.ty, generics, param.span))
                    .collect();
                for (parameter, ty) in params.iter().zip(parameter_types.iter().copied()) {
                    closure_ownership.define(parameter.name.clone(), ty, parameter.span);
                }
                let result = self
                    .resolve_type(ret_ty, generics, *span)
                    .unwrap_or(TypeId(0));
                self.closure_count += 1;
                let previous_return = self.current_return.replace(result);
                self.check_block(body, generics, &mut closure_ownership, false);
                self.current_return = previous_return;
                Some(self.types.intern(CanonicalType::ClosureShape {
                    params: parameter_types,
                    result,
                }))
            }
            Expr::Call { name, args, span } => {
                self.check_named_call(name, &[], args, &[], generics, ownership, *span)
            }
            Expr::GenericApply { .. } => {
                self.diagnostic(
                    "E_NOT_CALLABLE",
                    "generic application must be invoked",
                    expression.span(),
                );
                None
            }
            Expr::Invoke {
                callee,
                args,
                named_args,
                span,
            } => {
                let (name, type_args) = match callee.as_ref() {
                    Expr::Ident(name, _) => (name.as_str(), Vec::new()),
                    Expr::GenericApply { target, args, .. } => match target.as_ref() {
                        Expr::Ident(name, _) => (name.as_str(), args.clone()),
                        _ => {
                            self.diagnostic(
                                "E_NOT_CALLABLE",
                                "unsupported generic call target",
                                *span,
                            );
                            return None;
                        }
                    },
                    _ => {
                        self.check_expr(callee, generics, ownership, false);
                        for argument in args {
                            self.check_expr(argument, generics, ownership, true);
                        }
                        return None;
                    }
                };
                self.check_named_call(
                    name, &type_args, args, named_args, generics, ownership, *span,
                )
            }
            Expr::Index { target, index, .. } => {
                let index_ty = self.check_expr(index, generics, ownership, false);
                if index_ty.is_some_and(|ty| {
                    !matches!(self.types.values[ty.0 as usize], CanonicalType::Integer(_))
                }) {
                    self.diagnostic(
                        "E_INDEX_TYPE",
                        format!(
                            "TypeError: index must be int, found {}",
                            self.type_name(index_ty.unwrap())
                        ),
                        index.span(),
                    );
                }
                let target = self.check_expr(target, generics, ownership, false)?;
                match self.types.values[target.0 as usize].clone() {
                    CanonicalType::Slice { element, .. }
                    | CanonicalType::Array { element, .. }
                    | CanonicalType::LegacyList(element)
                    | CanonicalType::Pointer {
                        pointee: element, ..
                    } => Some(element),
                    CanonicalType::LegacyStr => {
                        // Preserve the hosted/native compatibility contract: a
                        // legacy `str` indexes one byte but exposes the value as
                        // the legacy integer type. The executable place retains
                        // its physical u8 projection, so backends need no AST
                        // knowledge to perform the widening conversion.
                        Some(self.types.intern(CanonicalType::Integer(IntegerType::I32)))
                    }
                    CanonicalType::LegacyBytes | CanonicalType::LegacyBuffer => Some(
                        self.types
                            .intern(CanonicalType::Integer(IntegerType::LegacyInt)),
                    ),
                    CanonicalType::StringView => {
                        Some(self.types.intern(CanonicalType::Integer(IntegerType::U8)))
                    }
                    _ => {
                        self.diagnostic(
                            "E_NOT_INDEXABLE",
                            format!("TypeError: cannot index {}", self.type_name(target)),
                            expression.span(),
                        );
                        None
                    }
                }
            }
            Expr::MemberCall {
                object,
                property,
                args,
                span,
            } => {
                if let Expr::Ident(namespace, _) = object.as_ref() {
                    if let Some(module) = self.host_modules.get(namespace).cloned() {
                        return self
                            .check_host_call(&module, property, args, generics, ownership, *span);
                    }
                    if let Some(intrinsic) = HirIntrinsic::resolve(namespace, property) {
                        return self.check_intrinsic_call(
                            namespace, intrinsic, args, generics, ownership, *span,
                        );
                    }
                }
                if let Some(result) = self
                    .check_variant_constructor(object, property, args, generics, ownership, *span)
                {
                    Some(result)
                } else {
                    self.check_expr(object, generics, ownership, false)
                }
            }
            Expr::Member {
                object,
                property,
                span,
            } => self.check_field_access(object, property, generics, ownership, *span),
            Expr::Cast { expr, ty, span } => {
                let source = self.check_expr(expr, generics, ownership, false)?;
                let target = self.resolve_type(ty, generics, *span)?;
                let valid = match (
                    &self.types.values[source.0 as usize],
                    &self.types.values[target.0 as usize],
                ) {
                    (CanonicalType::Integer(_), CanonicalType::Integer(_)) => true,
                    (CanonicalType::Pointer { .. }, CanonicalType::Integer(IntegerType::Usize))
                    | (CanonicalType::Integer(IntegerType::Usize), CanonicalType::Pointer { .. }) => {
                        true
                    }
                    (CanonicalType::LegacyStr, CanonicalType::Integer(IntegerType::Usize))
                    | (CanonicalType::Integer(IntegerType::Usize), CanonicalType::LegacyStr)
                        if matches!(self.profile, Profile::I686None | Profile::I686User) =>
                    {
                        true
                    }
                    (left, right) => left == right,
                };
                if !valid {
                    self.diagnostic(
                        "E_INVALID_CAST",
                        "cast is outside the canonical checked conversion set",
                        *span,
                    );
                }
                Some(target)
            }
        }
    }

    fn check_host_call(
        &mut self,
        module: &str,
        member: &str,
        args: &[Expr],
        generics: &BTreeMap<String, TypeId>,
        ownership: &mut Ownership,
        span: Span,
    ) -> Option<TypeId> {
        if HostCapabilityId::resolve(module, member).is_none() {
            let alias = module.rsplit('.').next().unwrap_or(module);
            self.diagnostic(
                "E_HOST_CAPABILITY",
                format!("TypeError: module '{alias}' has no public member '{member}'"),
                span,
            );
            return None;
        }
        let export = dbyte_module::stdlib_exports(module)?
            .into_iter()
            .find(|(name, _)| name == member)
            .map(|(_, export)| export)?;
        let dbyte_module::StdlibExport::Function { params, ret } = export;
        if args.len() != params.len() {
            self.diagnostic(
                "E_ARGUMENT_COUNT",
                format!(
                    "resolved host capability expects {} arguments, found {}",
                    params.len(),
                    args.len()
                ),
                span,
            );
        }
        for (index, (argument, annotation)) in args.iter().zip(&params).enumerate() {
            let expected = if matches!(annotation, TypeAnnotation::Inferred) {
                None
            } else {
                self.resolve_type(annotation, generics, argument.span())
            };
            let actual = self.check_expr_expected(argument, expected, generics, ownership, true);
            if let (Some(actual), Some(expected)) = (actual, expected) {
                if !self.types_compatible(actual, expected) {
                    self.diagnostic(
                        "E_TYPE_MISMATCH",
                        format!(
                            "TypeError: expected {}, found {} (argument {})",
                            annotation,
                            self.type_name(actual),
                            index + 1
                        ),
                        argument.span(),
                    );
                }
            }
        }
        if matches!(ret, TypeAnnotation::Inferred) {
            Some(self.types.intern(CanonicalType::Unit))
        } else {
            self.resolve_type(&ret, generics, span)
        }
    }

    fn check_intrinsic_call(
        &mut self,
        namespace: &str,
        intrinsic: HirIntrinsic,
        args: &[Expr],
        generics: &BTreeMap<String, TypeId>,
        ownership: &mut Ownership,
        span: Span,
    ) -> Option<TypeId> {
        let admitted = intrinsic.is_runtime_view()
            || (intrinsic.is_runtime_host_memory()
                && matches!(self.profile, Profile::Hosted | Profile::Vm))
            || match self.profile {
                Profile::I686None => namespace == "native",
                Profile::I686User => namespace == "user",
                Profile::Hosted | Profile::Vm => false,
            };
        if !admitted
            || (!intrinsic.is_runtime_view()
                && !intrinsic.is_runtime_host_memory()
                && intrinsic.is_user() != (namespace == "user"))
        {
            self.diagnostic(
                "E_PROFILE_CAPABILITY",
                format!(
                    "profile {:?} does not admit `{namespace}` intrinsics",
                    self.profile
                ),
                span,
            );
        }

        let u8_ty = self.types.intern(CanonicalType::Integer(IntegerType::U8));
        let u16_ty = self.types.intern(CanonicalType::Integer(IntegerType::U16));
        let u32_ty = self.types.intern(CanonicalType::Integer(IntegerType::U32));
        let i32_ty = self.types.intern(CanonicalType::Integer(IntegerType::I32));
        let usize_ty = self
            .types
            .intern(CanonicalType::Integer(IntegerType::Usize));
        let unit = TypeId(0);
        if intrinsic.is_runtime_host_memory() {
            let required_function = match intrinsic {
                HirIntrinsic::RuntimeMemoryAlloc => "system_alloc_raw",
                HirIntrinsic::RuntimeMemoryResize => "system_resize_raw",
                HirIntrinsic::RuntimeMemoryFree => "system_free_raw",
                HirIntrinsic::RuntimeMemoryRead8 => "system_read8_raw",
                HirIntrinsic::RuntimeMemoryWrite8 => "system_write8_raw",
                _ => unreachable!(),
            };
            let admitted_function = self
                .current_function_source_name
                .as_deref()
                .is_some_and(|name| name.ends_with(required_function));
            if !admitted_function {
                self.diagnostic(
                    "E_RUNTIME_CAPABILITY_PRIVATE",
                    "host memory operations are reserved for the versioned runtime module",
                    span,
                );
            }
            let pointer = self.types.intern(CanonicalType::Pointer {
                mutable: true,
                pointee: u8_ty,
            });
            let bool_ty = self.types.intern(CanonicalType::Bool);
            let (expected, result) = match intrinsic {
                HirIntrinsic::RuntimeMemoryAlloc => (vec![Some(usize_ty), Some(usize_ty)], pointer),
                HirIntrinsic::RuntimeMemoryResize => (
                    vec![
                        Some(pointer),
                        Some(usize_ty),
                        Some(usize_ty),
                        Some(usize_ty),
                    ],
                    pointer,
                ),
                HirIntrinsic::RuntimeMemoryFree => {
                    (vec![Some(pointer), Some(usize_ty), Some(usize_ty)], bool_ty)
                }
                HirIntrinsic::RuntimeMemoryRead8 => (vec![Some(pointer), Some(usize_ty)], u8_ty),
                HirIntrinsic::RuntimeMemoryWrite8 => {
                    (vec![Some(pointer), Some(usize_ty), Some(u8_ty)], unit)
                }
                _ => unreachable!(),
            };
            if args.len() != expected.len() {
                self.diagnostic(
                    "E_ARGUMENT_COUNT",
                    format!(
                        "runtime memory operation expects {} arguments, found {}",
                        expected.len(),
                        args.len()
                    ),
                    span,
                );
                return None;
            }
            for (argument, expected) in args.iter().zip(expected) {
                self.check_expr_expected(argument, expected, generics, ownership, false);
            }
            return Some(result);
        }
        if intrinsic.is_runtime_view() {
            let required_function = match intrinsic {
                HirIntrinsic::RuntimeSliceFromRaw => "as_slice",
                HirIntrinsic::RuntimeMutSliceFromRaw => "as_mut_slice",
                HirIntrinsic::RuntimeStringViewFromRaw => "as_view",
                _ => unreachable!(),
            };
            let admitted_function = self
                .current_function_source_name
                .as_deref()
                .is_some_and(|name| name.ends_with(required_function));
            if !admitted_function {
                self.diagnostic(
                    "E_RUNTIME_CAPABILITY_PRIVATE",
                    "raw view construction is reserved for the versioned runtime module",
                    span,
                );
            }
            if args.len() != 2 {
                self.diagnostic(
                    "E_ARGUMENT_COUNT",
                    "raw view construction expects pointer and length",
                    span,
                );
                return None;
            }
            let pointer = self.check_expr(&args[0], generics, ownership, false);
            self.check_expr_expected(&args[1], Some(usize_ty), generics, ownership, false);
            let result = match (
                intrinsic,
                pointer.map(|ty| self.types.values[ty.0 as usize].clone()),
            ) {
                (
                    HirIntrinsic::RuntimeSliceFromRaw,
                    Some(CanonicalType::Pointer { pointee, .. }),
                ) => self.types.intern(CanonicalType::Slice {
                    mutable: false,
                    element: pointee,
                }),
                (
                    HirIntrinsic::RuntimeMutSliceFromRaw,
                    Some(CanonicalType::Pointer {
                        mutable: true,
                        pointee,
                    }),
                ) => self.types.intern(CanonicalType::Slice {
                    mutable: true,
                    element: pointee,
                }),
                (
                    HirIntrinsic::RuntimeStringViewFromRaw,
                    Some(CanonicalType::Pointer { pointee, .. }),
                ) if self.types.values[pointee.0 as usize]
                    == CanonicalType::Integer(IntegerType::U8) =>
                {
                    self.types.intern(CanonicalType::StringView)
                }
                _ => {
                    self.diagnostic(
                        "E_TYPE_MISMATCH",
                        "raw view pointer has an incompatible canonical type",
                        args[0].span(),
                    );
                    return None;
                }
            };
            return Some(result);
        }
        let (expected, result): (Vec<Option<TypeId>>, TypeId) = match intrinsic {
            HirIntrinsic::VolatileLoad8 => (vec![None], u8_ty),
            HirIntrinsic::VolatileStore8 => (vec![None, Some(u8_ty)], unit),
            HirIntrinsic::VolatileLoad16 => (vec![None], u16_ty),
            HirIntrinsic::VolatileStore16 => (vec![None, Some(u16_ty)], unit),
            HirIntrinsic::VolatileLoad32 => (vec![None], u32_ty),
            HirIntrinsic::VolatileStore32 => (vec![None, Some(u32_ty)], unit),
            HirIntrinsic::IoOut8 => (vec![Some(u16_ty), Some(u8_ty)], unit),
            HirIntrinsic::IoIn8 => (vec![Some(u16_ty)], u8_ty),
            HirIntrinsic::IoOut16 => (vec![Some(u16_ty), Some(u16_ty)], unit),
            HirIntrinsic::IoIn16 => (vec![Some(u16_ty)], u16_ty),
            HirIntrinsic::CpuHlt
            | HirIntrinsic::CpuCli
            | HirIntrinsic::CpuSti
            | HirIntrinsic::CpuInt3 => (Vec::new(), unit),
            HirIntrinsic::CpuReadEflags | HirIntrinsic::CpuReadCr0 => (Vec::new(), u32_ty),
            HirIntrinsic::CpuWriteCr0 => (vec![Some(u32_ty)], unit),
            HirIntrinsic::CpuReadCr2 | HirIntrinsic::CpuReadCr3 => (Vec::new(), usize_ty),
            HirIntrinsic::CpuWriteCr3 | HirIntrinsic::InterruptResume => {
                (vec![Some(usize_ty)], unit)
            }
            HirIntrinsic::CpuInvlpg | HirIntrinsic::CpuLgdt | HirIntrinsic::CpuLidt => {
                (vec![None], unit)
            }
            HirIntrinsic::CpuLtr => (vec![Some(u16_ty)], unit),
            HirIntrinsic::CpuReloadSegments => (vec![Some(u16_ty), Some(u16_ty)], unit),
            HirIntrinsic::TaskBlockTrap => (vec![Some(u32_ty), Some(usize_ty)], i32_ty),
            HirIntrinsic::KernelImageStart
            | HirIntrinsic::KernelImageEnd
            | HirIntrinsic::BootstrapStackStart
            | HirIntrinsic::BootstrapStackEnd
            | HirIntrinsic::TaskTrampolineAddress => (Vec::new(), usize_ty),
            HirIntrinsic::UserSyscall0 => (vec![Some(u32_ty)], i32_ty),
            HirIntrinsic::UserSyscall1 => (vec![Some(u32_ty), None], i32_ty),
            HirIntrinsic::UserSyscall2 => (vec![Some(u32_ty), None, None], i32_ty),
            HirIntrinsic::UserSyscall3 => (vec![Some(u32_ty), None, None, None], i32_ty),
            HirIntrinsic::RuntimeSliceFromRaw
            | HirIntrinsic::RuntimeMutSliceFromRaw
            | HirIntrinsic::RuntimeStringViewFromRaw
            | HirIntrinsic::RuntimeMemoryAlloc
            | HirIntrinsic::RuntimeMemoryResize
            | HirIntrinsic::RuntimeMemoryFree
            | HirIntrinsic::RuntimeMemoryRead8
            | HirIntrinsic::RuntimeMemoryWrite8 => unreachable!(),
        };
        if args.len() != expected.len() {
            self.diagnostic(
                "E_ARGUMENT_COUNT",
                format!(
                    "resolved intrinsic expects {} arguments, found {}",
                    expected.len(),
                    args.len()
                ),
                span,
            );
        }
        for (index, argument) in args.iter().enumerate() {
            let expected_ty = expected.get(index).copied().flatten();
            let actual = self.check_expr_expected(argument, expected_ty, generics, ownership, true);
            if let (Some(actual), Some(expected_ty)) = (actual, expected_ty) {
                if !self.types_compatible(actual, expected_ty) {
                    self.diagnostic(
                        "E_TYPE_MISMATCH",
                        format!(
                            "intrinsic argument {} has the wrong canonical type",
                            index + 1
                        ),
                        argument.span(),
                    );
                }
            }
            if expected_ty.is_none()
                && actual.is_some_and(|ty| {
                    !matches!(
                        self.types.values[ty.0 as usize],
                        CanonicalType::Integer(_)
                            | CanonicalType::Pointer { .. }
                            | CanonicalType::LegacyStr
                    )
                })
            {
                self.diagnostic(
                    "E_TYPE_MISMATCH",
                    format!(
                        "intrinsic argument {} must be a pointer-sized scalar",
                        index + 1
                    ),
                    argument.span(),
                );
            }
        }
        if intrinsic == HirIntrinsic::InterruptResume
            && self
                .current_interrupt
                .is_some_and(dbyte_ast::InterruptClass::has_error_code)
        {
            self.diagnostic(
                "E_PROFILE_CAPABILITY",
                "interrupt_resume is forbidden in error-code interrupt functions",
                span,
            );
        }
        if intrinsic == HirIntrinsic::TaskBlockTrap && self.current_interrupt.is_some() {
            self.diagnostic(
                "E_PROFILE_CAPABILITY",
                "task_block_trap is only valid in normal functions",
                span,
            );
        }
        if intrinsic == HirIntrinsic::TaskBlockTrap
            && args
                .first()
                .and_then(constant_integer_expression)
                .is_none_or(|kind| !(1..=3).contains(&kind))
        {
            self.diagnostic(
                "E_INTRINSIC_CONSTANT",
                "task_block_trap kind must be the constant 1, 2, or 3",
                args.first().map_or(span, Expr::span),
            );
        }
        if matches!(
            intrinsic,
            HirIntrinsic::CpuLtr | HirIntrinsic::CpuReloadSegments
        ) && args
            .iter()
            .take(expected.len())
            .any(|argument| constant_integer_expression(argument).is_none())
        {
            self.diagnostic(
                "E_INTRINSIC_CONSTANT",
                "selector operands must be compile-time constants",
                span,
            );
        }
        Some(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn check_named_call(
        &mut self,
        name: &str,
        type_args: &[TypeAnnotation],
        args: &[Expr],
        named_args: &[dbyte_ast::NamedArgument],
        generics: &BTreeMap<String, TypeId>,
        ownership: &mut Ownership,
        span: Span,
    ) -> Option<TypeId> {
        if name == "interrupt_address" || name == "task_address" {
            if self.profile == Profile::I686User {
                self.diagnostic(
                    "E_PROFILE_CAPABILITY",
                    format!("i686 user profile does not admit `{name}`"),
                    span,
                );
                return None;
            }
            if !type_args.is_empty() || !named_args.is_empty() {
                self.diagnostic(
                    "E_ARGUMENT_COUNT",
                    format!("{name} does not accept type or named arguments"),
                    span,
                );
                return None;
            }
            let resolved = if name == "interrupt_address" {
                if args.len() != 2 {
                    self.diagnostic(
                        "E_ARGUMENT_COUNT",
                        "interrupt_address expects vector and handler",
                        span,
                    );
                    return None;
                }
                let Some(vector) = constant_integer_expression(&args[0]) else {
                    self.diagnostic(
                        "E_INTERRUPT_VECTOR",
                        "interrupt vector must be a compile-time integer",
                        args[0].span(),
                    );
                    return None;
                };
                let Ok(vector) = u8::try_from(vector) else {
                    self.diagnostic(
                        "E_INTERRUPT_VECTOR",
                        "interrupt vector must be in 0..255",
                        args[0].span(),
                    );
                    return None;
                };
                let Expr::Ident(handler, handler_span) = &args[1] else {
                    self.diagnostic(
                        "E_INTERRUPT_HANDLER",
                        "interrupt_address requires a resolved interrupt function",
                        args[1].span(),
                    );
                    return None;
                };
                let Some(function) = self.functions.iter().find(|item| item.name == *handler)
                else {
                    self.diagnostic(
                        "E_UNKNOWN_SYMBOL",
                        format!("unknown interrupt handler `{handler}`"),
                        *handler_span,
                    );
                    return None;
                };
                let Some(class) = function.interrupt else {
                    self.diagnostic(
                        "E_INTERRUPT_HANDLER",
                        format!("`{handler}` is not an interrupt function"),
                        *handler_span,
                    );
                    return None;
                };
                let requires_error = matches!(vector, 8 | 10 | 11 | 12 | 13 | 14 | 17);
                if class.has_error_code() != requires_error
                    || (vector == 128 && !class.cpl3_transition())
                {
                    self.diagnostic(
                        "E_INTERRUPT_CLASS_MISMATCH",
                        format!("vector {vector} is incompatible with handler class {class}"),
                        span,
                    );
                    return None;
                }
                ResolvedAddress::Interrupt {
                    vector,
                    handler: function.symbol,
                    class,
                }
            } else {
                if args.len() != 1 {
                    self.diagnostic(
                        "E_ARGUMENT_COUNT",
                        "task_address expects one function",
                        span,
                    );
                    return None;
                }
                let Expr::Ident(function_name, function_span) = &args[0] else {
                    self.diagnostic(
                        "E_TASK_ADDRESS",
                        "task_address requires a resolved function symbol",
                        args[0].span(),
                    );
                    return None;
                };
                let Some(function) = self
                    .functions
                    .iter()
                    .find(|item| item.name == *function_name)
                else {
                    self.diagnostic(
                        "E_UNKNOWN_SYMBOL",
                        format!("unknown task function `{function_name}`"),
                        *function_span,
                    );
                    return None;
                };
                if !function.public
                    || function.interrupt.is_some()
                    || !function.params.is_empty()
                    || function.name == "kernel_main"
                    || !matches!(function.result, TypeAnnotation::Int | TypeAnnotation::I32)
                {
                    self.diagnostic(
                        "E_TASK_ADDRESS",
                        "task target must be a public non-interrupt fn() -> int",
                        *function_span,
                    );
                    return None;
                }
                ResolvedAddress::Task {
                    function: function.symbol,
                }
            };
            self.resolved_addresses
                .insert((span.line, span.col), resolved);
            return Some(
                self.types
                    .intern(CanonicalType::Integer(IntegerType::Usize)),
            );
        }
        if name == "size_of" || name == "align_of" {
            if args.len() != 1 || !type_args.is_empty() || !named_args.is_empty() {
                self.diagnostic(
                    "E_ARGUMENT_COUNT",
                    format!("{name} expects exactly one type argument expression"),
                    span,
                );
            }
            if let Some(annotation) = args.first().and_then(type_expression_annotation) {
                if let Some(ty) = self.resolve_type(&annotation, generics, span) {
                    if !self.type_contains_generic(ty, &mut BTreeSet::new()) {
                        if let Err(error) = layout_of(
                            &HirProgram {
                                module: ModuleId(0),
                                profile: self.profile,
                                layout: self.layout,
                                types: self.types.values.clone(),
                                definitions: self.definitions.clone(),
                                functions: self.functions.clone(),
                                globals: self.globals.clone(),
                                specializations: BTreeSet::new(),
                                generic_capabilities: BTreeMap::new(),
                                closure_count: self.closure_count,
                                expressions: Vec::new(),
                                allocator_origins: Vec::new(),
                                borrow_origins: Vec::new(),
                                function_allocator_origins: BTreeMap::new(),
                                function_borrow_origins: BTreeMap::new(),
                                cleanup_edges: Vec::new(),
                                resolved_addresses: BTreeMap::new(),
                                interface_fingerprint: 0,
                            },
                            ty,
                        ) {
                            self.diagnostics.push(Diagnostic { span, ..error });
                        }
                    }
                }
            } else {
                self.diagnostic(
                    "E_TYPE_EXPECTED",
                    format!("{name} expects a canonical type name"),
                    span,
                );
            }
            return Some(
                self.types
                    .intern(CanonicalType::Integer(IntegerType::Usize)),
            );
        }
        if name == "print" {
            if !type_args.is_empty() || !named_args.is_empty() {
                self.diagnostic(
                    "E_ARGUMENT_COUNT",
                    "print does not accept type or named arguments",
                    span,
                );
            }
            for argument in args {
                self.check_expr(argument, generics, ownership, false);
            }
            return Some(TypeId(0));
        }
        if name == "len" {
            if args.len() != 1 || !type_args.is_empty() || !named_args.is_empty() {
                self.diagnostic("E_ARGUMENT_COUNT", "len expects one argument", span);
            }
            let argument = args
                .first()
                .and_then(|argument| self.check_expr(argument, generics, ownership, false));
            let legacy = argument.is_some_and(|ty| {
                matches!(
                    self.types.values[ty.0 as usize],
                    CanonicalType::LegacyStr
                        | CanonicalType::LegacyBytes
                        | CanonicalType::LegacyBuffer
                        | CanonicalType::LegacyList(_)
                )
            });
            return Some(self.types.intern(CanonicalType::Integer(if legacy {
                IntegerType::LegacyInt
            } else {
                IntegerType::Usize
            })));
        }
        let local_callable = ownership
            .lookup_mut(name)
            .map(|binding| (binding.ty, binding.moved));
        if let Some((callable, moved)) = local_callable {
            if moved {
                self.diagnostic(
                    "E_USE_AFTER_MOVE",
                    format!("callable `{name}` was moved"),
                    span,
                );
                return None;
            }
            let signature = match self.types.values[callable.0 as usize].clone() {
                CanonicalType::Function { params, result, .. }
                | CanonicalType::ClosureShape { params, result } => Some((params, result)),
                _ => None,
            };
            if let Some((params, result)) = signature {
                if !type_args.is_empty() || !named_args.is_empty() || args.len() != params.len() {
                    self.diagnostic(
                        "E_ARGUMENT_COUNT",
                        format!("callable `{name}` expects {} arguments", params.len()),
                        span,
                    );
                }
                for (index, argument) in args.iter().enumerate() {
                    let expected = params.get(index).copied();
                    let actual =
                        self.check_expr_expected(argument, expected, generics, ownership, true);
                    if let (Some(actual), Some(expected)) = (actual, expected) {
                        if !self.types_compatible(actual, expected) {
                            self.diagnostic(
                                "E_TYPE_MISMATCH",
                                format!(
                                    "argument {} to `{name}` has the wrong canonical type",
                                    index + 1
                                ),
                                argument.span(),
                            );
                        }
                    }
                }
                return Some(result);
            }
        }
        if let Some(definition) = self
            .definitions
            .iter()
            .find(|definition| definition.name == name)
            .cloned()
        {
            let fields = match &definition.kind {
                TypeDefKind::Struct { fields, .. } => fields.clone(),
                TypeDefKind::Enum { .. } => {
                    self.diagnostic(
                        "E_NOT_CALLABLE",
                        "enum type requires a qualified variant",
                        span,
                    );
                    return None;
                }
            };
            if !args.is_empty() {
                self.diagnostic(
                    "E_STRUCT_INITIALIZER",
                    format!("struct `{name}` accepts named fields only"),
                    span,
                );
            }
            let annotation = if type_args.is_empty() {
                TypeAnnotation::Named(name.to_owned())
            } else {
                TypeAnnotation::Generic {
                    base: name.to_owned(),
                    args: type_args.to_vec(),
                }
            };
            let result = self.resolve_type(&annotation, generics, span);
            let resolved_args: Vec<_> = type_args
                .iter()
                .filter_map(|argument| self.resolve_type(argument, generics, span))
                .collect();
            let substitutions: BTreeMap<_, _> = definition
                .generic_params
                .iter()
                .cloned()
                .zip(resolved_args)
                .collect();
            let mut initialized = BTreeSet::new();
            for argument in named_args {
                if !initialized.insert(argument.name.clone()) {
                    self.diagnostic(
                        "E_DUPLICATE_FIELD_INITIALIZER",
                        format!("field `{}` is initialized more than once", argument.name),
                        argument.span,
                    );
                    self.check_expr(&argument.value, generics, ownership, true);
                    continue;
                }
                let Some(field) = fields.iter().find(|field| field.name == argument.name) else {
                    self.diagnostic(
                        "E_UNKNOWN_FIELD",
                        format!("struct `{name}` has no field `{}`", argument.name),
                        argument.span,
                    );
                    self.check_expr(&argument.value, generics, ownership, true);
                    continue;
                };
                let expected = self.resolve_type(&field.annotation, &substitutions, field.span);
                let actual =
                    self.check_expr_expected(&argument.value, expected, generics, ownership, true);
                if let (Some(actual), Some(expected)) = (actual, expected) {
                    if !self.types_compatible(actual, expected) {
                        self.diagnostic(
                            "E_TYPE_MISMATCH",
                            format!("field `{}` has the wrong canonical type", field.name),
                            argument.span,
                        );
                    }
                }
            }
            for field in &fields {
                if !initialized.contains(&field.name) {
                    self.diagnostic(
                        "E_MISSING_FIELD",
                        format!("struct `{name}` is missing field `{}`", field.name),
                        span,
                    );
                }
            }
            return result;
        }
        if let Some(function) = self
            .functions
            .iter()
            .find(|function| function.name == name)
            .cloned()
        {
            if function.kind == FunctionKind::Drop {
                self.diagnostic(
                    "E_DROP_DIRECT_CALL",
                    "drop functions can only be invoked by compiler destruction glue",
                    span,
                );
                for argument in args {
                    self.check_expr(argument, generics, ownership, false);
                }
                return None;
            }
            if type_args.len() != function.generic_params.len() {
                self.diagnostic(
                    "E_GENERIC_ARGUMENT_COUNT",
                    format!(
                        "function `{name}` expects {} type arguments, found {}",
                        function.generic_params.len(),
                        type_args.len()
                    ),
                    span,
                );
            }
            let resolved_args: Vec<_> = type_args
                .iter()
                .filter_map(|arg| self.resolve_type(arg, generics, span))
                .collect();
            if let Some(definition) = function.generic {
                if self.current_generic == Some(definition) {
                    let same_type_recursion =
                        resolved_args.iter().enumerate().all(|(index, argument)| {
                            matches!(
                                self.types.values[argument.0 as usize],
                                CanonicalType::GenericParam {
                                    owner,
                                    index: parameter
                                } if owner == definition && parameter == index as u32
                            )
                        });
                    if !same_type_recursion {
                        self.diagnostic(
                            "E_GENERIC_INSTANTIATION_RECURSION",
                            format!(
                                "recursive call to `{name}` expands its canonical type arguments"
                            ),
                            span,
                        );
                    }
                }
                self.specializations.insert(SpecializationKey {
                    definition,
                    arguments: resolved_args.clone(),
                });
            }
            if args.len() != function.params.len() || !named_args.is_empty() {
                let source_name = display_function_name(name);
                self.diagnostic(
                    "E_ARGUMENT_COUNT",
                    format!(
                        "function `{source_name}` expects {} args, got {}",
                        function.params.len(),
                        args.len()
                    ),
                    span,
                );
            }
            let substitutions: BTreeMap<_, _> = function
                .generic_params
                .iter()
                .cloned()
                .zip(resolved_args)
                .collect();
            let result = self.resolve_type(&function.result, &substitutions, span);
            let temporary_argument_borrows = result.is_some_and(|result| {
                !matches!(
                    self.types.values[result.0 as usize],
                    CanonicalType::Borrow { .. }
                        | CanonicalType::Slice { .. }
                        | CanonicalType::StringView
                )
            });
            if temporary_argument_borrows {
                ownership.push_scope();
            }
            for (index, argument) in args.iter().enumerate() {
                let expected = function.params.get(index).and_then(|annotation| {
                    self.resolve_type(annotation, &substitutions, function.span)
                });
                let actual =
                    self.check_expr_expected(argument, expected, generics, ownership, true);
                if let (Some(actual), Some(expected)) = (actual, expected) {
                    if !self.types_compatible(actual, expected) {
                        self.diagnostic(
                            "E_TYPE_MISMATCH",
                            format!(
                                "argument {} to `{name}` has the wrong canonical type",
                                index + 1
                            ),
                            argument.span(),
                        );
                    }
                }
            }
            if temporary_argument_borrows {
                ownership.pop_scope();
            }
            return result;
        }
        for argument in args {
            self.check_expr(argument, generics, ownership, true);
        }
        self.diagnostic(
            "E_UNKNOWN_SYMBOL",
            format!("undefined function `{name}`"),
            span,
        );
        None
    }

    fn check_variant_constructor(
        &mut self,
        object: &Expr,
        variant_name: &str,
        args: &[Expr],
        generics: &BTreeMap<String, TypeId>,
        ownership: &mut Ownership,
        span: Span,
    ) -> Option<TypeId> {
        let (base, type_args) = match object {
            Expr::Ident(base, _) => (base.as_str(), Vec::new()),
            Expr::GenericApply { target, args, .. } => {
                let Expr::Ident(base, _) = target.as_ref() else {
                    return None;
                };
                (base.as_str(), args.clone())
            }
            _ => return None,
        };
        let definition = self
            .definitions
            .iter()
            .find(|definition| definition.name == base)
            .cloned()?;
        let TypeDefKind::Enum { variants } = &definition.kind else {
            return None;
        };
        if type_args.len() != definition.generic_params.len() {
            self.diagnostic(
                "E_GENERIC_ARGUMENT_COUNT",
                format!(
                    "enum `{base}` expects {} type arguments, found {}",
                    definition.generic_params.len(),
                    type_args.len()
                ),
                span,
            );
            for argument in args {
                self.check_expr(argument, generics, ownership, true);
            }
            return None;
        }
        let Some(variant) = variants
            .iter()
            .find(|variant| variant.name == variant_name)
            .cloned()
        else {
            self.diagnostic(
                "E_UNKNOWN_VARIANT",
                format!("enum `{base}` has no variant `{variant_name}`"),
                span,
            );
            return None;
        };
        let resolved_args: Vec<_> = type_args
            .iter()
            .filter_map(|argument| self.resolve_type(argument, generics, span))
            .collect();
        let substitutions: BTreeMap<_, _> = definition
            .generic_params
            .iter()
            .cloned()
            .zip(resolved_args)
            .collect();
        if args.len() != variant.payload.len() {
            self.diagnostic(
                "E_VARIANT_ARITY",
                format!(
                    "variant `{base}.{variant_name}` expects {} payload values, found {}",
                    variant.payload.len(),
                    args.len()
                ),
                span,
            );
        }
        for (index, argument) in args.iter().enumerate() {
            let expected = variant
                .payload
                .get(index)
                .and_then(|annotation| self.resolve_type(annotation, &substitutions, variant.span));
            let actual = self.check_expr_expected(argument, expected, generics, ownership, true);
            if let (Some(actual), Some(expected)) = (actual, expected) {
                if !self.types_compatible(actual, expected) {
                    self.diagnostic(
                        "E_TYPE_MISMATCH",
                        format!(
                            "payload {} of `{base}.{variant_name}` has the wrong type",
                            index + 1
                        ),
                        argument.span(),
                    );
                }
            }
        }
        let annotation = if type_args.is_empty() {
            TypeAnnotation::Named(base.to_owned())
        } else {
            TypeAnnotation::Generic {
                base: base.to_owned(),
                args: type_args,
            }
        };
        self.resolve_type(&annotation, generics, span)
    }

    fn check_field_access(
        &mut self,
        object: &Expr,
        property: &str,
        generics: &BTreeMap<String, TypeId>,
        ownership: &mut Ownership,
        span: Span,
    ) -> Option<TypeId> {
        let object_type = self.check_expr(object, generics, ownership, false)?;
        let (symbol, args) = match self.types.values[object_type.0 as usize].clone() {
            CanonicalType::Struct { symbol, args } => (symbol, args),
            CanonicalType::Pointer { pointee, .. } | CanonicalType::Borrow { pointee, .. } => {
                match self.types.values[pointee.0 as usize].clone() {
                    CanonicalType::Struct { symbol, args } => (symbol, args),
                    _ => {
                        self.diagnostic(
                            "E_FIELD_BASE_TYPE",
                            "field access requires a struct lvalue",
                            span,
                        );
                        return None;
                    }
                }
            }
            _ => {
                self.diagnostic(
                    "E_FIELD_BASE_TYPE",
                    "field access requires a struct lvalue",
                    span,
                );
                return None;
            }
        };
        let definition = self
            .definitions
            .iter()
            .find(|definition| definition.symbol == symbol)
            .cloned()?;
        let TypeDefKind::Struct { fields, .. } = definition.kind else {
            return None;
        };
        let Some(field) = fields.iter().find(|field| field.name == property) else {
            self.diagnostic(
                "E_UNKNOWN_FIELD",
                format!("struct `{}` has no field `{property}`", definition.name),
                span,
            );
            return None;
        };
        let substitutions: BTreeMap<_, _> = definition
            .generic_params
            .iter()
            .cloned()
            .zip(args)
            .collect();
        self.resolve_type(&field.annotation, &substitutions, field.span)
    }

    fn check_captures(
        &mut self,
        captures: &[dbyte_ast::Capture],
        ownership: &mut Ownership,
        span: Span,
    ) {
        let mut seen = BTreeSet::new();
        for capture in captures {
            if !seen.insert(capture.name.clone()) {
                self.diagnostic(
                    "E_DUPLICATE_CAPTURE",
                    format!("duplicate capture `{}`", capture.name),
                    capture.span,
                );
                continue;
            }
            let Some(binding) = ownership.lookup_mut(&capture.name) else {
                self.diagnostic(
                    "E_UNKNOWN_SYMBOL",
                    format!("unknown capture `{}`", capture.name),
                    capture.span,
                );
                continue;
            };
            match capture.mode {
                CaptureMode::Move => {
                    if binding.moved || binding.immut_borrows != 0 || binding.mut_borrowed {
                        self.diagnostic(
                            "E_CAPTURE_CONFLICT",
                            format!("cannot move capture `{}`", capture.name),
                            capture.span,
                        );
                    } else if !self.is_copy(binding.ty) {
                        binding.moved = true;
                    }
                }
                CaptureMode::Borrow => {
                    if binding.mut_borrowed {
                        self.diagnostic(
                            "E_CAPTURE_CONFLICT",
                            format!("capture `{}` conflicts with mutable borrow", capture.name),
                            capture.span,
                        );
                    } else {
                        binding.immut_borrows += 1;
                        ownership.record_borrow(capture.name.clone(), false);
                    }
                }
                CaptureMode::BorrowMut => {
                    if binding.mut_borrowed || binding.immut_borrows != 0 {
                        self.diagnostic(
                            "E_CAPTURE_CONFLICT",
                            format!("mutable capture `{}` is not exclusive", capture.name),
                            capture.span,
                        );
                    } else {
                        binding.mut_borrowed = true;
                        ownership.record_borrow(capture.name.clone(), true);
                    }
                }
            }
        }
        if captures.is_empty() && span == Span::zero() {
            unreachable!();
        }
    }

    fn capture_environment(
        &mut self,
        captures: &[dbyte_ast::Capture],
        ownership: &mut Ownership,
    ) -> Ownership {
        let mut environment = Ownership::default();
        environment.push_scope();
        for capture in captures {
            if let Some(binding) = ownership.lookup_mut(&capture.name) {
                environment.define(capture.name.clone(), binding.ty, capture.span);
            }
        }
        environment
    }

    fn check_match(
        &mut self,
        mode: MatchMode,
        value: &Expr,
        arms: &[dbyte_ast::MatchArm],
        generics: &BTreeMap<String, TypeId>,
        ownership: &mut Ownership,
        span: Span,
    ) {
        let value_type = self.check_expr(value, generics, ownership, mode == MatchMode::Move);
        let Some(value_type) = value_type else { return };
        let CanonicalType::Enum { symbol, args } = self.types.values[value_type.0 as usize].clone()
        else {
            self.diagnostic("E_MATCH_TYPE", "match value must be an enum", span);
            return;
        };
        let Some(definition) = self
            .definitions
            .iter()
            .find(|definition| definition.symbol == symbol)
            .cloned()
        else {
            return;
        };
        let TypeDefKind::Enum { variants } = definition.kind else {
            return;
        };
        let substitutions: BTreeMap<_, _> = definition
            .generic_params
            .iter()
            .cloned()
            .zip(args)
            .collect();
        let mut covered = BTreeSet::new();
        let mut wildcard = false;
        for arm in arms {
            match &arm.pattern {
                Pattern::Wildcard(_) => wildcard = true,
                Pattern::Variant {
                    path,
                    bindings,
                    span,
                } => {
                    let variant_name = path.rsplit('.').next().unwrap_or(path);
                    if let Some(variant) =
                        variants.iter().find(|variant| variant.name == variant_name)
                    {
                        if !covered.insert(variant.id) {
                            self.diagnostic(
                                "E_DUPLICATE_PATTERN",
                                format!("variant `{variant_name}` is matched more than once"),
                                *span,
                            );
                        }
                        if bindings.len() != variant.payload.len() {
                            self.diagnostic(
                                "E_PATTERN_ARITY",
                                format!(
                                    "variant `{variant_name}` has {} payload values, pattern binds {}",
                                    variant.payload.len(),
                                    bindings.len()
                                ),
                                *span,
                            );
                        }
                        ownership.push_scope();
                        for (binding, annotation) in bindings.iter().zip(&variant.payload) {
                            if let Some(payload) =
                                self.resolve_type(annotation, &substitutions, *span)
                            {
                                let ty = match mode {
                                    MatchMode::Move => payload,
                                    MatchMode::Borrow => self.types.intern(CanonicalType::Borrow {
                                        mutable: false,
                                        pointee: payload,
                                    }),
                                    MatchMode::BorrowMut => {
                                        self.types.intern(CanonicalType::Borrow {
                                            mutable: true,
                                            pointee: payload,
                                        })
                                    }
                                };
                                ownership.define(binding.clone(), ty, *span);
                            }
                        }
                        self.check_block(&arm.body, generics, ownership, false);
                        ownership.pop_scope();
                    } else {
                        self.diagnostic(
                            "E_UNKNOWN_VARIANT",
                            format!("unknown variant `{path}`"),
                            *span,
                        );
                    }
                }
            }
            if matches!(arm.pattern, Pattern::Wildcard(_)) {
                self.check_block(&arm.body, generics, ownership, false);
            }
        }
        if !wildcard && covered.len() != variants.len() {
            self.diagnostic(
                "E_NON_EXHAUSTIVE_MATCH",
                "match does not cover every variant",
                span,
            );
        }
    }

    fn pointer_widening(&self, actual: TypeId, expected: TypeId) -> bool {
        matches!(
            (
                &self.types.values[actual.0 as usize],
                &self.types.values[expected.0 as usize]
            ),
            (
                CanonicalType::Pointer {
                    mutable: true,
                    pointee: actual
                },
                CanonicalType::Pointer {
                    mutable: false,
                    pointee: expected
                }
            ) if actual == expected
        )
    }

    fn types_compatible(&self, actual: TypeId, expected: TypeId) -> bool {
        actual == expected
            || self.pointer_widening(actual, expected)
            || matches!(
                (
                    &self.types.values[actual.0 as usize],
                    &self.types.values[expected.0 as usize]
                ),
                (
                    CanonicalType::Integer(IntegerType::I32),
                    CanonicalType::Integer(IntegerType::LegacyInt)
                ) | (
                    CanonicalType::Integer(IntegerType::LegacyInt),
                    CanonicalType::Integer(IntegerType::I32)
                ) | (
                    CanonicalType::Integer(IntegerType::U8 | IntegerType::U16),
                    CanonicalType::Integer(IntegerType::LegacyInt)
                ) | (
                    CanonicalType::Integer(IntegerType::LegacyInt),
                    CanonicalType::Integer(IntegerType::U8 | IntegerType::U16)
                )
            )
    }

    fn validate_specializations(&mut self) {
        for specialization in self.specializations.clone() {
            for (index, argument) in specialization.arguments.iter().copied().enumerate() {
                let capabilities = self
                    .generic_capabilities
                    .get(&(specialization.definition, index as u32))
                    .cloned()
                    .unwrap_or_default();
                for capability in capabilities {
                    let satisfied = match capability {
                        BuiltinCapability::Copy => self.is_copy(argument),
                        BuiltinCapability::Numeric => matches!(
                            self.types.values[argument.0 as usize],
                            CanonicalType::Integer(_) | CanonicalType::Float
                        ),
                        BuiltinCapability::Equality => !matches!(
                            self.types.values[argument.0 as usize],
                            CanonicalType::ClosureShape { .. }
                                | CanonicalType::LegacyBuffer
                                | CanonicalType::LegacyList(_)
                        ),
                        BuiltinCapability::Ordering => matches!(
                            self.types.values[argument.0 as usize],
                            CanonicalType::Integer(_) | CanonicalType::Float
                        ),
                    };
                    if !satisfied {
                        self.diagnostic(
                            "E_GENERIC_CAPABILITY",
                            format!(
                                "specialization g{} argument {} does not satisfy {capability:?}",
                                specialization.definition.0,
                                index + 1
                            ),
                            Span::zero(),
                        );
                    }
                }
            }
        }
    }

    fn is_copy(&self, ty: TypeId) -> bool {
        self.is_copy_inner(ty, &mut BTreeSet::new())
    }

    fn type_name(&self, ty: TypeId) -> String {
        match &self.types.values[ty.0 as usize] {
            CanonicalType::Unit => "unit".into(),
            CanonicalType::Integer(IntegerType::LegacyInt) => "int".into(),
            CanonicalType::Integer(IntegerType::I32) => "int".into(),
            CanonicalType::Integer(integer) => format!("{integer:?}").to_lowercase(),
            CanonicalType::Float => "float".into(),
            CanonicalType::Bool => "bool".into(),
            CanonicalType::LegacyStr => "str".into(),
            CanonicalType::LegacyBytes => "bytes".into(),
            CanonicalType::LegacyBuffer => "buffer".into(),
            CanonicalType::LegacyList(element) => {
                format!("list[{}]", self.type_name(*element))
            }
            CanonicalType::Pointer { .. } => "ptr".into(),
            CanonicalType::Borrow { .. } => "borrow".into(),
            CanonicalType::Slice { .. } => "slice".into(),
            CanonicalType::StringView => "str".into(),
            CanonicalType::Array { .. } => "array".into(),
            CanonicalType::Struct { symbol, .. } | CanonicalType::Enum { symbol, .. } => self
                .definitions
                .iter()
                .find(|definition| definition.symbol == *symbol)
                .map_or_else(|| "aggregate".into(), |definition| definition.name.clone()),
            CanonicalType::Function { .. } => "fn".into(),
            CanonicalType::ClosureShape { .. } => "closure".into(),
            CanonicalType::GenericParam { .. } => "generic".into(),
        }
    }

    fn type_contains_generic(&self, ty: TypeId, active: &mut BTreeSet<TypeId>) -> bool {
        if !active.insert(ty) {
            return false;
        }
        let result = match &self.types.values[ty.0 as usize] {
            CanonicalType::GenericParam { .. } => true,
            CanonicalType::LegacyList(element)
            | CanonicalType::Pointer {
                pointee: element, ..
            }
            | CanonicalType::Borrow {
                pointee: element, ..
            }
            | CanonicalType::Slice { element, .. }
            | CanonicalType::Array { element, .. } => self.type_contains_generic(*element, active),
            CanonicalType::Struct { args, .. } | CanonicalType::Enum { args, .. } => args
                .iter()
                .copied()
                .any(|argument| self.type_contains_generic(argument, active)),
            CanonicalType::Function { params, result, .. }
            | CanonicalType::ClosureShape { params, result } => params
                .iter()
                .copied()
                .chain(std::iter::once(*result))
                .any(|part| self.type_contains_generic(part, active)),
            _ => false,
        };
        active.remove(&ty);
        result
    }

    fn is_copy_inner(&self, ty: TypeId, active: &mut BTreeSet<TypeId>) -> bool {
        if !active.insert(ty) {
            return false;
        }
        match &self.types.values[ty.0 as usize] {
            CanonicalType::Unit
            | CanonicalType::Integer(_)
            | CanonicalType::Float
            | CanonicalType::Bool
            | CanonicalType::Pointer { .. }
            | CanonicalType::Borrow { .. }
            | CanonicalType::Function { .. }
            | CanonicalType::LegacyStr
            | CanonicalType::LegacyBytes
            | CanonicalType::LegacyBuffer
            | CanonicalType::LegacyList(_) => true,
            CanonicalType::Array { element, .. } => self.is_copy_inner(*element, active),
            CanonicalType::Struct { symbol, args } | CanonicalType::Enum { symbol, args } => {
                if self.functions.iter().any(|function| {
                    function.kind == FunctionKind::Drop && function.drop_target == Some(*symbol)
                }) {
                    return false;
                }
                let Some(definition) = self
                    .definitions
                    .iter()
                    .find(|definition| definition.symbol == *symbol)
                else {
                    return false;
                };
                let substitutions: BTreeMap<_, _> = definition
                    .generic_params
                    .iter()
                    .cloned()
                    .zip(args.iter().copied())
                    .collect();
                match &definition.kind {
                    TypeDefKind::Struct { fields, .. } => fields.iter().all(|field| {
                        self.annotation_is_copy(&field.annotation, &substitutions, active)
                    }),
                    TypeDefKind::Enum { variants } => variants.iter().all(|variant| {
                        variant
                            .payload
                            .iter()
                            .all(|payload| self.annotation_is_copy(payload, &substitutions, active))
                    }),
                }
            }
            CanonicalType::ClosureShape { .. } | CanonicalType::GenericParam { .. } => false,
            CanonicalType::Slice { .. } | CanonicalType::StringView => true,
        }
    }

    fn annotation_is_copy(
        &self,
        annotation: &TypeAnnotation,
        substitutions: &BTreeMap<String, TypeId>,
        active: &mut BTreeSet<TypeId>,
    ) -> bool {
        match annotation {
            TypeAnnotation::Unit
            | TypeAnnotation::Int
            | TypeAnnotation::I8
            | TypeAnnotation::U8
            | TypeAnnotation::I16
            | TypeAnnotation::U16
            | TypeAnnotation::I32
            | TypeAnnotation::U32
            | TypeAnnotation::Usize
            | TypeAnnotation::Float
            | TypeAnnotation::Bool
            | TypeAnnotation::Str
            | TypeAnnotation::Pointer(_)
            | TypeAnnotation::ConstPointer(_)
            | TypeAnnotation::Slice(_)
            | TypeAnnotation::MutSlice(_)
            | TypeAnnotation::Borrow { .. }
            | TypeAnnotation::StringView
            | TypeAnnotation::Function { .. } => true,
            TypeAnnotation::Array { element, .. } => {
                self.annotation_is_copy(element, substitutions, active)
            }
            TypeAnnotation::Named(name) => {
                if let Some(ty) = substitutions.get(name) {
                    return self.is_copy_inner(*ty, active);
                }
                let Some(definition) = self.definitions.iter().find(|definition| {
                    definition.name == *name && definition.generic_params.is_empty()
                }) else {
                    return false;
                };
                self.types
                    .values
                    .iter()
                    .position(|ty| {
                        matches!(
                            ty,
                            CanonicalType::Struct { symbol, args }
                                | CanonicalType::Enum { symbol, args }
                                if *symbol == definition.symbol && args.is_empty()
                        )
                    })
                    .is_some_and(|index| self.is_copy_inner(TypeId(index as u32), active))
            }
            TypeAnnotation::Generic { base, args } => {
                let Some(definition) = self
                    .definitions
                    .iter()
                    .find(|definition| definition.name == *base)
                else {
                    return false;
                };
                let Some(argument_ids) = args
                    .iter()
                    .map(|argument| self.find_annotation_type(argument, substitutions))
                    .collect::<Option<Vec<_>>>()
                else {
                    return false;
                };
                let nested: BTreeMap<_, _> = definition
                    .generic_params
                    .iter()
                    .cloned()
                    .zip(argument_ids)
                    .collect();
                let concrete_args: Vec<_> = definition
                    .generic_params
                    .iter()
                    .filter_map(|name| nested.get(name).copied())
                    .collect();
                self.types
                    .values
                    .iter()
                    .position(|ty| {
                        matches!(
                            ty,
                            CanonicalType::Struct { symbol, args }
                                | CanonicalType::Enum { symbol, args }
                                if *symbol == definition.symbol && *args == concrete_args
                        )
                    })
                    .is_some_and(|index| self.is_copy_inner(TypeId(index as u32), active))
            }
            TypeAnnotation::Bytes
            | TypeAnnotation::Buffer
            | TypeAnnotation::List(_)
            | TypeAnnotation::Closure { .. }
            | TypeAnnotation::Inferred => false,
        }
    }

    fn find_annotation_type(
        &self,
        annotation: &TypeAnnotation,
        substitutions: &BTreeMap<String, TypeId>,
    ) -> Option<TypeId> {
        if let TypeAnnotation::Named(name) = annotation {
            if let Some(ty) = substitutions.get(name) {
                return Some(*ty);
            }
        }
        self.types
            .values
            .iter()
            .position(|canonical| annotation_matches_shallow(annotation, canonical))
            .map(|index| TypeId(index as u32))
    }

    fn interface_fingerprint(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for definition in self
            .definitions
            .iter()
            .filter(|definition| definition.public)
        {
            for byte in format!("{definition:?}").bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        for function in self.functions.iter().filter(|function| function.public) {
            for byte in format!("{function:?}").bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        for global in self.globals.iter().filter(|global| global.public) {
            for byte in format!("{global:?}").bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        for function in self.functions.iter().filter(|function| function.public) {
            if let Some(origin) = self.function_allocator_origins.get(&function.symbol) {
                for byte in format!(
                    "allocator:{:?}",
                    self.allocator_origins.get(origin.0 as usize)
                )
                .bytes()
                {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
            if let Some(origin) = self.function_borrow_origins.get(&function.symbol) {
                for byte in
                    format!("borrow:{:?}", self.borrow_origins.get(origin.0 as usize)).bytes()
                {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
        hash
    }
}

fn type_expression_annotation(expression: &Expr) -> Option<TypeAnnotation> {
    match expression {
        Expr::Ident(name, _) => Some(TypeAnnotation::Named(name.clone())),
        Expr::GenericApply { target, args, .. } => match target.as_ref() {
            Expr::Ident(name, _) => Some(TypeAnnotation::Generic {
                base: name.clone(),
                args: args.clone(),
            }),
            _ => None,
        },
        _ => None,
    }
}

fn integer_literal_fits(integer: IntegerType, value: i64, layout: TargetDataLayout) -> bool {
    match integer {
        IntegerType::I8 => i8::try_from(value).is_ok(),
        IntegerType::U8 => u8::try_from(value).is_ok(),
        IntegerType::I16 => i16::try_from(value).is_ok(),
        IntegerType::U16 => u16::try_from(value).is_ok(),
        IntegerType::I32 => i32::try_from(value).is_ok(),
        IntegerType::U32 => u32::try_from(value).is_ok(),
        IntegerType::Usize => match layout.usize_size {
            1 => u8::try_from(value).is_ok(),
            2 => u16::try_from(value).is_ok(),
            4 => u32::try_from(value).is_ok(),
            _ => value >= 0,
        },
        IntegerType::LegacyInt => true,
    }
}

fn is_integer_literal_expression(expression: &Expr) -> bool {
    matches!(expression, Expr::IntLit(..))
        || matches!(
            expression,
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            } if matches!(expr.as_ref(), Expr::IntLit(..))
        )
}

fn drop_parameter_projection(expression: &Expr, parameter: &str) -> bool {
    match expression {
        Expr::Unary {
            op: UnaryOp::Deref,
            expr,
            ..
        } => matches!(expr.as_ref(), Expr::Ident(name, _) if name == parameter),
        Expr::Member { object, .. } => drop_parameter_projection(object, parameter),
        // Indexing through a raw pointer stored in an owning container is the
        // low-level initialized-storage contract used by Array[T] glue; it is
        // not a partial move of the container's pointer field itself.
        Expr::Index { .. } => false,
        _ => false,
    }
}

fn constant_integer_expression(expression: &Expr) -> Option<i64> {
    match expression {
        Expr::IntLit(value, _) => Some(*value),
        Expr::Cast { expr, .. } => constant_integer_expression(expr),
        Expr::Unary {
            op: UnaryOp::Neg,
            expr,
            ..
        } => constant_integer_expression(expr)?.checked_neg(),
        Expr::Unary {
            op: UnaryOp::BitNot,
            expr,
            ..
        } => Some(!constant_integer_expression(expr)?),
        Expr::Binary {
            left, op, right, ..
        } => {
            let left = constant_integer_expression(left)?;
            let right = constant_integer_expression(right)?;
            match op {
                BinOp::Add => left.checked_add(right),
                BinOp::Sub => left.checked_sub(right),
                BinOp::Mul => left.checked_mul(right),
                BinOp::Div => left.checked_div(right),
                BinOp::Rem => left.checked_rem(right),
                BinOp::BitAnd => Some(left & right),
                BinOp::BitOr => Some(left | right),
                BinOp::BitXor => Some(left ^ right),
                BinOp::Shl => u32::try_from(right)
                    .ok()
                    .and_then(|shift| left.checked_shl(shift)),
                BinOp::Shr => u32::try_from(right)
                    .ok()
                    .and_then(|shift| left.checked_shr(shift)),
                _ => None,
            }
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Binding {
    ty: TypeId,
    moved: bool,
    immut_borrows: u32,
    mut_borrowed: bool,
    #[allow(dead_code)]
    span: Span,
}

#[derive(Default, Clone, PartialEq)]
struct Ownership {
    scopes: Vec<BTreeMap<String, Binding>>,
    borrows: Vec<Vec<(String, bool)>>,
}

impl Ownership {
    fn push_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
        self.borrows.push(Vec::new());
    }

    fn pop_scope(&mut self) {
        if let Some(borrows) = self.borrows.pop() {
            for (name, mutable) in borrows.into_iter().rev() {
                if let Some(binding) = self.lookup_mut(&name) {
                    if mutable {
                        binding.mut_borrowed = false;
                    } else {
                        binding.immut_borrows = binding.immut_borrows.saturating_sub(1);
                    }
                }
            }
        }
        self.scopes.pop();
    }

    fn define(&mut self, name: String, ty: TypeId, span: Span) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(
                name,
                Binding {
                    ty,
                    moved: false,
                    immut_borrows: 0,
                    mut_borrowed: false,
                    span,
                },
            );
        }
    }

    fn lookup_mut(&mut self, name: &str) -> Option<&mut Binding> {
        self.scopes
            .iter_mut()
            .rev()
            .find_map(|scope| scope.get_mut(name))
    }

    fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    fn record_borrow(&mut self, name: String, mutable: bool) {
        if let Some(borrows) = self.borrows.last_mut() {
            borrows.push((name, mutable));
        }
    }
}

pub fn layout_of(program: &HirProgram, ty: TypeId) -> Result<Layout, Diagnostic> {
    layout_of_inner(program, ty, &mut BTreeSet::new())
}

fn layout_of_inner(
    program: &HirProgram,
    ty: TypeId,
    active: &mut BTreeSet<TypeId>,
) -> Result<Layout, Diagnostic> {
    if !active.insert(ty) {
        return Err(Diagnostic {
            code: "E_RECURSIVE_LAYOUT",
            message: "recursive by-value type has no finite layout".into(),
            span: Span::zero(),
        });
    }
    let result = match program.type_of(ty) {
        CanonicalType::Unit => Layout { size: 0, align: 1 },
        CanonicalType::Integer(IntegerType::I8 | IntegerType::U8) | CanonicalType::Bool => {
            Layout { size: 1, align: 1 }
        }
        CanonicalType::Integer(IntegerType::I16 | IntegerType::U16) => Layout { size: 2, align: 2 },
        CanonicalType::Integer(_) | CanonicalType::Float | CanonicalType::Function { .. } => {
            Layout { size: 4, align: 4 }
        }
        CanonicalType::Pointer { .. } | CanonicalType::Borrow { .. } | CanonicalType::LegacyStr => {
            Layout {
                size: program.layout.pointer_size,
                align: program.layout.pointer_align,
            }
        }
        CanonicalType::Slice { .. }
        | CanonicalType::StringView
        | CanonicalType::ClosureShape { .. } => Layout {
            size: program.layout.pointer_size + program.layout.usize_size,
            align: program.layout.pointer_align.max(program.layout.usize_align),
        },
        CanonicalType::Array { element, length } => {
            let element = layout_of_inner(program, *element, active)?;
            Layout {
                size: element
                    .size
                    .checked_mul(*length)
                    .ok_or_else(|| Diagnostic {
                        code: "E_LAYOUT_OVERFLOW",
                        message: "array layout overflows u32".into(),
                        span: Span::zero(),
                    })?,
                align: element.align,
            }
        }
        CanonicalType::Struct { symbol, args } => {
            layout_aggregate(program, *symbol, args, active)?.0
        }
        CanonicalType::Enum { symbol, args } => layout_enum(program, *symbol, args, active)?.layout,
        CanonicalType::LegacyBytes
        | CanonicalType::LegacyBuffer
        | CanonicalType::LegacyList(_)
        | CanonicalType::GenericParam { .. } => {
            return Err(Diagnostic {
                code: "E_LAYOUT_UNAVAILABLE",
                message: "open or hosted compatibility type has no native object layout".into(),
                span: Span::zero(),
            })
        }
    };
    active.remove(&ty);
    Ok(result)
}

fn layout_aggregate(
    program: &HirProgram,
    symbol: SymbolId,
    args: &[TypeId],
    active: &mut BTreeSet<TypeId>,
) -> Result<(Layout, Vec<u32>), Diagnostic> {
    let definition = program
        .definitions
        .iter()
        .find(|definition| definition.symbol == symbol)
        .ok_or_else(|| Diagnostic {
            code: "E_UNKNOWN_TYPE",
            message: "aggregate definition is missing".into(),
            span: Span::zero(),
        })?;
    let TypeDefKind::Struct { repr, fields } = &definition.kind else {
        return Err(Diagnostic {
            code: "E_NOT_A_STRUCT",
            message: "type is not a struct".into(),
            span: definition.span,
        });
    };
    let substitutions: BTreeMap<_, _> = definition
        .generic_params
        .iter()
        .cloned()
        .zip(args.iter().copied())
        .collect();
    let mut offset = 0u32;
    let mut align = 1u32;
    let mut offsets = Vec::new();
    for field in fields {
        let field_layout = layout_annotation(program, &field.annotation, &substitutions, active)?;
        let field_align = if matches!(repr, Repr::CPacked) {
            1
        } else {
            field_layout.align
        };
        offset = align_up(offset, field_align)?;
        offsets.push(offset);
        offset = offset
            .checked_add(field_layout.size)
            .ok_or_else(|| Diagnostic {
                code: "E_LAYOUT_OVERFLOW",
                message: "struct layout overflows u32".into(),
                span: field.span,
            })?;
        align = align.max(field_align);
    }
    let size = align_up(offset, align)?;
    Ok((Layout { size, align }, offsets))
}

pub fn layout_struct(
    program: &HirProgram,
    symbol: SymbolId,
    args: &[TypeId],
) -> Result<(Layout, Vec<u32>), Diagnostic> {
    layout_aggregate(program, symbol, args, &mut BTreeSet::new())
}

pub fn struct_field_layout(
    program: &HirProgram,
    ty: TypeId,
    field_id: FieldId,
) -> Result<(TypeId, u32), Diagnostic> {
    let CanonicalType::Struct { symbol, args } = program.type_of(ty) else {
        return Err(Diagnostic {
            code: "E_NOT_A_STRUCT",
            message: "field layout requires a struct type".into(),
            span: Span::zero(),
        });
    };
    let definition = program
        .definitions
        .iter()
        .find(|definition| definition.symbol == *symbol)
        .ok_or_else(|| Diagnostic {
            code: "E_UNKNOWN_TYPE",
            message: "struct definition is missing".into(),
            span: Span::zero(),
        })?;
    let TypeDefKind::Struct { fields, .. } = &definition.kind else {
        unreachable!()
    };
    let index = fields
        .iter()
        .position(|field| field.id == field_id)
        .ok_or_else(|| Diagnostic {
            code: "E_UNKNOWN_FIELD",
            message: "field does not belong to the struct type".into(),
            span: definition.span,
        })?;
    let (_, offsets) = layout_struct(program, *symbol, args)?;
    let substitutions: BTreeMap<_, _> = definition
        .generic_params
        .iter()
        .cloned()
        .zip(args.iter().copied())
        .collect();
    let field_ty = resolved_annotation(program, &fields[index].annotation, &substitutions)
        .map_err(|error| Diagnostic {
            message: format!(
                "{} while resolving field `{}.{}`",
                error.message, definition.name, fields[index].name
            ),
            ..error
        })?;
    Ok((field_ty, offsets[index]))
}

pub fn enum_variant_layout(
    program: &HirProgram,
    ty: TypeId,
    variant_id: VariantId,
) -> Result<EnumVariantFieldLayout, Diagnostic> {
    let CanonicalType::Enum { symbol, args } = program.type_of(ty) else {
        return Err(Diagnostic {
            code: "E_NOT_AN_ENUM",
            message: "variant layout requires an enum type".into(),
            span: Span::zero(),
        });
    };
    let definition = program
        .definitions
        .iter()
        .find(|definition| definition.symbol == *symbol)
        .ok_or_else(|| Diagnostic {
            code: "E_UNKNOWN_TYPE",
            message: "enum definition is missing".into(),
            span: Span::zero(),
        })?;
    let TypeDefKind::Enum { variants } = &definition.kind else {
        unreachable!()
    };
    let variant = variants
        .iter()
        .find(|variant| variant.id == variant_id)
        .ok_or_else(|| Diagnostic {
            code: "E_UNKNOWN_VARIANT",
            message: "variant does not belong to the enum type".into(),
            span: definition.span,
        })?;
    let substitutions: BTreeMap<_, _> = definition
        .generic_params
        .iter()
        .cloned()
        .zip(args.iter().copied())
        .collect();
    let enum_layout = layout_enum(program, *symbol, args, &mut BTreeSet::new())?;
    let mut offset = 0u32;
    let mut payload = Vec::with_capacity(variant.payload.len());
    for annotation in &variant.payload {
        let payload_ty = resolved_annotation(program, annotation, &substitutions)?;
        let layout = layout_of(program, payload_ty)?;
        offset = align_up(offset, layout.align)?;
        payload.push((payload_ty, enum_layout.payload_offset + offset));
        offset = offset.checked_add(layout.size).ok_or_else(|| Diagnostic {
            code: "E_LAYOUT_OVERFLOW",
            message: "enum variant payload offset overflow".into(),
            span: variant.span,
        })?;
    }
    Ok((variant.discriminant, enum_layout.payload_offset, payload))
}

pub fn layout_enum(
    program: &HirProgram,
    symbol: SymbolId,
    args: &[TypeId],
    active: &mut BTreeSet<TypeId>,
) -> Result<EnumLayout, Diagnostic> {
    let definition = program
        .definitions
        .iter()
        .find(|definition| definition.symbol == symbol)
        .ok_or_else(|| Diagnostic {
            code: "E_UNKNOWN_TYPE",
            message: "enum definition is missing".into(),
            span: Span::zero(),
        })?;
    let TypeDefKind::Enum { variants } = &definition.kind else {
        return Err(Diagnostic {
            code: "E_NOT_AN_ENUM",
            message: "type is not an enum".into(),
            span: definition.span,
        });
    };
    let substitutions: BTreeMap<_, _> = definition
        .generic_params
        .iter()
        .cloned()
        .zip(args.iter().copied())
        .collect();
    let mut payload = Layout { size: 0, align: 1 };
    for variant in variants {
        let mut size = 0;
        let mut align = 1;
        for annotation in &variant.payload {
            let field = layout_annotation(program, annotation, &substitutions, active)?;
            size = align_up(size, field.align)?;
            size = size.checked_add(field.size).ok_or_else(|| Diagnostic {
                code: "E_LAYOUT_OVERFLOW",
                message: "enum payload layout overflows u32".into(),
                span: variant.span,
            })?;
            align = align.max(field.align);
        }
        size = align_up(size, align)?;
        payload.size = payload.size.max(size);
        payload.align = payload.align.max(align);
    }
    let payload_offset = align_up(4, payload.align)?;
    let align = 4.max(payload.align);
    let size = align_up(
        payload_offset
            .checked_add(payload.size)
            .ok_or_else(|| Diagnostic {
                code: "E_LAYOUT_OVERFLOW",
                message: "enum layout overflows u32".into(),
                span: definition.span,
            })?,
        align,
    )?;
    Ok(EnumLayout {
        layout: Layout { size, align },
        payload_offset,
        payload,
    })
}

fn layout_annotation(
    program: &HirProgram,
    annotation: &TypeAnnotation,
    substitutions: &BTreeMap<String, TypeId>,
    active: &mut BTreeSet<TypeId>,
) -> Result<Layout, Diagnostic> {
    let scalar = match annotation {
        TypeAnnotation::Unit => Some(Layout { size: 0, align: 1 }),
        TypeAnnotation::I8 | TypeAnnotation::U8 | TypeAnnotation::Bool => {
            Some(Layout { size: 1, align: 1 })
        }
        TypeAnnotation::I16 | TypeAnnotation::U16 => Some(Layout { size: 2, align: 2 }),
        TypeAnnotation::Int
        | TypeAnnotation::I32
        | TypeAnnotation::U32
        | TypeAnnotation::Usize
        | TypeAnnotation::Float
        | TypeAnnotation::Function { .. } => Some(Layout { size: 4, align: 4 }),
        TypeAnnotation::Pointer(_)
        | TypeAnnotation::ConstPointer(_)
        | TypeAnnotation::Borrow { .. }
        | TypeAnnotation::Str => Some(Layout {
            size: program.layout.pointer_size,
            align: program.layout.pointer_align,
        }),
        TypeAnnotation::Slice(_)
        | TypeAnnotation::MutSlice(_)
        | TypeAnnotation::StringView
        | TypeAnnotation::Closure { .. } => Some(Layout {
            size: program.layout.pointer_size + program.layout.usize_size,
            align: program.layout.pointer_align.max(program.layout.usize_align),
        }),
        _ => None,
    };
    if let Some(layout) = scalar {
        return Ok(layout);
    }
    match annotation {
        TypeAnnotation::Array { element, length } => {
            let element = layout_annotation(program, element, substitutions, active)?;
            Ok(Layout {
                size: element
                    .size
                    .checked_mul(*length)
                    .ok_or_else(|| Diagnostic {
                        code: "E_LAYOUT_OVERFLOW",
                        message: "array layout overflows u32".into(),
                        span: Span::zero(),
                    })?,
                align: element.align,
            })
        }
        TypeAnnotation::Named(name) => {
            if let Some(ty) = substitutions.get(name) {
                return layout_of_inner(program, *ty, active);
            }
            let definition = program
                .definitions
                .iter()
                .find(|definition| definition.name == *name)
                .ok_or_else(|| Diagnostic {
                    code: "E_UNKNOWN_TYPE",
                    message: format!("unknown layout type `{name}`"),
                    span: Span::zero(),
                })?;
            if let Some(index) = program.types.iter().position(|ty| {
                matches!(
                    ty,
                    CanonicalType::Struct { symbol, args }
                        | CanonicalType::Enum { symbol, args }
                        if *symbol == definition.symbol && args.is_empty()
                )
            }) {
                return layout_of_inner(program, TypeId(index as u32), active);
            }
            match definition.kind {
                TypeDefKind::Struct { .. } => {
                    layout_aggregate(program, definition.symbol, &[], active).map(|value| value.0)
                }
                TypeDefKind::Enum { .. } => {
                    layout_enum(program, definition.symbol, &[], active).map(|value| value.layout)
                }
            }
        }
        TypeAnnotation::Generic { base, args } => {
            let definition = program
                .definitions
                .iter()
                .find(|definition| definition.name == *base)
                .ok_or_else(|| Diagnostic {
                    code: "E_UNKNOWN_TYPE",
                    message: format!("unknown layout type `{base}`"),
                    span: Span::zero(),
                })?;
            let arguments = args
                .iter()
                .map(|argument| resolved_annotation(program, argument, substitutions))
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(index) = program.types.iter().position(|ty| {
                matches!(
                    ty,
                    CanonicalType::Struct { symbol, args }
                        | CanonicalType::Enum { symbol, args }
                        if *symbol == definition.symbol && *args == arguments
                )
            }) {
                return layout_of_inner(program, TypeId(index as u32), active);
            }
            match definition.kind {
                TypeDefKind::Struct { .. } => {
                    layout_aggregate(program, definition.symbol, &arguments, active)
                        .map(|value| value.0)
                }
                TypeDefKind::Enum { .. } => {
                    layout_enum(program, definition.symbol, &arguments, active)
                        .map(|value| value.layout)
                }
            }
        }
        TypeAnnotation::Bytes
        | TypeAnnotation::Buffer
        | TypeAnnotation::List(_)
        | TypeAnnotation::Inferred => Err(Diagnostic {
            code: "E_LAYOUT_UNAVAILABLE",
            message: format!("type `{annotation}` has no native object layout"),
            span: Span::zero(),
        }),
        _ => unreachable!("scalar annotations returned above"),
    }
}

fn resolved_annotation(
    program: &HirProgram,
    annotation: &TypeAnnotation,
    substitutions: &BTreeMap<String, TypeId>,
) -> Result<TypeId, Diagnostic> {
    if let TypeAnnotation::Named(name) = annotation {
        if let Some(id) = substitutions.get(name) {
            return Ok(*id);
        }
    }
    program
        .types
        .iter()
        .position(|canonical| annotation_matches(program, annotation, canonical, substitutions))
        .map(|index| TypeId(index as u32))
        .ok_or_else(|| Diagnostic {
            code: "E_LAYOUT_UNAVAILABLE",
            message: format!("cannot resolve concrete layout for `{annotation}`"),
            span: Span::zero(),
        })
}

fn annotation_matches(
    program: &HirProgram,
    annotation: &TypeAnnotation,
    canonical: &CanonicalType,
    substitutions: &BTreeMap<String, TypeId>,
) -> bool {
    match (annotation, canonical) {
        (TypeAnnotation::Unit, CanonicalType::Unit)
        | (TypeAnnotation::Int, CanonicalType::Integer(IntegerType::LegacyInt))
        | (TypeAnnotation::I8, CanonicalType::Integer(IntegerType::I8))
        | (TypeAnnotation::U8, CanonicalType::Integer(IntegerType::U8))
        | (TypeAnnotation::I16, CanonicalType::Integer(IntegerType::I16))
        | (TypeAnnotation::U16, CanonicalType::Integer(IntegerType::U16))
        | (TypeAnnotation::I32, CanonicalType::Integer(IntegerType::I32))
        | (TypeAnnotation::U32, CanonicalType::Integer(IntegerType::U32))
        | (TypeAnnotation::Usize, CanonicalType::Integer(IntegerType::Usize))
        | (TypeAnnotation::Float, CanonicalType::Float)
        | (TypeAnnotation::Bool, CanonicalType::Bool)
        | (TypeAnnotation::Str, CanonicalType::LegacyStr)
        | (TypeAnnotation::Bytes, CanonicalType::LegacyBytes)
        | (TypeAnnotation::Buffer, CanonicalType::LegacyBuffer)
        | (TypeAnnotation::StringView, CanonicalType::StringView) => true,
        (TypeAnnotation::Named(name), _) => {
            substitutions
                .get(name)
                .is_some_and(|id| program.type_of(*id) == canonical)
                || matches!(
                    canonical,
                    CanonicalType::Struct { symbol, args }
                        | CanonicalType::Enum { symbol, args }
                        if args.is_empty()
                            && program.definitions.iter().any(|definition| {
                                definition.symbol == *symbol && definition.name == *name
                            })
                )
        }
        (TypeAnnotation::List(inner), CanonicalType::LegacyList(element)) => {
            annotation_matches(program, inner, program.type_of(*element), substitutions)
        }
        (
            TypeAnnotation::Pointer(inner),
            CanonicalType::Pointer {
                mutable: true,
                pointee,
            },
        )
        | (
            TypeAnnotation::ConstPointer(inner),
            CanonicalType::Pointer {
                mutable: false,
                pointee,
            },
        ) => annotation_matches(program, inner, program.type_of(*pointee), substitutions),
        (
            TypeAnnotation::Borrow {
                mutable,
                pointee: inner,
            },
            CanonicalType::Borrow {
                mutable: candidate_mutable,
                pointee,
            },
        ) => {
            mutable == candidate_mutable
                && annotation_matches(program, inner, program.type_of(*pointee), substitutions)
        }
        (
            TypeAnnotation::Slice(inner),
            CanonicalType::Slice {
                mutable: false,
                element,
            },
        )
        | (
            TypeAnnotation::MutSlice(inner),
            CanonicalType::Slice {
                mutable: true,
                element,
            },
        ) => annotation_matches(program, inner, program.type_of(*element), substitutions),
        (
            TypeAnnotation::Array { element, length },
            CanonicalType::Array {
                element: candidate,
                length: candidate_length,
            },
        ) => {
            length == candidate_length
                && annotation_matches(program, element, program.type_of(*candidate), substitutions)
        }
        (
            TypeAnnotation::Generic { base, args },
            CanonicalType::Struct {
                symbol,
                args: candidates,
            }
            | CanonicalType::Enum {
                symbol,
                args: candidates,
            },
        ) => {
            program
                .definitions
                .iter()
                .any(|definition| definition.symbol == *symbol && definition.name == *base)
                && args.len() == candidates.len()
                && args.iter().zip(candidates).all(|(annotation, candidate)| {
                    annotation_matches(
                        program,
                        annotation,
                        program.type_of(*candidate),
                        substitutions,
                    )
                })
        }
        (
            TypeAnnotation::Function {
                convention,
                params,
                ret,
            },
            CanonicalType::Function {
                convention: candidate_convention,
                params: candidates,
                result,
            },
        ) => {
            convention == candidate_convention
                && params.len() == candidates.len()
                && params
                    .iter()
                    .zip(candidates)
                    .all(|(annotation, candidate)| {
                        annotation_matches(
                            program,
                            annotation,
                            program.type_of(*candidate),
                            substitutions,
                        )
                    })
                && annotation_matches(program, ret, program.type_of(*result), substitutions)
        }
        (
            TypeAnnotation::Closure { params, ret },
            CanonicalType::ClosureShape {
                params: candidates,
                result,
            },
        ) => {
            params.len() == candidates.len()
                && params
                    .iter()
                    .zip(candidates)
                    .all(|(annotation, candidate)| {
                        annotation_matches(
                            program,
                            annotation,
                            program.type_of(*candidate),
                            substitutions,
                        )
                    })
                && annotation_matches(program, ret, program.type_of(*result), substitutions)
        }
        _ => false,
    }
}

fn annotation_matches_shallow(annotation: &TypeAnnotation, canonical: &CanonicalType) -> bool {
    matches!(
        (annotation, canonical),
        (
            TypeAnnotation::Int,
            CanonicalType::Integer(IntegerType::LegacyInt)
        ) | (TypeAnnotation::I8, CanonicalType::Integer(IntegerType::I8))
            | (TypeAnnotation::U8, CanonicalType::Integer(IntegerType::U8))
            | (
                TypeAnnotation::I16,
                CanonicalType::Integer(IntegerType::I16)
            )
            | (
                TypeAnnotation::U16,
                CanonicalType::Integer(IntegerType::U16)
            )
            | (
                TypeAnnotation::I32,
                CanonicalType::Integer(IntegerType::I32)
            )
            | (
                TypeAnnotation::U32,
                CanonicalType::Integer(IntegerType::U32)
            )
            | (
                TypeAnnotation::Usize,
                CanonicalType::Integer(IntegerType::Usize)
            )
            | (TypeAnnotation::Float, CanonicalType::Float)
            | (TypeAnnotation::Bool, CanonicalType::Bool)
            | (TypeAnnotation::StringView, CanonicalType::StringView)
    )
}

/// Preserve the hosted language's historical top-level inference when the
/// project resolver promotes an imported module's `let` binding to a global.
/// This happens before canonical type checking, so only syntax-directed
/// literals are admitted here; every other initializer remains inferred and
/// receives the ordinary deterministic diagnostic.
fn infer_legacy_global_annotation(expression: &Expr) -> Option<TypeAnnotation> {
    match expression {
        Expr::UnitLit(_) => Some(TypeAnnotation::Unit),
        Expr::IntLit(..) => Some(TypeAnnotation::Int),
        Expr::FloatLit(..) => Some(TypeAnnotation::Float),
        Expr::BoolLit(..) => Some(TypeAnnotation::Bool),
        Expr::StrLit(..) | Expr::FStr(..) => Some(TypeAnnotation::Str),
        Expr::BytesLit(..) => Some(TypeAnnotation::Bytes),
        Expr::List(values, _) => {
            let first = values.first().and_then(infer_legacy_global_annotation)?;
            if values
                .iter()
                .skip(1)
                .all(|value| infer_legacy_global_annotation(value).as_ref() == Some(&first))
            {
                Some(TypeAnnotation::List(Box::new(first)))
            } else {
                None
            }
        }
        Expr::Unary { expr, .. } | Expr::Cast { expr, .. } => infer_legacy_global_annotation(expr),
        _ => None,
    }
}

fn infer_legacy_return_annotation(
    statements: &[Stmt],
    params: &[dbyte_ast::Param],
) -> Option<TypeAnnotation> {
    fn expression_type(expression: &Expr, params: &[dbyte_ast::Param]) -> Option<TypeAnnotation> {
        match expression {
            Expr::UnitLit(_) => Some(TypeAnnotation::Unit),
            Expr::IntLit(..) => Some(TypeAnnotation::Int),
            Expr::FloatLit(..) => Some(TypeAnnotation::Float),
            Expr::BoolLit(..) => Some(TypeAnnotation::Bool),
            Expr::StrLit(..) => Some(TypeAnnotation::Str),
            Expr::BytesLit(..) => Some(TypeAnnotation::Bytes),
            Expr::Ident(name, _) => params
                .iter()
                .find(|parameter| parameter.name == *name)
                .map(|parameter| parameter.ty.clone()),
            Expr::Unary { expr, .. } | Expr::Cast { expr, .. } => expression_type(expr, params),
            Expr::Binary { left, .. } => expression_type(left, params),
            Expr::Call { name, .. } if name == "len" => Some(TypeAnnotation::Int),
            _ => None,
        }
    }

    for statement in statements {
        match statement {
            Stmt::Return {
                value: Some(value), ..
            } => return expression_type(value, params),
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                if let Some(inferred) = infer_legacy_return_annotation(then_body, params) {
                    return Some(inferred);
                }
                if let Some(inferred) = else_body
                    .as_deref()
                    .and_then(|body| infer_legacy_return_annotation(body, params))
                {
                    return Some(inferred);
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => {
                if let Some(inferred) = infer_legacy_return_annotation(body, params) {
                    return Some(inferred);
                }
            }
            _ => {}
        }
    }
    None
}

fn display_function_name(name: &str) -> &str {
    for prefix in ["__dbyte_m", "__dbyte_pub_m"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            if let Some((module, source)) = rest.split_once('_') {
                if module.chars().all(|character| character.is_ascii_digit()) {
                    return source;
                }
            }
        }
    }
    name
}

fn align_up(value: u32, align: u32) -> Result<u32, Diagnostic> {
    let mask = align.checked_sub(1).ok_or_else(|| Diagnostic {
        code: "E_LAYOUT_ALIGNMENT",
        message: "alignment cannot be zero".into(),
        span: Span::zero(),
    })?;
    value
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or_else(|| Diagnostic {
            code: "E_LAYOUT_OVERFLOW",
            message: "alignment arithmetic overflows u32".into(),
            span: Span::zero(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    fn parse(source: &str) -> Program {
        let tokens = Lexer::new(source).tokenize().unwrap();
        Parser::new(tokens).parse_program().unwrap()
    }

    #[test]
    fn ids_and_dump_are_deterministic() {
        let source = "pub struct Pair[A, B]:\n    first: A\n    second: B\n\npub enum Choice[T]:\n    None\n    Some(T)\n\nfn id[T](value: T) -> T:\n    return value\n";
        let first = analyze(&parse(source), Profile::Hosted, TargetDataLayout::I686).unwrap();
        let second = analyze(&parse(source), Profile::Hosted, TargetDataLayout::I686).unwrap();
        assert_eq!(first.dump(), second.dump());
        assert!(first.dump().contains("generic=Some(GenericDefId"));
    }

    #[test]
    fn enum_layout_uses_public_u32_tag_contract() {
        let source = "pub enum Value:\n    Empty\n    Byte(u8)\n    Number(u32)\n";
        let hir = analyze(&parse(source), Profile::I686None, TargetDataLayout::I686).unwrap();
        let symbol = hir
            .definitions
            .iter()
            .find(|definition| definition.name == "Value")
            .unwrap()
            .symbol;
        let ty = hir
            .types
            .iter()
            .position(
                |ty| matches!(ty, CanonicalType::Enum { symbol: found, .. } if *found == symbol),
            )
            .map(|index| TypeId(index as u32));
        let ty = ty.unwrap_or({
            // A declaration without a use is still laid out by constructing its canonical identity.
            TypeId(u32::MAX)
        });
        let layout = if ty.0 == u32::MAX {
            layout_enum(&hir, symbol, &[], &mut BTreeSet::new()).unwrap()
        } else {
            let CanonicalType::Enum { args, .. } = hir.type_of(ty) else {
                unreachable!()
            };
            layout_enum(&hir, symbol, args, &mut BTreeSet::new()).unwrap()
        };
        assert_eq!(layout.payload_offset, 4);
        assert_eq!(layout.layout, Layout { size: 8, align: 4 });
    }

    #[test]
    fn copy_aggregates_can_be_reused_and_variant_payloads_are_checked() {
        let source = "struct Pair:\n    first: i32\n    second: i32\n\nfn check() -> i32:\n    let pair = Pair(first = 1, second = 2)\n    let second = pair\n    let third = pair\n    let result = Result[i32, u8].Ok(3)\n    match result:\n        case Result.Ok(value):\n            return value\n        case Result.Err(error):\n            return error as i32\n";
        analyze(&parse(source), Profile::Hosted, TargetDataLayout::I686)
            .expect("all-Copy aggregate and exhaustive enum match analyze");
    }

    #[test]
    fn named_struct_initializers_reject_duplicate_unknown_and_missing_fields() {
        let source = "struct Pair:\n    first: i32\n    second: i32\n\nfn bad() -> i32:\n    let pair = Pair(first = 1, first = 2, extra = 3)\n    return 0\n";
        let diagnostics = analyze(&parse(source), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("invalid named construction must fail");
        let codes: BTreeSet<_> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert!(codes.contains("E_DUPLICATE_FIELD_INITIALIZER"));
        assert!(codes.contains("E_UNKNOWN_FIELD"));
        assert!(codes.contains("E_MISSING_FIELD"));
    }

    #[test]
    fn result_try_unwraps_once_and_requires_identical_error_type() {
        let valid = "fn propagate(input: Result[i32, u8]) -> Result[i32, u8]:\n    let value = input?\n    return Result[i32, u8].Ok(value)\n";
        analyze(&parse(valid), Profile::Hosted, TargetDataLayout::I686)
            .expect("matching Result error type propagates");

        let invalid = "fn propagate(input: Result[i32, u8]) -> Result[i32, u16]:\n    let value = input?\n    return Result[i32, u16].Ok(value)\n";
        let diagnostics = analyze(&parse(invalid), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("mismatched Result error type fails");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_TRY_RETURN_TYPE"));
    }

    #[test]
    fn defer_is_lowered_to_lifo_cleanup_edges_for_try_and_return() {
        let source = "fn run(input: Result[i32, u8]) -> Result[i32, u8]:\n    defer []:\n        let marker = 1\n    let value = input?\n    return Result[i32, u8].Ok(value)\n";
        let hir = analyze(&parse(source), Profile::Hosted, TargetDataLayout::I686)
            .expect("defer Result flow analyzes");
        assert!(hir
            .cleanup_edges
            .iter()
            .any(|edge| edge.exit == CleanupExit::PropagateError));
        assert!(hir
            .cleanup_edges
            .iter()
            .any(|edge| edge.exit == CleanupExit::Return));
        assert!(hir.dump().contains("cleanup"));
    }

    #[test]
    fn closures_require_explicit_captures_and_are_locally_callable() {
        let valid = "fn run() -> i32:\n    let factor = 2\n    let add = fn [borrow factor](value: i32) -> i32:\n        return value + factor\n    return add(40)\n";
        let hir = analyze(&parse(valid), Profile::Hosted, TargetDataLayout::I686)
            .expect("explicitly captured closure analyzes");
        assert_eq!(hir.closure_count, 1);

        let invalid = "fn run() -> i32:\n    let factor = 2\n    let add = fn [](value: i32) -> i32:\n        return value + factor\n    return add(40)\n";
        let diagnostics = analyze(&parse(invalid), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("implicit closure capture must fail");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_UNKNOWN_SYMBOL"
                && diagnostic.message.contains("factor")));
    }

    #[test]
    fn recursive_layout_is_rejected_but_pointer_recursion_is_finite() {
        let bad = "struct Node:\n    next: Node\n";
        let diagnostics = analyze(&parse(bad), Profile::I686None, TargetDataLayout::I686)
            .expect_err("recursive by-value aggregate must fail");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_RECURSIVE_LAYOUT"));

        let good = "struct Node:\n    next: ptr[Node]\n";
        analyze(&parse(good), Profile::I686None, TargetDataLayout::I686)
            .expect("pointer recursion has finite target layout");
    }

    #[test]
    fn borrowed_returns_require_one_direct_parameter_source() {
        let valid = "fn view(data: slice[u8]) -> slice[u8]:\n    return data\n";
        let hir = analyze(&parse(valid), Profile::Hosted, TargetDataLayout::I686)
            .expect("single direct borrowed parameter can be returned");
        let function = hir
            .functions
            .iter()
            .find(|function| function.name == "view")
            .unwrap();
        let origin = hir.function_borrow_origins[&function.symbol];
        assert!(matches!(
            hir.borrow_origins[origin.0 as usize],
            BorrowOrigin::Parameter { index: 0, .. }
        ));

        let ambiguous =
            "fn view(left: slice[u8], right: slice[u8]) -> slice[u8]:\n    return left\n";
        let diagnostics = analyze(&parse(ambiguous), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("ambiguous borrowed return must fail");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_AMBIGUOUS_BORROWED_RETURN"));

        let forwarded = "fn identity(data: slice[u8]) -> slice[u8]:\n    return data\n\nfn forward(data: slice[u8]) -> slice[u8]:\n    return identity(data)\n";
        let hir = analyze(&parse(forwarded), Profile::Hosted, TargetDataLayout::I686)
            .expect("borrow provenance is translated across direct calls");
        let function = hir
            .functions
            .iter()
            .find(|function| function.name == "forward")
            .unwrap();
        let origin = hir.function_borrow_origins[&function.symbol];
        assert!(matches!(
            hir.borrow_origins[origin.0 as usize],
            BorrowOrigin::Parameter { index: 0, .. }
        ));
    }

    #[test]
    fn allocator_provenance_rejects_lexical_escape_and_summarizes_parameters() {
        let leaking = "struct Arena:\n    marker: u32\n\nstruct AllocatorRef:\n    context: ptr[u8]\n\nfn arena_initialize(arena: ptr[Arena]) -> AllocatorRef:\n    return AllocatorRef(context = null)\n\nfn leak() -> AllocatorRef:\n    let arena = Arena(marker = 0 as u32)\n    let allocator = arena_initialize(&arena)\n    return allocator\n";
        let errors = analyze(&parse(leaking), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("lexical allocator must not escape");
        assert!(errors
            .iter()
            .any(|error| error.code == "E_ALLOCATOR_LIFETIME_ESCAPE"));

        let propagated = "struct AllocatorRef:\n    context: ptr[u8]\n\npub fn pass(allocator: AllocatorRef) -> AllocatorRef:\n    return allocator\n";
        let hir = analyze(&parse(propagated), Profile::Hosted, TargetDataLayout::I686).unwrap();
        let function = hir
            .functions
            .iter()
            .find(|function| function.name == "pass")
            .unwrap();
        let origin = hir.function_allocator_origins[&function.symbol];
        assert!(matches!(
            hir.allocator_origins[origin.0 as usize],
            AllocatorOrigin::Parameter { index: 0, .. }
        ));

        let forwarded = "struct AllocatorRef:\n    context: ptr[u8]\n\nfn pass(allocator: AllocatorRef) -> AllocatorRef:\n    return allocator\n\nfn forward(allocator: AllocatorRef) -> AllocatorRef:\n    let selected = pass(allocator)\n    let replacement = selected\n    return replacement\n";
        let hir = analyze(&parse(forwarded), Profile::Hosted, TargetDataLayout::I686)
            .expect("allocator provenance survives calls and assignments");
        let function = hir
            .functions
            .iter()
            .find(|function| function.name == "forward")
            .unwrap();
        let origin = hir.function_allocator_origins[&function.symbol];
        assert!(matches!(
            hir.allocator_origins[origin.0 as usize],
            AllocatorOrigin::Parameter { index: 0, .. }
        ));
    }

    #[test]
    fn generic_builtin_capabilities_are_inferred_and_checked_at_specialization() {
        let valid = "fn add[T](left: T, right: T) -> T:\n    return left + right\n\nfn run() -> i32:\n    return add[i32](40, 2)\n";
        let hir = analyze(&parse(valid), Profile::Hosted, TargetDataLayout::I686)
            .expect("numeric specialization satisfies inferred capability");
        assert!(hir.dump().contains("Numeric"));

        let invalid = "fn add[T](left: T, right: T) -> T:\n    return left + right\n\nfn run() -> StringView:\n    return add[StringView](\"a\", \"b\")\n";
        let diagnostics = analyze(&parse(invalid), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("StringView does not satisfy numeric capability");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_GENERIC_CAPABILITY"));
    }

    #[test]
    fn generic_recursion_distinguishes_same_type_from_expanding_instantiation() {
        let same = "fn again[T](value: T) -> T:\n    return again[T](value)\n";
        analyze(&parse(same), Profile::Hosted, TargetDataLayout::I686)
            .expect("same-type generic recursion is finite in the specialization cache");

        let growing = "fn again[T](value: T) -> T:\n    return again[array[T, 2]](value)\n";
        let diagnostics = analyze(&parse(growing), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("type-growing generic recursion must fail deterministically");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_GENERIC_INSTANTIATION_RECURSION"));
    }

    #[test]
    fn null_and_string_literals_require_and_use_contextual_canonical_types() {
        let valid = "fn run() -> i32:\n    let pointer: ptr[u8] = null\n    let legacy: str = \"legacy\"\n    let view: StringView = \"view\"\n    return 0\n";
        analyze(&parse(valid), Profile::Hosted, TargetDataLayout::I686)
            .expect("context determines null and text representation");

        let invalid = "fn run() -> i32:\n    let value = null\n    return 0\n";
        let diagnostics = analyze(&parse(invalid), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("untyped null must fail");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_NULL_INFERENCE"));
    }

    #[test]
    fn function_pointers_and_noncapturing_closure_coercion_are_convention_checked() {
        let valid = "fn increment(value: i32) -> i32:\n    return value + 1\n\nfn run() -> i32:\n    let callback: fn(i32) -> i32 = increment\n    let add_one: fn(i32) -> i32 = fn [](value: i32) -> i32:\n        return value + 1\n    return callback(add_one(40))\n";
        analyze(&parse(valid), Profile::Hosted, TargetDataLayout::I686)
            .expect("function pointer and noncapturing closure coercion analyze");

        let invalid = "fn run() -> i32:\n    let factor = 2\n    let callback: fn(i32) -> i32 = fn [borrow factor](value: i32) -> i32:\n        return value + factor\n    return 0\n";
        let diagnostics = analyze(&parse(invalid), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("capturing closure cannot coerce to a function pointer");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_TYPE_MISMATCH"));
    }

    #[test]
    fn pointer_field_access_and_usize_only_pointer_casts_are_canonical() {
        let valid = "struct Cursor:\n    position: usize\n\nfn read(cursor: ptr[Cursor]) -> usize:\n    return (*cursor).position\n";
        let hir = analyze(&parse(valid), Profile::I686None, TargetDataLayout::I686)
            .expect("deref field access uses canonical struct identity");
        assert!(hir.dump().contains("LValue"));

        let invalid = "fn bad(value: u32) -> ptr[u8]:\n    return value as ptr[u8]\n";
        let diagnostics = analyze(&parse(invalid), Profile::I686None, TargetDataLayout::I686)
            .expect_err("u32 cannot bypass usize for pointer conversion");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_INVALID_CAST"));
    }

    #[test]
    fn repr_c_and_repr_c_packed_share_the_target_layout_engine() {
        let source = "@repr(C)\nstruct Normal:\n    byte: u8\n    word: u32\n\n@repr(C, packed)\nstruct Packed:\n    byte: u8\n    word: u32\n";
        let hir = analyze(&parse(source), Profile::I686None, TargetDataLayout::I686)
            .expect("repr declarations analyze");
        let normal = hir
            .definitions
            .iter()
            .find(|definition| definition.name == "Normal")
            .unwrap();
        let packed = hir
            .definitions
            .iter()
            .find(|definition| definition.name == "Packed")
            .unwrap();
        assert_eq!(
            layout_aggregate(&hir, normal.symbol, &[], &mut BTreeSet::new()).unwrap(),
            (Layout { size: 8, align: 4 }, vec![0, 4])
        );
        assert_eq!(
            layout_aggregate(&hir, packed.symbol, &[], &mut BTreeSet::new()).unwrap(),
            (Layout { size: 5, align: 1 }, vec![0, 1])
        );
    }

    #[test]
    fn duplicate_and_non_exhaustive_constructs_are_diagnostics() {
        let source = "enum State:\n    Ready\n    Done\n\nfn run(value: State) -> i32:\n    match value:\n        case State.Ready:\n            return 1\n    return 0\n";
        let errors = analyze(&parse(source), Profile::Hosted, TargetDataLayout::I686).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.code == "E_NON_EXHAUSTIVE_MATCH"));
    }

    #[test]
    fn profile_validation_runs_after_canonical_semantics_and_only_removes_capabilities() {
        let interrupt =
            "interrupt(no_error_code) fn irq(frame: ptr[InterruptFrame]):\n    return\n";
        let hosted = analyze(&parse(interrupt), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("hosted profile must reject privileged interrupt declarations");
        assert!(hosted.iter().any(|diagnostic| {
            diagnostic.code == "E_PROFILE_CAPABILITY"
                && diagnostic.message.contains("interrupt functions")
        }));

        let foreign = "extern \"C\" fn c_add(left: i32, right: i32) -> i32\n";
        let vm = analyze(&parse(foreign), Profile::Vm, TargetDataLayout::I686)
            .expect_err("VM profile must reject external C linkage");
        assert!(vm.iter().any(|diagnostic| {
            diagnostic.code == "E_PROFILE_CAPABILITY" && diagnostic.message.contains("extern C")
        }));
        analyze(&parse(foreign), Profile::I686None, TargetDataLayout::I686)
            .expect("i686-none profile admits the bounded C ABI");
    }

    #[test]
    fn raw_view_construction_is_private_to_runtime_entry_points() {
        let source = "fn steal(data: ptr[u8], length: usize) -> slice[u8]:\n    return __dbyte_runtime.slice_from_raw(data, length)\n";
        let diagnostics = analyze(&parse(source), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("application code cannot construct an unchecked borrowed view");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_RUNTIME_CAPABILITY_PRIVATE"));

        let memory = "fn steal(size: usize) -> ptr[u8]:\n    return __dbyte_runtime.memory_alloc(size, 4 as usize)\n";
        let diagnostics = analyze(&parse(memory), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("application code cannot invoke the host provider directly");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_RUNTIME_CAPABILITY_PRIVATE"));
    }

    #[test]
    fn runtime_view_provenance_keeps_the_owner_borrowed() {
        let source = "struct Text:\n    data: ptr[u8]\n    len: usize\n\ndrop fn destroy(value: borrow mut Text):\n    return unit\n\nfn as_view(text: borrow Text) -> StringView:\n    return __dbyte_runtime.string_view_from_raw((*text).data, (*text).len)\n\nfn bad(text: Text) -> unit:\n    let view = as_view(borrow text)\n    drop(text)\n    let length = len(view)\n    return unit\n";
        let diagnostics = analyze(&parse(source), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("a StringView must keep its owner immutably borrowed");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E_MOVE_WHILE_BORROWED" || diagnostic.code == "E_BORROW_CONFLICT"
        }));
    }

    #[test]
    fn user_profile_rejects_native_namespace_and_constant_kernel_pointers() {
        let source = "fn run() -> i32:\n    native.cpu_cli()\n    let pointer: ptr[u8] = 3238002688 as usize as ptr[u8]\n    return 0\n";
        let diagnostics = analyze(&parse(source), Profile::I686User, TargetDataLayout::I686)
            .expect_err("user profile cannot reach native operations or kernel addresses");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E_PROFILE_CAPABILITY"
                && diagnostic.message.contains("native.cpu_cli")
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E_PROFILE_CAPABILITY" && diagnostic.message.contains("kernel range")
        }));
    }

    #[test]
    fn contextual_integer_literals_use_the_declared_width_and_fail_out_of_range() {
        let valid = "fn id[T](value: T) -> T:\n    return value\n\nlet byte: u8 = id[u8](255)\n";
        analyze(&parse(valid), Profile::Hosted, TargetDataLayout::I686)
            .expect("generic arguments contextualize integer literals");

        let invalid = "let byte: u8 = 256\n";
        let diagnostics = analyze(&parse(invalid), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("narrow literal overflow must fail before lowering");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_INTEGER_LITERAL_RANGE"));
    }

    #[test]
    fn v121_drop_functions_are_nominal_unique_and_not_directly_callable() {
        let valid = "struct Resource:\n    id: u32\n\ndrop fn destroy(value: borrow mut Resource):\n    value.id = 0\n\nfn consume(value: Resource) -> unit:\n    drop(value)\n    return unit\n";
        let hir = analyze(&parse(valid), Profile::Hosted, TargetDataLayout::I686)
            .expect("canonical drop declaration analyzes");
        let drop = hir
            .functions
            .iter()
            .find(|function| function.kind == FunctionKind::Drop)
            .expect("drop function recorded");
        assert!(drop.drop_target.is_some());

        let direct = "struct Resource:\n    id: u32\n\ndrop fn destroy(value: borrow mut Resource):\n    value.id = 0\n\nfn bad(value: Resource):\n    destroy(borrow mut value)\n";
        let diagnostics = analyze(&parse(direct), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("drop function must not be directly callable");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_DROP_DIRECT_CALL"));

        let field_move = "struct Child:\n    id: u32\n\ndrop fn destroy_child(value: borrow mut Child):\n    (*value).id = 0 as u32\n\nstruct Parent:\n    child: Child\n\nfn consume(value: Child) -> unit:\n    return unit\n\ndrop fn destroy_parent(value: borrow mut Parent):\n    consume((*value).child)\n";
        let diagnostics = analyze(&parse(field_move), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("a destructor cannot steal a field from structural drop glue");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_DROP_FIELD_MOVE"));
    }

    #[test]
    fn v121_rejects_maybe_moved_control_flow_without_runtime_drop_flags() {
        let source = "struct Resource:\n    id: u32\n\ndrop fn destroy(value: borrow mut Resource):\n    value.id = 0\n\nfn consume(value: Resource) -> unit:\n    return unit\n\nfn branch(flag: bool, value: Resource) -> unit:\n    if flag:\n        consume(value)\n    return unit\n";
        let diagnostics = analyze(&parse(source), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("branch-dependent moves require runtime drop flags and must be rejected");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_OWNERSHIP_MAYBE_MOVED"));
    }
}
