use dbyte_ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Let,
    Fn,
    Return,
    If,
    Else,
    While,
    For,
    In,
    Import,
    As,
    Pub,
    True,
    False,

    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),

    Plus,
    Minus,
    Star,
    Slash,
    Bang,
    Equal,
    EqualEqual,
    BangEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Arrow,

    Colon,
    Comma,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,

    Newline,
    Indent,
    Dedent,
    Eof,
}

impl std::fmt::Display for TokenKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenKind::Let => write!(f, "let"),
            TokenKind::Fn => write!(f, "fn"),
            TokenKind::Return => write!(f, "return"),
            TokenKind::If => write!(f, "if"),
            TokenKind::Else => write!(f, "else"),
            TokenKind::While => write!(f, "while"),
            TokenKind::For => write!(f, "for"),
            TokenKind::In => write!(f, "in"),
            TokenKind::Import => write!(f, "import"),
            TokenKind::As => write!(f, "as"),
            TokenKind::Pub => write!(f, "pub"),
            TokenKind::True => write!(f, "true"),
            TokenKind::False => write!(f, "false"),
            TokenKind::Ident(s) => write!(f, "{}", s),
            TokenKind::Int(n) => write!(f, "{}", n),
            TokenKind::Float(n) => write!(f, "{}", n),
            TokenKind::Str(s) => write!(f, "\"{}\"", s),
            TokenKind::Bytes(_) => write!(f, "b\"...\""),
            TokenKind::Plus => write!(f, "+"),
            TokenKind::Minus => write!(f, "-"),
            TokenKind::Star => write!(f, "*"),
            TokenKind::Slash => write!(f, "/"),
            TokenKind::Bang => write!(f, "!"),
            TokenKind::Equal => write!(f, "="),
            TokenKind::EqualEqual => write!(f, "=="),
            TokenKind::BangEqual => write!(f, "!="),
            TokenKind::Less => write!(f, "<"),
            TokenKind::LessEqual => write!(f, "<="),
            TokenKind::Greater => write!(f, ">"),
            TokenKind::GreaterEqual => write!(f, ">="),
            TokenKind::Arrow => write!(f, "->"),
            TokenKind::Colon => write!(f, ":"),
            TokenKind::Comma => write!(f, ","),
            TokenKind::LParen => write!(f, "("),
            TokenKind::RParen => write!(f, ")"),
            TokenKind::LBracket => write!(f, "["),
            TokenKind::RBracket => write!(f, "]"),
            TokenKind::Dot => write!(f, "."),
            TokenKind::Newline => write!(f, "<newline>"),
            TokenKind::Indent => write!(f, "<indent>"),
            TokenKind::Dedent => write!(f, "<dedent>"),
            TokenKind::Eof => write!(f, "<eof>"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug)]
pub struct LexError {
    pub msg: String,
    pub span: Span,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LexError at {}: {}", self.span, self.msg)
    }
}

pub struct Lexer<'src> {
    _src: &'src str,
    chars: std::iter::Peekable<std::str::CharIndices<'src>>,
    line: usize,
    col: usize,
    indent_stack: Vec<usize>,
}

impl<'src> Lexer<'src> {
    pub fn new(src: &'src str) -> Self {
        let src = src.trim_start_matches('\u{feff}');
        Self {
            _src: src,
            chars: src.char_indices().peekable(),
            line: 1,
            col: 1,
            indent_stack: vec![0],
        }
    }

    fn span(&self) -> Span {
        Span::new(self.line, self.col)
    }

    fn advance(&mut self) -> Option<(usize, char)> {
        let r = self.chars.next();
        if let Some((_, c)) = r {
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
        }
        r
    }

    fn peek_char(&mut self) -> Option<char> {
        self.chars.peek().map(|(_, c)| *c)
    }

    fn peek2_char(&mut self) -> Option<char> {
        let mut clone = self.chars.clone();
        clone.next();
        clone.next().map(|(_, c)| c)
    }

    fn handle_indent(&mut self, indent: usize) -> Vec<Token> {
        let mut toks = Vec::new();
        let cur = *self.indent_stack.last().unwrap();
        let sp = self.span();
        if indent > cur {
            self.indent_stack.push(indent);
            toks.push(Token::new(TokenKind::Indent, sp));
        } else if indent < cur {
            while *self.indent_stack.last().unwrap() > indent {
                self.indent_stack.pop();
                toks.push(Token::new(TokenKind::Dedent, sp));
            }
        }
        toks
    }

    fn read_bytes(&mut self) -> Result<Token, LexError> {
        let sp = self.span();
        let mut bytes = Vec::new();
        loop {
            match self.advance() {
                Some((_, '"')) => break,
                Some((_, '\\')) => match self.advance() {
                    Some((_, 'n')) => bytes.push(b'\n'),
                    Some((_, 'r')) => bytes.push(b'\r'),
                    Some((_, 't')) => bytes.push(b'\t'),
                    Some((_, '\\')) => bytes.push(b'\\'),
                    Some((_, '"')) => bytes.push(b'"'),
                    Some((_, 'x')) => {
                        let h1 = self
                            .advance()
                            .ok_or_else(|| LexError {
                                msg: "invalid byte escape".into(),
                                span: sp,
                            })?
                            .1;
                        let h2 = self
                            .advance()
                            .ok_or_else(|| LexError {
                                msg: "invalid byte escape".into(),
                                span: sp,
                            })?
                            .1;
                        let hex = format!("{}{}", h1, h2);
                        let b = u8::from_str_radix(&hex, 16).map_err(|_| LexError {
                            msg: "invalid byte escape".into(),
                            span: sp,
                        })?;
                        bytes.push(b);
                    }
                    _ => {
                        return Err(LexError {
                            msg: "invalid byte escape".into(),
                            span: sp,
                        });
                    }
                },
                Some((_, c)) => {
                    if c.is_ascii() {
                        bytes.push(c as u8);
                    } else {
                        return Err(LexError {
                            msg: "non-ASCII character in byte literal".into(),
                            span: sp,
                        });
                    }
                }
                None => {
                    return Err(LexError {
                        msg: "unterminated byte literal".into(),
                        span: sp,
                    })
                }
            }
        }
        Ok(Token::new(TokenKind::Bytes(bytes), sp))
    }

    fn read_string(&mut self) -> Result<Token, LexError> {
        let sp = self.span();
        let mut s = String::new();
        loop {
            match self.advance() {
                Some((_, '"')) => break,
                Some((_, '\\')) => match self.advance() {
                    Some((_, 'n')) => s.push('\n'),
                    Some((_, 't')) => s.push('\t'),
                    Some((_, '"')) => s.push('"'),
                    Some((_, '\\')) => s.push('\\'),
                    Some((_, c)) => s.push(c),
                    None => {
                        return Err(LexError {
                            msg: "unterminated string".into(),
                            span: sp,
                        })
                    }
                },
                Some((_, c)) => s.push(c),
                None => {
                    return Err(LexError {
                        msg: "unterminated string".into(),
                        span: sp,
                    })
                }
            }
        }
        Ok(Token::new(TokenKind::Str(s), sp))
    }

    fn read_number(&mut self, first: char) -> Token {
        let sp = self.span();
        let mut s = String::from(first);
        let mut is_float = false;
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                s.push(c);
                self.advance();
            } else if c == '.'
                && !is_float
                && self.peek2_char().is_some_and(|c2| c2.is_ascii_digit())
            {
                is_float = true;
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        if is_float {
            Token::new(TokenKind::Float(s.parse().unwrap()), sp)
        } else {
            Token::new(TokenKind::Int(s.parse().unwrap()), sp)
        }
    }

    fn read_ident(&mut self, first: char) -> Token {
        let sp = self.span();
        let mut s = String::from(first);
        while let Some(c) = self.peek_char() {
            if c.is_alphanumeric() || c == '_' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        let kind = match s.as_str() {
            "let" => TokenKind::Let,
            "fn" => TokenKind::Fn,
            "return" => TokenKind::Return,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "in" => TokenKind::In,
            "import" => TokenKind::Import,
            "as" => TokenKind::As,
            "pub" => TokenKind::Pub,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            _ => TokenKind::Ident(s),
        };
        Token::new(kind, sp)
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens: Vec<Token> = Vec::new();
        let mut at_line_start = true;

        loop {
            if at_line_start {
                let mut indent = 0usize;
                while let Some(&(_, c)) = self.chars.peek() {
                    if c == ' ' {
                        indent += 1;
                        self.advance();
                    } else if c == '\t' {
                        indent += 4;
                        self.advance();
                    } else {
                        break;
                    }
                }
                match self.peek_char() {
                    None | Some('\n') | Some('#') => {}
                    _ => {
                        let mut ind_toks = self.handle_indent(indent);
                        tokens.append(&mut ind_toks);
                    }
                }
                at_line_start = false;
            }

            let c = match self.peek_char() {
                None => {
                    let sp = self.span();
                    while self.indent_stack.len() > 1 {
                        self.indent_stack.pop();
                        tokens.push(Token::new(TokenKind::Dedent, sp));
                    }
                    tokens.push(Token::new(TokenKind::Eof, sp));
                    break;
                }
                Some(c) => c,
            };

            match c {
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '#' => {
                    while self.peek_char().is_some_and(|c| c != '\n') {
                        self.advance();
                    }
                }
                '\n' => {
                    let sp = self.span();
                    self.advance();
                    tokens.push(Token::new(TokenKind::Newline, sp));
                    at_line_start = true;
                }
                '"' => {
                    self.advance();
                    tokens.push(self.read_string()?);
                }
                'b' if self.peek2_char() == Some('"') => {
                    self.advance(); // consume 'b'
                    self.advance(); // consume '"'
                    tokens.push(self.read_bytes()?);
                }
                c if c.is_ascii_digit() => {
                    self.advance();
                    tokens.push(self.read_number(c));
                }
                c if c.is_alphabetic() || c == '_' => {
                    self.advance();
                    tokens.push(self.read_ident(c));
                }
                '+' => {
                    let sp = self.span();
                    self.advance();
                    tokens.push(Token::new(TokenKind::Plus, sp));
                }
                '*' => {
                    let sp = self.span();
                    self.advance();
                    tokens.push(Token::new(TokenKind::Star, sp));
                }
                '/' => {
                    let sp = self.span();
                    self.advance();
                    tokens.push(Token::new(TokenKind::Slash, sp));
                }
                ',' => {
                    let sp = self.span();
                    self.advance();
                    tokens.push(Token::new(TokenKind::Comma, sp));
                }
                '(' => {
                    let sp = self.span();
                    self.advance();
                    tokens.push(Token::new(TokenKind::LParen, sp));
                }
                ')' => {
                    let sp = self.span();
                    self.advance();
                    tokens.push(Token::new(TokenKind::RParen, sp));
                }
                '[' => {
                    let sp = self.span();
                    self.advance();
                    tokens.push(Token::new(TokenKind::LBracket, sp));
                }
                ']' => {
                    let sp = self.span();
                    self.advance();
                    tokens.push(Token::new(TokenKind::RBracket, sp));
                }
                '.' => {
                    let sp = self.span();
                    self.advance();
                    tokens.push(Token::new(TokenKind::Dot, sp));
                }
                ':' => {
                    let sp = self.span();
                    self.advance();
                    tokens.push(Token::new(TokenKind::Colon, sp));
                }
                '-' => {
                    let sp = self.span();
                    self.advance();
                    if self.peek_char() == Some('>') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::Arrow, sp));
                    } else {
                        tokens.push(Token::new(TokenKind::Minus, sp));
                    }
                }
                '=' => {
                    let sp = self.span();
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::EqualEqual, sp));
                    } else {
                        tokens.push(Token::new(TokenKind::Equal, sp));
                    }
                }
                '!' => {
                    let sp = self.span();
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::BangEqual, sp));
                    } else {
                        tokens.push(Token::new(TokenKind::Bang, sp));
                    }
                }
                '<' => {
                    let sp = self.span();
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::LessEqual, sp));
                    } else {
                        tokens.push(Token::new(TokenKind::Less, sp));
                    }
                }
                '>' => {
                    let sp = self.span();
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::GreaterEqual, sp));
                    } else {
                        tokens.push(Token::new(TokenKind::Greater, sp));
                    }
                }
                other => {
                    return Err(LexError {
                        msg: format!("unexpected character '{}'", other),
                        span: self.span(),
                    });
                }
            }
        }
        Ok(tokens)
    }
}
