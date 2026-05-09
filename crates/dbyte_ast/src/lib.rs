/// Span: ตำแหน่งใน source code (สำหรับ error messages)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

impl Span {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

// ─── Type Annotations ────────────────────────────────────────────────────────

/// Type annotation ที่ user เขียนใน source
#[derive(Debug, Clone, PartialEq)]
pub enum TypeAnnotation {
    Int,
    Float,
    Bool,
    Str,
    Inferred, // ไม่มี annotation → infer ทีหลัง
}

impl std::fmt::Display for TypeAnnotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeAnnotation::Int     => write!(f, "int"),
            TypeAnnotation::Float   => write!(f, "float"),
            TypeAnnotation::Bool    => write!(f, "bool"),
            TypeAnnotation::Str     => write!(f, "str"),
            TypeAnnotation::Inferred => write!(f, "<inferred>"),
        }
    }
}

// ─── Expressions ─────────────────────────────────────────────────────────────

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
            BinOp::Add  => "+",  BinOp::Sub  => "-",
            BinOp::Mul  => "*",  BinOp::Div  => "/",
            BinOp::EqEq => "==", BinOp::NotEq => "!=",
            BinOp::Lt   => "<",  BinOp::Gt   => ">",
            BinOp::LtEq => "<=", BinOp::GtEq => ">=",
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
pub enum Expr {
    /// Integer literal: `42`
    IntLit(i64, Span),
    /// Float literal: `3.14`
    FloatLit(f64, Span),
    /// Bool literal: `true` / `false`
    BoolLit(bool, Span),
    /// String literal: `"hello"`
    StrLit(String, Span),
    /// Variable reference: `x`
    Ident(String, Span),
    /// Binary expression: `a + b`
    Binary {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
        span: Span,
    },
    /// Unary expression: `-x`, `!b`
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    /// Function call: `print(x + 1)`
    Call {
        name: String,
        args: Vec<Expr>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::IntLit(_, s)    => *s,
            Expr::FloatLit(_, s)  => *s,
            Expr::BoolLit(_, s)   => *s,
            Expr::StrLit(_, s)    => *s,
            Expr::Ident(_, s)     => *s,
            Expr::Binary { span, .. } => *span,
            Expr::Unary  { span, .. } => *span,
            Expr::Call   { span, .. } => *span,
        }
    }
}

// ─── Statements ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: TypeAnnotation,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `let name: type = expr`
    Let {
        name: String,
        ty: TypeAnnotation,
        value: Expr,
        span: Span,
    },
    /// `name = expr`  (assignment)
    Assign {
        name: String,
        value: Expr,
        span: Span,
    },
    /// `fn name(params) -> ret: body`
    FnDef {
        name: String,
        params: Vec<Param>,
        ret_ty: TypeAnnotation,
        body: Vec<Stmt>,
        span: Span,
    },
    /// `return expr`
    Return {
        value: Option<Expr>,
        span: Span,
    },
    /// `if cond: body else: body`
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
        span: Span,
    },
    /// expression as statement: `print(x)`
    Expr(Expr),
}

// ─── Program ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}
