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
    Float,
    Bool,
    Str,
    Bytes,
    Buffer,
    List(Box<TypeAnnotation>),
    Inferred,
}

impl std::fmt::Display for TypeAnnotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeAnnotation::Int => write!(f, "int"),
            TypeAnnotation::Float => write!(f, "float"),
            TypeAnnotation::Bool => write!(f, "bool"),
            TypeAnnotation::Str => write!(f, "str"),
            TypeAnnotation::Bytes => write!(f, "bytes"),
            TypeAnnotation::Buffer => write!(f, "buffer"),
            TypeAnnotation::List(inner) => write!(f, "list[{}]", inner),
            TypeAnnotation::Inferred => write!(f, "<inferred>"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    EqEq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
}

impl std::fmt::Display for BinOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::EqEq => "==",
            BinOp::NotEq => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::LtEq => "<=",
            BinOp::GtEq => ">=",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
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
            | Expr::Ident(_, s)
            | Expr::List(_, s) => *s,
            Expr::Binary { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Call { span, .. }
            | Expr::Index { span, .. }
            | Expr::Member { span, .. }
            | Expr::MemberCall { span, .. } => *span,
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
    FnDef {
        is_pub: bool,
        name: String,
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
    Expr(Expr),
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. } => *span,
            Stmt::Assign { span, .. } => *span,
            Stmt::FnDef { span, .. } => *span,
            Stmt::Return { span, .. } => *span,
            Stmt::If { span, .. } => *span,
            Stmt::While { span, .. } => *span,
            Stmt::For { span, .. } => *span,
            Stmt::Import { span, .. } => *span,
            Stmt::Expr(e) => e.span(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}
