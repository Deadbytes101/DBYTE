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

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }
    fn peek_kind(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
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

    fn consume_newline_or_eof(&mut self) {
        if matches!(self.peek_kind(), TokenKind::Newline) {
            self.advance();
        }
    }

    fn parse_type_full(&mut self) -> Result<TypeAnnotation, ParseError> {
        let token = self.peek().clone();
        if let TokenKind::Ident(name) = token.kind {
            let ty = match name.as_str() {
                "int" => {
                    self.advance();
                    TypeAnnotation::Int
                }
                "i8" => {
                    self.advance();
                    TypeAnnotation::I8
                }
                "u8" => {
                    self.advance();
                    TypeAnnotation::U8
                }
                "i16" => {
                    self.advance();
                    TypeAnnotation::I16
                }
                "u16" => {
                    self.advance();
                    TypeAnnotation::U16
                }
                "i32" => {
                    self.advance();
                    TypeAnnotation::I32
                }
                "u32" => {
                    self.advance();
                    TypeAnnotation::U32
                }
                "usize" => {
                    self.advance();
                    TypeAnnotation::Usize
                }
                "float" => {
                    self.advance();
                    TypeAnnotation::Float
                }
                "bool" => {
                    self.advance();
                    TypeAnnotation::Bool
                }
                "str" => {
                    self.advance();
                    TypeAnnotation::Str
                }
                "bytes" => {
                    self.advance();
                    TypeAnnotation::Bytes
                }
                "buffer" => {
                    self.advance();
                    TypeAnnotation::Buffer
                }
                "list" => {
                    self.advance();
                    if matches!(self.peek_kind(), TokenKind::LBracket) {
                        self.advance();
                        let inner = self.parse_type_full()?;
                        self.expect(&TokenKind::RBracket)?;
                        TypeAnnotation::List(Box::new(inner))
                    } else {
                        TypeAnnotation::Inferred
                    }
                }
                "ptr" => {
                    self.advance();
                    self.expect(&TokenKind::LBracket)?;
                    let inner = self.parse_type_full()?;
                    self.expect(&TokenKind::RBracket)?;
                    TypeAnnotation::Pointer(Box::new(inner))
                }
                "array" => {
                    self.advance();
                    self.expect(&TokenKind::LBracket)?;
                    let element = self.parse_type_full()?;
                    self.expect(&TokenKind::Comma)?;
                    let length_token = self.peek().clone();
                    let length = match length_token.kind {
                        TokenKind::Int(length) => {
                            let length = u32::try_from(length).map_err(|_| ParseError {
                                msg: "array length must be an unsigned 32-bit integer".into(),
                                span: length_token.span,
                            })?;
                            self.advance();
                            length
                        }
                        _ => {
                            return Err(ParseError {
                                msg: "expected integer array length".into(),
                                span: length_token.span,
                            })
                        }
                    };
                    self.expect(&TokenKind::RBracket)?;
                    TypeAnnotation::Array {
                        element: Box::new(element),
                        length,
                    }
                }
                _ => {
                    self.advance();
                    let mut qualified = name;
                    while matches!(self.peek_kind(), TokenKind::Dot) {
                        self.advance();
                        match self.peek_kind().clone() {
                            TokenKind::Ident(part) => {
                                self.advance();
                                qualified.push('.');
                                qualified.push_str(&part);
                            }
                            _ => {
                                return Err(ParseError {
                                    msg: "expected type name after `.`".into(),
                                    span: self.peek().span,
                                });
                            }
                        }
                    }
                    TypeAnnotation::Named(qualified)
                }
            };
            Ok(ty)
        } else {
            Err(ParseError {
                msg: format!("expected type, found `{}`", token.kind),
                span: token.span,
            })
        }
    }

    pub fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_bitwise_or()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::EqualEqual => BinOp::EqEq,
                TokenKind::BangEqual => BinOp::NotEq,
                TokenKind::Less => BinOp::Lt,
                TokenKind::LessEqual => BinOp::LtEq,
                TokenKind::Greater => BinOp::Gt,
                TokenKind::GreaterEqual => BinOp::GtEq,
                _ => break,
            };
            let span = self.peek().span;
            self.advance();
            let right = self.parse_bitwise_or()?;
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_bitwise_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_bitwise_xor()?;
        while matches!(self.peek_kind(), TokenKind::Pipe) {
            let span = self.peek().span;
            self.advance();
            let right = self.parse_bitwise_xor()?;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::BitOr,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_bitwise_xor(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_bitwise_and()?;
        while matches!(self.peek_kind(), TokenKind::Caret) {
            let span = self.peek().span;
            self.advance();
            let right = self.parse_bitwise_and()?;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::BitXor,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_bitwise_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_shift()?;
        while matches!(self.peek_kind(), TokenKind::Ampersand) {
            let span = self.peek().span;
            self.advance();
            let right = self.parse_shift()?;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::BitAnd,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_shift(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::ShiftLeft => BinOp::Shl,
                TokenKind::ShiftRight => BinOp::Shr,
                _ => break,
            };
            let span = self.peek().span;
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            let span = self.peek().span;
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_cast()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Rem,
                _ => break,
            };
            let span = self.peek().span;
            self.advance();
            let right = self.parse_cast()?;
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_cast(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_unary()?;
        while matches!(self.peek_kind(), TokenKind::As) {
            let span = self.peek().span;
            self.advance();
            let ty = self.parse_type_full()?;
            expr = Expr::Cast {
                expr: Box::new(expr),
                ty,
                span,
            };
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek_kind() {
            TokenKind::Minus => {
                let span = self.peek().span;
                self.advance();
                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(self.parse_unary()?),
                    span,
                })
            }
            TokenKind::Bang => {
                let span = self.peek().span;
                self.advance();
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(self.parse_unary()?),
                    span,
                })
            }
            TokenKind::Tilde => {
                let span = self.peek().span;
                self.advance();
                Ok(Expr::Unary {
                    op: UnaryOp::BitNot,
                    expr: Box::new(self.parse_unary()?),
                    span,
                })
            }
            TokenKind::Ampersand => {
                let span = self.peek().span;
                self.advance();
                Ok(Expr::Unary {
                    op: UnaryOp::AddressOf,
                    expr: Box::new(self.parse_unary()?),
                    span,
                })
            }
            TokenKind::Star => {
                let span = self.peek().span;
                self.advance();
                Ok(Expr::Unary {
                    op: UnaryOp::Deref,
                    expr: Box::new(self.parse_unary()?),
                    span,
                })
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
                            if matches!(self.peek_kind(), TokenKind::Comma) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        self.expect(&TokenKind::RParen)?;
                        base = Expr::Call { name, args, span };
                    } else if let Expr::Member {
                        object,
                        property,
                        span,
                    } = base
                    {
                        self.advance();
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
                        base = Expr::MemberCall {
                            object,
                            property,
                            args,
                            span,
                        };
                    } else {
                        break;
                    }
                }
                TokenKind::LBracket => {
                    let span = self.peek().span;
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket)?;
                    base = Expr::Index {
                        target: Box::new(base),
                        index: Box::new(index),
                        span,
                    };
                }
                TokenKind::Dot => {
                    let span = self.peek().span;
                    self.advance();
                    let property = match self.peek_kind().clone() {
                        TokenKind::Ident(n) => {
                            self.advance();
                            n
                        }
                        _ => {
                            return Err(ParseError {
                                msg: "expected identifier after `.`".into(),
                                span: self.peek().span,
                            })
                        }
                    };
                    base = Expr::Member {
                        object: Box::new(base),
                        property,
                        span,
                    };
                }
                _ => break,
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
                let parts = parse_fstr_parts(&s);
                let all_literal = parts.len() == 1 && matches!(&parts[0], FStrPart::Literal(_));
                if all_literal || parts.is_empty() {
                    Ok(Expr::StrLit(s, tok.span))
                } else {
                    Ok(Expr::FStr(parts, tok.span))
                }
            }
            TokenKind::Bytes(ref b) => {
                let b = b.clone();
                self.advance();
                Ok(Expr::BytesLit(b, tok.span))
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
                    if matches!(self.peek_kind(), TokenKind::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
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
            TokenKind::Let => self.parse_let(false),
            TokenKind::Fn => self.parse_fn(false, None),
            TokenKind::Interrupt => self.parse_interrupt_fn(false),
            TokenKind::Const => self.parse_const(false),
            TokenKind::Static => self.parse_static(false),
            TokenKind::Struct => self.parse_struct(false, false),
            TokenKind::Packed => self.parse_packed_struct(false),
            TokenKind::Pub => {
                self.advance();
                match self.peek_kind().clone() {
                    TokenKind::Let => self.parse_let(true),
                    TokenKind::Fn => self.parse_fn(true, None),
                    TokenKind::Interrupt => self.parse_interrupt_fn(true),
                    TokenKind::Const => self.parse_const(true),
                    TokenKind::Static => self.parse_static(true),
                    TokenKind::Struct => self.parse_struct(true, false),
                    TokenKind::Packed => self.parse_packed_struct(true),
                    _ => Err(ParseError {
                        msg: "expected `let`, `fn`, `interrupt`, `const`, `static`, `struct`, or `packed struct` after `pub`".into(),
                        span: self.peek().span,
                    }),
                }
            }
            TokenKind::Import => self.parse_import(),
            TokenKind::Return => self.parse_return(),
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for(),
            TokenKind::Ident(_) => {
                let next_is_assign = self
                    .peek_at(1)
                    .is_some_and(|t| matches!(t.kind, TokenKind::Equal));
                if next_is_assign {
                    self.parse_assign()
                } else {
                    self.parse_expr_or_store()
                }
            }
            _ => self.parse_expr_or_store(),
        }
    }

    fn parse_expr_or_store(&mut self) -> Result<Stmt, ParseError> {
        let target = self.parse_expr()?;
        if matches!(self.peek_kind(), TokenKind::Equal) {
            let span = target.span();
            if !matches!(
                &target,
                Expr::Index { .. }
                    | Expr::Member { .. }
                    | Expr::Unary {
                        op: UnaryOp::Deref,
                        ..
                    }
            ) {
                return Err(ParseError {
                    msg: "assignment target must be a name, dereference, index, or field".into(),
                    span,
                });
            }
            self.advance();
            let value = self.parse_expr()?;
            self.consume_newline_or_eof();
            Ok(Stmt::Store {
                target,
                value,
                span,
            })
        } else {
            self.consume_newline_or_eof();
            Ok(Stmt::Expr(target))
        }
    }

    fn parse_import(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance();

        let path = match self.peek_kind().clone() {
            TokenKind::Str(s) => {
                self.advance();
                s
            }
            TokenKind::Ident(s) => {
                self.advance();
                let mut p = s;
                while matches!(self.peek_kind(), TokenKind::Dot) {
                    self.advance();
                    match self.peek_kind().clone() {
                        TokenKind::Ident(n) => {
                            self.advance();
                            p.push('.');
                            p.push_str(&n);
                        }
                        _ => {
                            return Err(ParseError {
                                msg: "expected identifier after `.`".into(),
                                span: self.peek().span,
                            })
                        }
                    }
                }
                p
            }
            _ => {
                return Err(ParseError {
                    msg: "expected string or identifier after `import`".into(),
                    span: self.peek().span,
                })
            }
        };

        self.expect(&TokenKind::As)?;
        let alias = match self.peek_kind().clone() {
            TokenKind::Ident(n) => {
                self.advance();
                n
            }
            _ => {
                return Err(ParseError {
                    msg: "expected alias identifier after `as`".into(),
                    span: self.peek().span,
                })
            }
        };

        self.consume_newline_or_eof();
        Ok(Stmt::Import { path, alias, span })
    }

    fn parse_let(&mut self, is_pub: bool) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance();
        let name = match self.peek_kind().clone() {
            TokenKind::Ident(n) => {
                self.advance();
                n
            }
            _ => {
                return Err(ParseError {
                    msg: "expected identifier after `let`".into(),
                    span: self.peek().span,
                })
            }
        };
        let ty = if matches!(self.peek_kind(), TokenKind::Colon) {
            self.advance();
            self.parse_type_full()?
        } else {
            TypeAnnotation::Inferred
        };
        self.expect(&TokenKind::Equal)?;
        let value = self.parse_expr()?;
        self.consume_newline_or_eof();
        Ok(Stmt::Let {
            is_pub,
            name,
            ty,
            value,
            span,
        })
    }

    fn parse_const(&mut self, is_pub: bool) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance();
        let name = self.parse_declaration_name("const")?;
        self.expect(&TokenKind::Colon)?;
        let ty = self.parse_type_full()?;
        self.expect(&TokenKind::Equal)?;
        let value = self.parse_expr()?;
        self.consume_newline_or_eof();
        Ok(Stmt::Const {
            is_pub,
            name,
            ty,
            value,
            span,
        })
    }

    fn parse_static(&mut self, is_pub: bool) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance();
        let is_mut = if matches!(self.peek_kind(), TokenKind::Mut) {
            self.advance();
            true
        } else {
            false
        };
        let name = self.parse_declaration_name(if is_mut { "static mut" } else { "static" })?;
        self.expect(&TokenKind::Colon)?;
        let ty = self.parse_type_full()?;
        let value = if matches!(self.peek_kind(), TokenKind::Equal) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.consume_newline_or_eof();
        Ok(Stmt::Static {
            is_pub,
            is_mut,
            name,
            ty,
            value,
            span,
        })
    }

    fn parse_declaration_name(&mut self, declaration: &str) -> Result<String, ParseError> {
        match self.peek_kind().clone() {
            TokenKind::Ident(name) => {
                self.advance();
                Ok(name)
            }
            _ => Err(ParseError {
                msg: format!("expected identifier after `{declaration}`"),
                span: self.peek().span,
            }),
        }
    }

    fn parse_packed_struct(&mut self, is_pub: bool) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance();
        self.expect(&TokenKind::Struct)?;
        self.parse_struct_after_keyword(is_pub, true, span)
    }

    fn parse_struct(&mut self, is_pub: bool, is_packed: bool) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance();
        self.parse_struct_after_keyword(is_pub, is_packed, span)
    }

    fn parse_struct_after_keyword(
        &mut self,
        is_pub: bool,
        is_packed: bool,
        span: Span,
    ) -> Result<Stmt, ParseError> {
        let name = self.parse_declaration_name("struct")?;
        self.expect(&TokenKind::Colon)?;
        self.consume_newline_or_eof();
        self.skip_newlines();
        self.expect(&TokenKind::Indent)?;

        let mut fields = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek_kind(), TokenKind::Dedent | TokenKind::Eof) {
                break;
            }
            let field_span = self.peek().span;
            let field_name = match self.peek_kind().clone() {
                TokenKind::Ident(name) => {
                    self.advance();
                    name
                }
                _ => {
                    return Err(ParseError {
                        msg: "expected struct field name".into(),
                        span: field_span,
                    })
                }
            };
            self.expect(&TokenKind::Colon)?;
            let ty = self.parse_type_full()?;
            self.consume_newline_or_eof();
            fields.push(StructField {
                name: field_name,
                ty,
                span: field_span,
            });
        }
        if matches!(self.peek_kind(), TokenKind::Dedent) {
            self.advance();
        }
        Ok(Stmt::StructDef {
            is_pub,
            is_packed,
            name,
            fields,
            span,
        })
    }

    fn parse_assign(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        let name = match self.peek_kind().clone() {
            TokenKind::Ident(n) => {
                self.advance();
                n
            }
            _ => {
                return Err(ParseError {
                    msg: "expected identifier".into(),
                    span: self.peek().span,
                })
            }
        };
        self.expect(&TokenKind::Equal)?;
        let value = self.parse_expr()?;
        self.consume_newline_or_eof();
        Ok(Stmt::Assign { name, value, span })
    }

    fn parse_interrupt_fn(&mut self, is_pub: bool) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.expect(&TokenKind::Interrupt)?;
        self.expect(&TokenKind::LParen)?;
        let class = match self.peek_kind().clone() {
            TokenKind::Ident(name) if name == "no_error_code" => InterruptClass::NoErrorCode,
            TokenKind::Ident(name) if name == "error_code" => InterruptClass::ErrorCode,
            _ => {
                return Err(ParseError {
                    msg: "expected interrupt class `no_error_code` or `error_code`".into(),
                    span: self.peek().span,
                })
            }
        };
        self.advance();
        self.expect(&TokenKind::RParen)?;
        if !matches!(self.peek_kind(), TokenKind::Fn) {
            return Err(ParseError {
                msg: "expected `fn` after interrupt class".into(),
                span,
            });
        }
        self.parse_fn(is_pub, Some(class))
    }

    fn parse_fn(
        &mut self,
        is_pub: bool,
        interrupt: Option<InterruptClass>,
    ) -> Result<Stmt, ParseError> {
        let span = self.peek().span;
        self.advance();
        let name = match self.peek_kind().clone() {
            TokenKind::Ident(n) => {
                self.advance();
                n
            }
            _ => {
                return Err(ParseError {
                    msg: "expected function name".into(),
                    span: self.peek().span,
                })
            }
        };
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::RParen | TokenKind::Eof) {
            let p_span = self.peek().span;
            let p_name = match self.peek_kind().clone() {
                TokenKind::Ident(n) => {
                    self.advance();
                    n
                }
                _ => {
                    return Err(ParseError {
                        msg: "expected parameter name".into(),
                        span: self.peek().span,
                    })
                }
            };
            self.expect(&TokenKind::Colon)?;
            let ty = self.parse_type_full()?;
            params.push(Param {
                name: p_name,
                ty,
                span: p_span,
            });
            if matches!(self.peek_kind(), TokenKind::Comma) {
                self.advance();
            }
        }
        self.expect(&TokenKind::RParen)?;
        let ret_ty = if matches!(self.peek_kind(), TokenKind::Arrow) {
            self.advance();
            self.parse_type_full()?
        } else {
            TypeAnnotation::Inferred
        };
        self.expect(&TokenKind::Colon)?;
        self.consume_newline_or_eof();
        let body = self.parse_block()?;
        Ok(Stmt::FnDef {
            is_pub,
            interrupt,
            name,
            params,
            ret_ty,
            body,
            span,
        })
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
        Ok(Stmt::Return {
            value: Some(value),
            span,
        })
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
        } else {
            None
        };
        Ok(Stmt::If {
            cond,
            then_body,
            else_body,
            span,
        })
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
            TokenKind::Ident(n) => {
                self.advance();
                n
            }
            _ => {
                return Err(ParseError {
                    msg: "expected variable name after `for`".into(),
                    span: self.peek().span,
                })
            }
        };
        self.expect(&TokenKind::In)?;
        let iterable = self.parse_expr()?;
        self.expect(&TokenKind::Colon)?;
        self.consume_newline_or_eof();
        let body = self.parse_block()?;
        Ok(Stmt::For {
            var,
            iterable,
            body,
            span,
        })
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, ParseError> {
        self.skip_newlines();
        if !matches!(self.peek_kind(), TokenKind::Indent) {
            return Err(ParseError {
                msg: "expected indented block".into(),
                span: self.peek().span,
            });
        }
        self.advance();
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek_kind(), TokenKind::Dedent | TokenKind::Eof) {
                break;
            }
            stmts.push(self.parse_stmt()?);
        }
        if matches!(self.peek_kind(), TokenKind::Dedent) {
            self.advance();
        }
        Ok(stmts)
    }

    pub fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_eof() {
                break;
            }
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
            if !cur.is_empty() {
                parts.push(FStrPart::Literal(std::mem::take(&mut cur)));
            }
            let mut var = String::new();
            while let Some(&nc) = chars.peek() {
                if nc == '}' {
                    chars.next();
                    break;
                }
                var.push(nc);
                chars.next();
            }
            let var = var.trim().to_string();
            if !var.is_empty() {
                parts.push(FStrPart::Interp(var));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        parts.push(FStrPart::Literal(cur));
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_lexer::Lexer;

    fn parse(source: &str) -> Program {
        let tokens = Lexer::new(source).tokenize().expect("source should lex");
        Parser::new(tokens)
            .parse_program()
            .expect("source should parse")
    }

    #[test]
    fn parses_native_systems_declarations_and_types() {
        let program = parse(
            "pub struct Header:\n    tag: u16\n    next: ptr[Header]\n    owner: ptr[types.Owner]\npacked struct Wire:\n    bytes: array[u8, 4]\npub const PORT: usize = 1016\npub static MESSAGE: array[u8, 4] = [68, 66, 89, 0]\nstatic mut BUFFER: array[u8, 64]\n",
        );

        assert!(matches!(
            &program.stmts[0],
            Stmt::StructDef {
                is_pub: true,
                is_packed: false,
                name,
                fields,
                ..
            } if name == "Header"
                && fields[0].ty == TypeAnnotation::U16
                && fields[1].ty == TypeAnnotation::Pointer(Box::new(TypeAnnotation::Named("Header".into())))
                && fields[2].ty == TypeAnnotation::Pointer(Box::new(TypeAnnotation::Named("types.Owner".into())))
        ));
        assert!(matches!(
            &program.stmts[1],
            Stmt::StructDef {
                is_packed: true,
                fields,
                ..
            } if fields[0].ty == TypeAnnotation::Array {
                element: Box::new(TypeAnnotation::U8),
                length: 4,
            }
        ));
        assert!(matches!(
            &program.stmts[2],
            Stmt::Const { is_pub: true, .. }
        ));
        assert!(matches!(
            &program.stmts[3],
            Stmt::Static {
                is_pub: true,
                is_mut: false,
                value: Some(_),
                ..
            }
        ));
        assert!(matches!(
            &program.stmts[4],
            Stmt::Static {
                is_pub: false,
                is_mut: true,
                value: None,
                ..
            }
        ));
    }

    #[test]
    fn parses_every_native_scalar_annotation() {
        let program = parse(
            "fn widths(a: i8, b: u8, c: i16, d: u16, e: i32, f: u32, g: usize, h: bool) -> i32:\n    return 0\n",
        );
        let Stmt::FnDef { params, ret_ty, .. } = &program.stmts[0] else {
            panic!("expected function");
        };
        let types: Vec<_> = params.iter().map(|param| param.ty.clone()).collect();
        assert_eq!(
            types,
            vec![
                TypeAnnotation::I8,
                TypeAnnotation::U8,
                TypeAnnotation::I16,
                TypeAnnotation::U16,
                TypeAnnotation::I32,
                TypeAnnotation::U32,
                TypeAnnotation::Usize,
                TypeAnnotation::Bool,
            ]
        );
        assert_eq!(*ret_ty, TypeAnnotation::I32);
    }

    #[test]
    fn parses_native_operators_casts_and_general_store_targets() {
        let program = parse(
            "fn kernel_main() -> i32:\n    let p: ptr[u32] = 4096 as ptr[u32]\n    let pp: ptr[ptr[u32]] = &p\n    *p = (~1 & 255) | (2 << 3) ^ (8 >> 1)\n    p[1] = 7\n    p.value = 9\n    return *p as i32\n",
        );
        let Stmt::FnDef { body, .. } = &program.stmts[0] else {
            panic!("expected function");
        };
        assert!(matches!(
            &body[0],
            Stmt::Let {
                ty: TypeAnnotation::Pointer(inner),
                value: Expr::Cast { ty: cast_ty, .. },
                ..
            } if **inner == TypeAnnotation::U32
                && *cast_ty == TypeAnnotation::Pointer(Box::new(TypeAnnotation::U32))
        ));
        assert!(matches!(
            &body[1],
            Stmt::Let {
                value: Expr::Unary {
                    op: UnaryOp::AddressOf,
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            &body[2],
            Stmt::Store {
                target: Expr::Unary {
                    op: UnaryOp::Deref,
                    ..
                },
                value: Expr::Binary {
                    op: BinOp::BitOr,
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            &body[3],
            Stmt::Store {
                target: Expr::Index { .. },
                ..
            }
        ));
        assert!(matches!(
            &body[4],
            Stmt::Store {
                target: Expr::Member { .. },
                ..
            }
        ));
        assert!(matches!(
            &body[5],
            Stmt::Return {
                value: Some(Expr::Cast {
                    expr,
                    ty: TypeAnnotation::I32,
                    ..
                }),
                ..
            } if matches!(expr.as_ref(), Expr::Unary { op: UnaryOp::Deref, .. })
        ));
    }

    #[test]
    fn preserves_existing_import_visibility_and_list_syntax() {
        let program = parse(
            "import \"module.dby\" as module\npub let values: list[int] = [1, 2]\npub fn first(items: list[int]) -> int:\n    return items[0]\n",
        );
        assert!(
            matches!(&program.stmts[0], Stmt::Import { path, alias, .. } if path == "module.dby" && alias == "module")
        );
        assert!(matches!(
            &program.stmts[1],
            Stmt::Let {
                is_pub: true,
                ty: TypeAnnotation::List(inner),
                ..
            } if **inner == TypeAnnotation::Int
        ));
        assert!(matches!(
            &program.stmts[2],
            Stmt::FnDef { is_pub: true, .. }
        ));
    }

    #[test]
    fn rejects_non_integer_array_lengths() {
        let tokens = Lexer::new("let x: array[u8, nope] = []\n")
            .tokenize()
            .unwrap();
        let error = Parser::new(tokens).parse_program().unwrap_err();
        assert_eq!(error.msg, "expected integer array length");
    }

    #[test]
    fn parses_explicit_interrupt_abi_classes() {
        let program = parse(
            "pub interrupt(no_error_code) fn irq0(frame: ptr[InterruptFrame]):\n    return\ninterrupt(error_code) fn fault(frame: ptr[InterruptFrame], error_code: u32):\n    return\n",
        );
        assert!(matches!(
            &program.stmts[0],
            Stmt::FnDef {
                is_pub: true,
                interrupt: Some(InterruptClass::NoErrorCode),
                ret_ty: TypeAnnotation::Inferred,
                ..
            }
        ));
        assert!(matches!(
            &program.stmts[1],
            Stmt::FnDef {
                interrupt: Some(InterruptClass::ErrorCode),
                ..
            }
        ));
    }
}
