#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

impl Span {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
    pub fn zero() -> Self {
        Self { line: 1, col: 1 }
    }
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeAnnotation {
    Int,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    Usize,
    Float,
    Bool,
    Str,
    Bytes,
    Buffer,
    List(Box<TypeAnnotation>),
    Pointer(Box<TypeAnnotation>),
    ConstPointer(Box<TypeAnnotation>),
    Slice(Box<TypeAnnotation>),
    MutSlice(Box<TypeAnnotation>),
    StringView,
    Array {
        element: Box<TypeAnnotation>,
        length: u32,
    },
    Named(String),
    Generic {
        base: String,
        args: Vec<TypeAnnotation>,
    },
    Function {
        convention: CallConvention,
        params: Vec<TypeAnnotation>,
        ret: Box<TypeAnnotation>,
    },
    Closure {
        params: Vec<TypeAnnotation>,
        ret: Box<TypeAnnotation>,
    },
    Inferred,
}

impl std::fmt::Display for TypeAnnotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeAnnotation::Int => write!(f, "int"),
            TypeAnnotation::I8 => write!(f, "i8"),
            TypeAnnotation::U8 => write!(f, "u8"),
            TypeAnnotation::I16 => write!(f, "i16"),
            TypeAnnotation::U16 => write!(f, "u16"),
            TypeAnnotation::I32 => write!(f, "i32"),
            TypeAnnotation::U32 => write!(f, "u32"),
            TypeAnnotation::Usize => write!(f, "usize"),
            TypeAnnotation::Float => write!(f, "float"),
            TypeAnnotation::Bool => write!(f, "bool"),
            TypeAnnotation::Str => write!(f, "str"),
            TypeAnnotation::Bytes => write!(f, "bytes"),
            TypeAnnotation::Buffer => write!(f, "buffer"),
            TypeAnnotation::List(inner) => write!(f, "list[{}]", inner),
            TypeAnnotation::Pointer(inner) => write!(f, "ptr[{}]", inner),
            TypeAnnotation::ConstPointer(inner) => write!(f, "const_ptr[{}]", inner),
            TypeAnnotation::Slice(inner) => write!(f, "slice[{}]", inner),
            TypeAnnotation::MutSlice(inner) => write!(f, "mut_slice[{}]", inner),
            TypeAnnotation::StringView => write!(f, "StringView"),
            TypeAnnotation::Array { element, length } => {
                write!(f, "array[{}, {}]", element, length)
            }
            TypeAnnotation::Named(name) => f.write_str(name),
            TypeAnnotation::Generic { base, args } => {
                write!(f, "{base}[")?;
                for (index, arg) in args.iter().enumerate() {
                    if index != 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{arg}")?;
                }
                write!(f, "]")
            }
            TypeAnnotation::Function {
                convention,
                params,
                ret,
            } => {
                if *convention == CallConvention::C {
                    write!(f, "extern \"C\" ")?;
                }
                write!(f, "fn(")?;
                for (index, param) in params.iter().enumerate() {
                    if index != 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{param}")?;
                }
                write!(f, ") -> {ret}")
            }
            TypeAnnotation::Closure { params, ret } => {
                write!(f, "closure(")?;
                for (index, param) in params.iter().enumerate() {
                    if index != 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{param}")?;
                }
                write!(f, ") -> {ret}")
            }
            TypeAnnotation::Inferred => write!(f, "<inferred>"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CallConvention {
    DByte,
    C,
}

impl std::fmt::Display for CallConvention {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::DByte => "DByte",
            Self::C => "C",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    Borrow,
    BorrowMut,
    Move,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Capture {
    pub mode: CaptureMode,
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NamedArgument {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    EqEq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl std::fmt::Display for BinOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::EqEq => "==",
            BinOp::NotEq => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::LtEq => "<=",
            BinOp::GtEq => ">=",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
    AddressOf,
    Deref,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FStrPart {
    Literal(String),
    Interp(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    IntLit(i64, Span),
    FloatLit(f64, Span),
    BoolLit(bool, Span),
    StrLit(String, Span),
    BytesLit(Vec<u8>, Span),
    FStr(Vec<FStrPart>, Span),
    NullLit(Span),
    Ident(String, Span),
    List(Vec<Expr>, Span),
    Binary {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    Call {
        name: String,
        args: Vec<Expr>,
        span: Span,
    },
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    Member {
        object: Box<Expr>,
        property: String,
        span: Span,
    },
    MemberCall {
        object: Box<Expr>,
        property: String,
        args: Vec<Expr>,
        span: Span,
    },
    GenericApply {
        target: Box<Expr>,
        args: Vec<TypeAnnotation>,
        span: Span,
    },
    Invoke {
        callee: Box<Expr>,
        args: Vec<Expr>,
        named_args: Vec<NamedArgument>,
        span: Span,
    },
    Borrow {
        mutable: bool,
        expr: Box<Expr>,
        span: Span,
    },
    Try {
        expr: Box<Expr>,
        span: Span,
    },
    Closure {
        captures: Vec<Capture>,
        params: Vec<Param>,
        ret_ty: TypeAnnotation,
        body: Vec<Stmt>,
        span: Span,
    },
    Cast {
        expr: Box<Expr>,
        ty: TypeAnnotation,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::IntLit(_, s)
            | Expr::FloatLit(_, s)
            | Expr::BoolLit(_, s)
            | Expr::StrLit(_, s)
            | Expr::BytesLit(_, s)
            | Expr::FStr(_, s)
            | Expr::NullLit(s)
            | Expr::Ident(_, s)
            | Expr::List(_, s) => *s,
            Expr::Binary { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Call { span, .. }
            | Expr::GenericApply { span, .. }
            | Expr::Invoke { span, .. }
            | Expr::Borrow { span, .. }
            | Expr::Try { span, .. }
            | Expr::Closure { span, .. }
            | Expr::Index { span, .. }
            | Expr::Member { span, .. }
            | Expr::MemberCall { span, .. }
            | Expr::Cast { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: TypeAnnotation,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructField {
    pub name: String,
    pub ty: TypeAnnotation,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    pub payload: Vec<TypeAnnotation>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Wildcard(Span),
    Variant {
        path: String,
        bindings: Vec<String>,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    Move,
    Borrow,
    BorrowMut,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repr {
    DByte,
    C,
    CPacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptClass {
    NoErrorCode,
    ErrorCode,
    NoErrorCodeCpl3Transition,
    ErrorCodeCpl3Transition,
}

impl InterruptClass {
    pub const fn has_error_code(self) -> bool {
        matches!(self, Self::ErrorCode | Self::ErrorCodeCpl3Transition)
    }

    pub const fn cpl3_transition(self) -> bool {
        matches!(
            self,
            Self::NoErrorCodeCpl3Transition | Self::ErrorCodeCpl3Transition
        )
    }
}

impl std::fmt::Display for InterruptClass {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoErrorCode => "no_error_code",
            Self::ErrorCode => "error_code",
            Self::NoErrorCodeCpl3Transition => "no_error_code,cpl3_transition",
            Self::ErrorCodeCpl3Transition => "error_code,cpl3_transition",
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        is_pub: bool,
        name: String,
        ty: TypeAnnotation,
        value: Expr,
        span: Span,
    },
    Assign {
        name: String,
        value: Expr,
        span: Span,
    },
    /// Assignment through a native lvalue such as `*ptr`, `array[index]`, or
    /// `record.field`. Simple identifier assignment remains `Assign` so hosted
    /// consumers retain their existing AST contract.
    Store {
        target: Expr,
        value: Expr,
        span: Span,
    },
    FnDef {
        is_pub: bool,
        interrupt: Option<InterruptClass>,
        call_convention: CallConvention,
        name: String,
        generic_params: Vec<String>,
        params: Vec<Param>,
        ret_ty: TypeAnnotation,
        body: Vec<Stmt>,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    For {
        var: String,
        iterable: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    Import {
        path: String,
        alias: String,
        span: Span,
    },
    StructDef {
        is_pub: bool,
        is_packed: bool,
        repr: Repr,
        name: String,
        generic_params: Vec<String>,
        fields: Vec<StructField>,
        span: Span,
    },
    EnumDef {
        is_pub: bool,
        name: String,
        generic_params: Vec<String>,
        variants: Vec<EnumVariant>,
        span: Span,
    },
    Match {
        mode: MatchMode,
        value: Expr,
        arms: Vec<MatchArm>,
        span: Span,
    },
    Defer {
        captures: Vec<Capture>,
        body: Vec<Stmt>,
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
    Const {
        is_pub: bool,
        name: String,
        ty: TypeAnnotation,
        value: Expr,
        span: Span,
    },
    Static {
        is_pub: bool,
        is_mut: bool,
        name: String,
        ty: TypeAnnotation,
        value: Option<Expr>,
        span: Span,
    },
    Expr(Expr),
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. } => *span,
            Stmt::Assign { span, .. } => *span,
            Stmt::Store { span, .. } => *span,
            Stmt::FnDef { span, .. } => *span,
            Stmt::Return { span, .. } => *span,
            Stmt::If { span, .. } => *span,
            Stmt::While { span, .. } => *span,
            Stmt::For { span, .. } => *span,
            Stmt::Import { span, .. } => *span,
            Stmt::StructDef { span, .. } => *span,
            Stmt::EnumDef { span, .. } => *span,
            Stmt::Match { span, .. } => *span,
            Stmt::Defer { span, .. } => *span,
            Stmt::Break { span } => *span,
            Stmt::Continue { span } => *span,
            Stmt::Const { span, .. } => *span,
            Stmt::Static { span, .. } => *span,
            Stmt::Expr(e) => e.span(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systems_type_annotations_have_stable_source_spelling() {
        let ty = TypeAnnotation::Pointer(Box::new(TypeAnnotation::Array {
            element: Box::new(TypeAnnotation::U16),
            length: 4,
        }));
        assert_eq!(ty.to_string(), "ptr[array[u16, 4]]");
        assert_eq!(TypeAnnotation::Named("Header".into()).to_string(), "Header");
    }

    #[test]
    fn systems_operators_have_stable_source_spelling() {
        assert_eq!(BinOp::BitAnd.to_string(), "&");
        assert_eq!(BinOp::BitOr.to_string(), "|");
        assert_eq!(BinOp::BitXor.to_string(), "^");
        assert_eq!(BinOp::Shl.to_string(), "<<");
        assert_eq!(BinOp::Shr.to_string(), ">>");
        assert_eq!(BinOp::Rem.to_string(), "%");
    }
}
