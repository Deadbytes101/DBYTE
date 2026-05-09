use dbyte_ast::*;
use dbyte_lexer::{Token, TokenKind};

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

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token { &self.tokens[self.pos] }
    fn peek_kind(&self) -> &TokenKind { &self.tokens[self.pos].kind }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    fn advance(&mut self) -> &Token {
        let t = &self.tokens[self.pos];
        if self.pos + 1 < self.tokens.len() { self.pos += 1; }
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
        while matches!(self.peek_kind(), TokenKind::Newline) { self.advance(); }
    }

    fn at_eof(&self) -> bool { matches!(self.peek_kind(), TokenKind::Eof) }

    fn consume_newline_or_eof(&mut self) {
        if matches!(self.peek_kind(), TokenKind::Newline) { self.advance(); }
    }

    fn parse_type_full(&mut self) -> TypeAnnotation {
        if let TokenKind::Ident(name) = self.peek_kind().clone() {
            match name.as_str() {
                "int"   => { self.advance(); TypeAnnotation::Int }
                "float" => { self.advance(); TypeAnnotation::Float }
                "bool"  => { self.advance(); TypeAnnotation::Bool }
                "str"   => { self.advance(); TypeAnnotation::Str }
                "list"  => {
                    self.advance();
                    if matches!(self.peek_kind(), TokenKind::LBracket) {
                        self.advance();
                        let inner = self.parse_type_full();
                        let _ = self.expect(&TokenKind::RBracket);
                        TypeAnnotation::List(Box::new(inner))
                    } else {
                        TypeAnnotation::Inferred
                    }
                }
                _ => TypeAnnotation::Inferred,
            }
        } else {
            TypeAnnotation::Inferred
        }
    }

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
                let span = self.peek().span; self.advance();
                Ok(Expr::Unary { op: UnaryOp::Neg, expr: Box::new(self.parse_postfix()?), span })
            }
            TokenKind::Bang => {
                let span = self.peek().span; self.advance();
                Ok(Expr::Unary { op: UnaryOp::Not, expr: Box::new(self.parse_postfix()?), span })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut base = self.parse_primary()?;
        loop {
            match self.peek_kind() {
                TokenKind::LParen => {
                    if let Expr::Ident(name, span) = &base {
                        let name = name.clone();
                        let span = *span;
                        self.advance();
                        let mut args = Vec::new();
                        while !matches!(self.peek_kind(), TokenKind::RParen | TokenKind::Eof) {
                            args.push(self.parse_expr()?);
                            if matches!(self.peek_kind(), TokenKind::Comma) { self.advance(); } else { break; }
                        }
                        self.expect(&TokenKind::RParen)?;
                        base = Expr::Call { name, args, span };
                    } else {
                        break;
                    }
                }
                TokenKind::LBracket => {
                    let span = self.peek().span;
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket)?;
                    base = Expr::Index { target: Box::new(base), index: Box::new(index), span };
                }
                _ => break,
            }
        }
        Ok(base)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Int(n)   => { self.advance(); Ok(Expr::IntLit(n, tok.span)) }
            TokenKind::Float(n) => { self.advance(); Ok(Expr::FloatLit(n, tok.span)) }
            TokenKind::True     => { self.advance(); Ok(Expr::BoolLit(true, tok.span)) }
            TokenKind::False    => { self.advance(); Ok(Expr::BoolLit(false, tok.span)) }
            TokenKind::Str(ref s) => {
                let s = s.clone();
                self.advance();
                let parts = parse_fstr_parts(&s);
                let all_literal = parts.len() == 1 && matches!(&parts[0], FStrPart::Literal(_));
                if all_literal || parts.is_empty() {
                    Ok(Expr::StrLit(s, tok.span))
                } else {
                    Ok(Expr::FStr(parts, tok.span))
                }
            }
            TokenKind::Ident(ref name) => {
                let name = name.clone();
                self.advance();
                Ok(Expr::Ident(name, tok.span))
            }
            TokenKind::LParen => {
                self.advance();
                let e = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(e)
            }
            TokenKind::LBracket => {
                self.advance();
                let mut elems = Vec::new();
                while !matches!(self.peek_kind(), TokenKind::RBracket | TokenKind::Eof) {
                    elems.push(self.parse_expr()?);
                    if matches!(self.peek_kind(), TokenKind::Comma) { self.advance(); } else { break; }
                }
                self.expect(&TokenKind::RBracket)?;
                Ok(Expr::List(elems, tok.span))
            }
            _ => Err(ParseError {
                msg: format!("unexpected token `{}`", tok.kind),
                span: tok.span,
            }),
        }
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.skip_newlines();
        match self.peek_kind().clone() {
            TokenKind::Let    => self.parse_let(),
            TokenKind::Fn     => self.parse_fn(),
            TokenKind::Return => self.parse_return(),
            TokenKind::If     => self.parse_if(),
            TokenKind::While  => self.parse_while(),
            TokenKind::For    => self.parse_for(),
            TokenKind::Ident(_) => {
                let next_is_assign = self.peek_at(1)
                    .map_or(false, |t| matches!(t.kind, TokenKind::Equal));
                if next_is_assign {
                    self.parse_assign()
                } else {
                    let expr = self.parse_expr()?;
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

    fn parse_let(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance();
        let name = match self.peek_kind().clone() {
            TokenKind::Ident(n) => { self.advance(); n }
            _ => return Err(ParseError { msg: "expected identifier after `let`".into(), span: self.peek().span }),
        };
        let ty = if matches!(self.peek_kind(), TokenKind::Colon) {
            self.advance();
            self.parse_type_full()
        } else {
            TypeAnnotation::Inferred
        };
        self.expect(&TokenKind::Equal)?;
        let value = self.parse_expr()?;
        self.consume_newline_or_eof();
        Ok(Stmt::Let { name, ty, value, span })
    }

    fn parse_assign(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        let name = match self.peek_kind().clone() {
            TokenKind::Ident(n) => { self.advance(); n }
            _ => return Err(ParseError { msg: "expected identifier".into(), span: self.peek().span }),
        };
        self.expect(&TokenKind::Equal)?;
        let value = self.parse_expr()?;
        self.consume_newline_or_eof();
        Ok(Stmt::Assign { name, value, span })
    }

    fn parse_fn(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance();
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
            let ty = self.parse_type_full();
            params.push(Param { name: p_name, ty, span: p_span });
            if matches!(self.peek_kind(), TokenKind::Comma) { self.advance(); }
        }
        self.expect(&TokenKind::RParen)?;
        let ret_ty = if matches!(self.peek_kind(), TokenKind::Arrow) {
            self.advance(); self.parse_type_full()
        } else { TypeAnnotation::Inferred };
        self.expect(&TokenKind::Colon)?;
        self.consume_newline_or_eof();
        let body = self.parse_block()?;
        Ok(Stmt::FnDef { name, params, ret_ty, body, span })
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance();
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
        self.advance();
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::Colon)?;
        self.consume_newline_or_eof();
        let then_body = self.parse_block()?;
        let else_body = if matches!(self.peek_kind(), TokenKind::Else) {
            self.advance();
            self.expect(&TokenKind::Colon)?;
            self.consume_newline_or_eof();
            Some(self.parse_block()?)
        } else { None };
        Ok(Stmt::If { cond, then_body, else_body, span })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance();
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::Colon)?;
        self.consume_newline_or_eof();
        let body = self.parse_block()?;
        Ok(Stmt::While { cond, body, span })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance();
        let var = match self.peek_kind().clone() {
            TokenKind::Ident(n) => { self.advance(); n }
            _ => return Err(ParseError { msg: "expected variable name after `for`".into(), span: self.peek().span }),
        };
        self.expect(&TokenKind::In)?;
        let iterable = self.parse_expr()?;
        self.expect(&TokenKind::Colon)?;
        self.consume_newline_or_eof();
        let body = self.parse_block()?;
        Ok(Stmt::For { var, iterable, body, span })
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, ParseError> {
        self.skip_newlines();
        if !matches!(self.peek_kind(), TokenKind::Indent) {
            return Err(ParseError { msg: "expected indented block".into(), span: self.peek().span });
        }
        self.advance();
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek_kind(), TokenKind::Dedent | TokenKind::Eof) { break; }
            stmts.push(self.parse_stmt()?);
        }
        if matches!(self.peek_kind(), TokenKind::Dedent) { self.advance(); }
        Ok(stmts)
    }

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

fn parse_fstr_parts(s: &str) -> Vec<FStrPart> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            if !cur.is_empty() { parts.push(FStrPart::Literal(std::mem::take(&mut cur))); }
            let mut var = String::new();
            while let Some(&nc) = chars.peek() {
                if nc == '}' { chars.next(); break; }
                var.push(nc); chars.next();
            }
            let var = var.trim().to_string();
            if !var.is_empty() { parts.push(FStrPart::Interp(var)); }
        } else { cur.push(c); }
    }
    if !cur.is_empty() { parts.push(FStrPart::Literal(cur)); }
    parts
}
