//! Canonical, target-parameterized semantic model for DByte.
//!
//! This crate deliberately contains no interpreter values, VM opcodes, x86
//! registers, relocation kinds, or host-sized layout decisions.

use dbyte_ast::{
    BinOp, CallConvention, CaptureMode, Expr, MatchMode, Pattern, Program, Repr, Span, Stmt,
    TypeAnnotation, UnaryOp,
};
use std::collections::{BTreeMap, BTreeSet};

mod executable;
pub use executable::*;
mod project;

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
                "expr {} t{} {:?} {:?}\n",
                expression.span, expression.ty.0, expression.category, expression.ownership
            ));
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
    cleanup_edges: Vec<CleanupEdge>,
    resolved_addresses: BTreeMap<(usize, usize), ResolvedAddress>,
    active_defers: Vec<(Span, u32)>,
    lexical_depth: u32,
    borrowed_parameters: BTreeSet<String>,
    current_generic: Option<GenericDefId>,
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
            cleanup_edges: Vec::new(),
            resolved_addresses: BTreeMap::new(),
            active_defers: Vec::new(),
            lexical_depth: 0,
            borrowed_parameters: BTreeSet::new(),
            current_generic: None,
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
            Expr::IntLit(..)
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
                        convention: *call_convention,
                        interrupt: *interrupt,
                        declaration_only: body.is_empty(),
                        generic_params: generic_params.clone(),
                        params: params.iter().map(|param| param.ty.clone()).collect(),
                        result: ret_ty.clone(),
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
                        self.globals.push(GlobalDef {
                            symbol,
                            name: name.clone(),
                            public: *is_pub,
                            mutable: false,
                            constant: true,
                            annotation: ty.clone(),
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
                        self.globals.push(GlobalDef {
                            symbol,
                            name: name.clone(),
                            public: *is_pub,
                            mutable: *is_mut,
                            constant: false,
                            annotation: ty.clone(),
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
                .find(|function| function.name == *name)
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
            for parameter in params {
                if let Some(ty) = self.resolve_type(&parameter.ty, &generics, parameter.span) {
                    if matches!(
                        self.types.values[ty.0 as usize],
                        CanonicalType::Slice { .. }
                            | CanonicalType::StringView
                            | CanonicalType::Borrow { .. }
                    ) {
                        self.borrowed_parameters.insert(parameter.name.clone());
                    }
                    ownership.define(parameter.name.clone(), ty, parameter.span);
                }
            }
            self.current_return = function
                .as_ref()
                .and_then(|function| self.resolve_type(&function.result, &generics, function.span));
            self.current_interrupt = function.as_ref().and_then(|function| function.interrupt);
            self.current_generic = function.as_ref().and_then(|function| function.generic);
            self.check_block(body, &generics, &mut ownership, false);
            self.current_return = None;
            self.current_interrupt = None;
            self.current_generic = None;
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
                    }
                }
                Stmt::Assign { name, value, span } => {
                    let actual = self.check_expr(value, generics, ownership, true);
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
                                format!("assignment to `{name}` has the wrong type"),
                                *span,
                            );
                        } else {
                            binding.moved = false;
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
                                format!("unknown binding `{name}`"),
                                *span,
                            );
                        }
                    }
                }
                Stmt::Return { value, span } => {
                    self.record_cleanup_edges(CleanupExit::Return, *span, 0);
                    if let Some(value) = value {
                        if self.current_return.is_some_and(|return_type| {
                            matches!(
                                self.types.values[return_type.0 as usize],
                                CanonicalType::Slice { .. }
                                    | CanonicalType::StringView
                                    | CanonicalType::Borrow { .. }
                            )
                        }) {
                            let direct_source = matches!(
                                value,
                                Expr::Ident(name, _) if self.borrowed_parameters.contains(name)
                            );
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
                                        "return value has type t{}, expected t{}",
                                        actual.0, expected.0
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
                    ..
                } => {
                    let condition = self.check_expr(cond, generics, ownership, false);
                    if condition.is_some_and(|ty| {
                        !matches!(self.types.values[ty.0 as usize], CanonicalType::Bool)
                    }) {
                        self.diagnostic(
                            "E_CONDITION_TYPE",
                            "if condition must be bool",
                            cond.span(),
                        );
                    }
                    self.check_block(then_body, generics, ownership, in_loop);
                    if let Some(body) = else_body {
                        self.check_block(body, generics, ownership, in_loop);
                    }
                }
                Stmt::While { cond, body, .. } => {
                    let condition = self.check_expr(cond, generics, ownership, false);
                    if condition.is_some_and(|ty| {
                        !matches!(self.types.values[ty.0 as usize], CanonicalType::Bool)
                    }) {
                        self.diagnostic(
                            "E_CONDITION_TYPE",
                            "while condition must be bool",
                            cond.span(),
                        );
                    }
                    self.check_block(body, generics, ownership, true);
                }
                Stmt::For { iterable, body, .. } => {
                    self.check_expr(iterable, generics, ownership, false);
                    self.check_block(body, generics, ownership, true);
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
                    self.check_expr(expression, generics, ownership, false);
                }
                Stmt::Store { target, value, .. } => {
                    self.check_expr(target, generics, ownership, false);
                    self.check_expr(value, generics, ownership, true);
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
            self.expressions.push(HirExpression {
                span: expression.span(),
                ty,
                category,
                ownership: action,
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
                    });
                    return Some(expected);
                }
            }
            let contextual = matches!(
                (&self.types.values[expected.0 as usize], expression),
                (CanonicalType::Pointer { .. }, Expr::NullLit(_))
                    | (CanonicalType::LegacyStr, Expr::StrLit(..))
            );
            if contextual {
                self.expressions.push(HirExpression {
                    span: expression.span(),
                    ty: expected,
                    category: ValueCategory::RValue,
                    ownership: OwnershipAction::Copy,
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
            Expr::IntLit(..) => Some(self.types.intern(CanonicalType::Integer(IntegerType::I32))),
            Expr::FloatLit(..) => Some(self.types.intern(CanonicalType::Float)),
            Expr::BoolLit(..) => Some(self.types.intern(CanonicalType::Bool)),
            Expr::StrLit(..) => Some(self.types.intern(CanonicalType::StringView)),
            Expr::BytesLit(..) => Some(self.types.intern(CanonicalType::LegacyBytes)),
            Expr::FStr(_, span) => {
                self.diagnostic(
                    "E_LEGACY_TEXT",
                    "formatted strings retain legacy str semantics and are not StringView",
                    *span,
                );
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
                        format!("unknown binding `{name}`"),
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
                        format!("unknown binding `{name}`"),
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
                let left_is_integer_literal = is_integer_literal_expression(left);
                let (left, right) = if left_is_integer_literal {
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
                self.check_expr(index, generics, ownership, false);
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
                    CanonicalType::StringView => {
                        Some(self.types.intern(CanonicalType::Integer(IntegerType::U8)))
                    }
                    _ => {
                        self.diagnostic(
                            "E_NOT_INDEXABLE",
                            "value is not an array or bounded slice",
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

    fn check_intrinsic_call(
        &mut self,
        namespace: &str,
        intrinsic: HirIntrinsic,
        args: &[Expr],
        generics: &BTreeMap<String, TypeId>,
        ownership: &mut Ownership,
        span: Span,
    ) -> Option<TypeId> {
        let admitted = match self.profile {
            Profile::I686None => namespace == "native",
            Profile::I686User => namespace == "user",
            Profile::Hosted | Profile::Vm => false,
        };
        if !admitted || intrinsic.is_user() != (namespace == "user") {
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
                            cleanup_edges: Vec::new(),
                            resolved_addresses: BTreeMap::new(),
                            interface_fingerprint: 0,
                        },
                        ty,
                    ) {
                        self.diagnostics.push(Diagnostic { span, ..error });
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
            for argument in args {
                self.check_expr(argument, generics, ownership, false);
            }
            return Some(
                self.types
                    .intern(CanonicalType::Integer(IntegerType::Usize)),
            );
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
                self.diagnostic(
                    "E_ARGUMENT_COUNT",
                    format!(
                        "function `{name}` expects {} arguments",
                        function.params.len()
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
            return self.resolve_type(&function.result, &substitutions, span);
        }
        for argument in args {
            self.check_expr(argument, generics, ownership, true);
        }
        self.diagnostic(
            "E_UNKNOWN_SYMBOL",
            format!("unknown callable `{name}`"),
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
        let CanonicalType::Struct { symbol, args } =
            self.types.values[object_type.0 as usize].clone()
        else {
            self.diagnostic(
                "E_FIELD_BASE_TYPE",
                "field access requires a struct lvalue",
                span,
            );
            return None;
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
            || matches!(self.profile, Profile::I686None | Profile::I686User)
                && matches!(
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
            | CanonicalType::LegacyStr => true,
            CanonicalType::Array { element, .. } => self.is_copy_inner(*element, active),
            CanonicalType::Struct { symbol, args } | CanonicalType::Enum { symbol, args } => {
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
            CanonicalType::LegacyBytes
            | CanonicalType::LegacyBuffer
            | CanonicalType::LegacyList(_)
            | CanonicalType::ClosureShape { .. }
            | CanonicalType::GenericParam { .. } => false,
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
            TypeAnnotation::Int
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

#[derive(Debug, Clone)]
struct Binding {
    ty: TypeId,
    moved: bool,
    immut_borrows: u32,
    mut_borrowed: bool,
    #[allow(dead_code)]
    span: Span,
}

#[derive(Default)]
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
    let field_ty = resolved_annotation(program, &fields[index].annotation, &substitutions)?;
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
        TypeAnnotation::Pointer(_) | TypeAnnotation::ConstPointer(_) | TypeAnnotation::Str => {
            Some(Layout {
                size: program.layout.pointer_size,
                align: program.layout.pointer_align,
            })
        }
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
        (TypeAnnotation::Int, CanonicalType::Integer(IntegerType::LegacyInt))
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
        analyze(&parse(valid), Profile::Hosted, TargetDataLayout::I686)
            .expect("single direct borrowed parameter can be returned");

        let ambiguous =
            "fn view(left: slice[u8], right: slice[u8]) -> slice[u8]:\n    return left\n";
        let diagnostics = analyze(&parse(ambiguous), Profile::Hosted, TargetDataLayout::I686)
            .expect_err("ambiguous borrowed return must fail");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_AMBIGUOUS_BORROWED_RETURN"));
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
}
