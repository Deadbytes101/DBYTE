use dbyte_ast::*;
use dbyte_lexer::{Token, TokenKind};

// ─── Parse Error ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ParseError {
    pub msg: String,
    pub span: Span,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ParseError at {}: {}", self.span, self.msg)
    }
}

// ─── Parser ──────────────────────────────────────────────────────────────────

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    // ── Token navigation ──────────────────────────────────────────────────

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn advance(&mut self) -> &Token {
        let t = &self.tokens[self.pos];
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<&Token, ParseError> {
        if std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(kind) {
            Ok(self.advance())
        } else {
            Err(ParseError {
                msg: format!("expected `{}`, found `{}`", kind, self.peek_kind()),
                span: self.peek().span,
            })
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek_kind(), TokenKind::Newline) {
            self.advance();
        }
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Eof)
    }

    // ── Type annotation ───────────────────────────────────────────────────

    fn parse_type(&mut self) -> TypeAnnotation {
        if let TokenKind::Ident(name) = self.peek_kind().clone() {
            let ty = match name.as_str() {
                "int"   => TypeAnnotation::Int,
                "float" => TypeAnnotation::Float,
                "bool"  => TypeAnnotation::Bool,
                "str"   => TypeAnnotation::Str,
                _       => TypeAnnotation::Inferred,
            };
            if ty != TypeAnnotation::Inferred {
                self.advance();
            }
            ty
        } else {
            TypeAnnotation::Inferred
        }
    }

    // ── Expressions ───────────────────────────────────────────────────────

    /// entry point — precedence climbing at top level is comparison
    pub fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::EqualEqual   => BinOp::EqEq,
                TokenKind::BangEqual    => BinOp::NotEq,
                TokenKind::Less         => BinOp::Lt,
                TokenKind::LessEqual    => BinOp::LtEq,
                TokenKind::Greater      => BinOp::Gt,
                TokenKind::GreaterEqual => BinOp::GtEq,
                _ => break,
            };
            let span = self.peek().span;
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Plus  => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            let span = self.peek().span;
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Star  => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                _ => break,
            };
            let span = self.peek().span;
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek_kind() {
            TokenKind::Minus => {
                let span = self.peek().span;
                self.advance();
                let e = self.parse_call()?;
                Ok(Expr::Unary { op: UnaryOp::Neg, expr: Box::new(e), span })
            }
            TokenKind::Bang => {
                let span = self.peek().span;
                self.advance();
                let e = self.parse_call()?;
                Ok(Expr::Unary { op: UnaryOp::Not, expr: Box::new(e), span })
            }
            _ => self.parse_call(),
        }
    }

    fn parse_call(&mut self) -> Result<Expr, ParseError> {
        let base = self.parse_primary()?;

        // if next token is `(` this is a function call
        if let Expr::Ident(name, span) = &base {
            if matches!(self.peek_kind(), TokenKind::LParen) {
                let span = *span;
                let name = name.clone();
                self.advance(); // consume `(`
                let mut args = Vec::new();
                while !matches!(self.peek_kind(), TokenKind::RParen | TokenKind::Eof) {
                    args.push(self.parse_expr()?);
                    if matches!(self.peek_kind(), TokenKind::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.expect(&TokenKind::RParen)?;
                return Ok(Expr::Call { name, args, span });
            }
        }
        Ok(base)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Int(n) => {
                self.advance();
                Ok(Expr::IntLit(n, tok.span))
            }
            TokenKind::Float(n) => {
                self.advance();
                Ok(Expr::FloatLit(n, tok.span))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::BoolLit(true, tok.span))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::BoolLit(false, tok.span))
            }
            TokenKind::Str(ref s) => {
                let s = s.clone();
                self.advance();
                Ok(Expr::StrLit(s, tok.span))
            }
            TokenKind::Ident(ref name) => {
                let name = name.clone();
                self.advance();
                Ok(Expr::Ident(name, tok.span))
            }
            TokenKind::LParen => {
                self.advance(); // consume `(`
                let e = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(e)
            }
            _ => Err(ParseError {
                msg: format!("unexpected token `{}`", tok.kind),
                span: tok.span,
            }),
        }
    }

    // ── Statements ────────────────────────────────────────────────────────

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.skip_newlines();
        match self.peek_kind().clone() {
            TokenKind::Let    => self.parse_let(),
            TokenKind::Fn     => self.parse_fn(),
            TokenKind::Return => self.parse_return(),
            TokenKind::If     => self.parse_if(),
            TokenKind::Ident(ref name) => {
                // could be assignment  `name = expr`  or expression `name(...)...`
                let name = name.clone();
                let span = self.peek().span;
                self.advance();
                if matches!(self.peek_kind(), TokenKind::Equal) {
                    // assignment
                    self.advance(); // consume `=`
                    let value = self.parse_expr()?;
                    self.consume_newline_or_eof();
                    Ok(Stmt::Assign { name, value, span })
                } else {
                    // put the ident back conceptually by building an expr from it
                    let ident_expr = Expr::Ident(name.clone(), span);
                    // check for call
                    let expr = if matches!(self.peek_kind(), TokenKind::LParen) {
                        self.advance(); // consume `(`
                        let mut args = Vec::new();
                        while !matches!(self.peek_kind(), TokenKind::RParen | TokenKind::Eof) {
                            args.push(self.parse_expr()?);
                            if matches!(self.peek_kind(), TokenKind::Comma) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        self.expect(&TokenKind::RParen)?;
                        Expr::Call { name, args, span }
                    } else {
                        // binary / comparison starting from ident
                        self.finish_expr_from(ident_expr)?
                    };
                    self.consume_newline_or_eof();
                    Ok(Stmt::Expr(expr))
                }
            }
            _ => {
                let expr = self.parse_expr()?;
                self.consume_newline_or_eof();
                Ok(Stmt::Expr(expr))
            }
        }
    }

    /// Continue parsing a binary expression when we've already consumed the lhs
    fn finish_expr_from(&mut self, lhs: Expr) -> Result<Expr, ParseError> {
        // just try additive/comparison wrapping lhs
        let mut left = lhs;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Plus          => BinOp::Add,
                TokenKind::Minus         => BinOp::Sub,
                TokenKind::Star          => BinOp::Mul,
                TokenKind::Slash         => BinOp::Div,
                TokenKind::EqualEqual    => BinOp::EqEq,
                TokenKind::BangEqual     => BinOp::NotEq,
                TokenKind::Less          => BinOp::Lt,
                TokenKind::LessEqual     => BinOp::LtEq,
                TokenKind::Greater       => BinOp::Gt,
                TokenKind::GreaterEqual  => BinOp::GtEq,
                _ => break,
            };
            let span = self.peek().span;
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span };
        }
        Ok(left)
    }

    fn consume_newline_or_eof(&mut self) {
        if matches!(self.peek_kind(), TokenKind::Newline) {
            self.advance();
        }
    }

    fn parse_let(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance(); // consume `let`
        let name = match self.peek_kind().clone() {
            TokenKind::Ident(n) => { self.advance(); n }
            _ => return Err(ParseError { msg: "expected identifier after `let`".into(), span: self.peek().span }),
        };
        // optional `: type`
        let ty = if matches!(self.peek_kind(), TokenKind::Colon) {
            self.advance(); // consume `:`
            self.parse_type()
        } else {
            TypeAnnotation::Inferred
        };
        self.expect(&TokenKind::Equal)?;
        let value = self.parse_expr()?;
        self.consume_newline_or_eof();
        Ok(Stmt::Let { name, ty, value, span })
    }

    fn parse_fn(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance(); // consume `fn`
        let name = match self.peek_kind().clone() {
            TokenKind::Ident(n) => { self.advance(); n }
            _ => return Err(ParseError { msg: "expected function name".into(), span: self.peek().span }),
        };
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::RParen | TokenKind::Eof) {
            let p_span = self.peek().span;
            let p_name = match self.peek_kind().clone() {
                TokenKind::Ident(n) => { self.advance(); n }
                _ => return Err(ParseError { msg: "expected parameter name".into(), span: self.peek().span }),
            };
            self.expect(&TokenKind::Colon)?;
            let ty = self.parse_type();
            params.push(Param { name: p_name, ty, span: p_span });
            if matches!(self.peek_kind(), TokenKind::Comma) { self.advance(); }
        }
        self.expect(&TokenKind::RParen)?;

        // optional `-> ret_type`
        let ret_ty = if matches!(self.peek_kind(), TokenKind::Arrow) {
            self.advance();
            self.parse_type()
        } else {
            TypeAnnotation::Inferred
        };

        self.expect(&TokenKind::Colon)?;
        self.consume_newline_or_eof();

        // body is an indented block
        let body = self.parse_block()?;
        Ok(Stmt::FnDef { name, params, ret_ty, body, span })
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance(); // consume `return`
        if matches!(self.peek_kind(), TokenKind::Newline | TokenKind::Eof) {
            self.consume_newline_or_eof();
            return Ok(Stmt::Return { value: None, span });
        }
        let value = self.parse_expr()?;
        self.consume_newline_or_eof();
        Ok(Stmt::Return { value: Some(value), span })
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance(); // consume `if`
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::Colon)?;
        self.consume_newline_or_eof();
        let then_body = self.parse_block()?;

        let else_body = if matches!(self.peek_kind(), TokenKind::Else) {
            self.advance(); // consume `else`
            self.expect(&TokenKind::Colon)?;
            self.consume_newline_or_eof();
            Some(self.parse_block()?)
        } else {
            None
        };

        Ok(Stmt::If { cond, then_body, else_body, span })
    }

    /// Parse an indented block: INDENT stmts DEDENT
    fn parse_block(&mut self) -> Result<Vec<Stmt>, ParseError> {
        self.skip_newlines();
        if !matches!(self.peek_kind(), TokenKind::Indent) {
            return Err(ParseError {
                msg: "expected indented block".into(),
                span: self.peek().span,
            });
        }
        self.advance(); // consume INDENT

        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek_kind(), TokenKind::Dedent | TokenKind::Eof) {
                break;
            }
            stmts.push(self.parse_stmt()?);
        }

        if matches!(self.peek_kind(), TokenKind::Dedent) {
            self.advance(); // consume DEDENT
        }
        Ok(stmts)
    }

    // ── Top-level parse ───────────────────────────────────────────────────

    pub fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_eof() { break; }
            stmts.push(self.parse_stmt()?);
        }
        Ok(Program { stmts })
    }
}
