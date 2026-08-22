use dbyte_ast::Span;
use std::fmt;

/// Stable diagnostic classes used by the native compiler and its CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticCategory {
    Lex,
    Parse,
    UnknownSymbol,
    DuplicateLocal,
    DuplicateFunction,
    WrongArity,
    InvalidReturn,
    InvalidType,
    UnsupportedFeature,
    InvalidEntryPoint,
    InvalidIntrinsic,
    InterruptClassMismatch,
    UnresolvedFunction,
    IntegerOutOfRange,
    InvalidNarrowing,
    InvalidPointerOperation,
    BadDereference,
    InvalidField,
    DuplicateField,
    BadArrayLength,
    InvalidModuleImport,
    DuplicateModule,
    DuplicateModuleSymbol,
    UnknownType,
    DuplicateType,
    Encoder,
    Elf,
    Object,
    Link,
    DuplicateSymbol,
    UndefinedSymbol,
    InvalidObject,
    UnsupportedMachine,
    InvalidRelocation,
    RelocationOutOfRange,
    LayoutOverflow,
}

impl fmt::Display for DiagnosticCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Lex => "Lex",
            Self::Parse => "Parse",
            Self::UnknownSymbol => "UnknownSymbol",
            Self::DuplicateLocal => "DuplicateLocal",
            Self::DuplicateFunction => "DuplicateFunction",
            Self::WrongArity => "WrongArity",
            Self::InvalidReturn => "InvalidReturn",
            Self::InvalidType => "InvalidType",
            Self::UnsupportedFeature => "UnsupportedFeature",
            Self::InvalidEntryPoint => "InvalidEntryPoint",
            Self::InvalidIntrinsic => "InvalidIntrinsic",
            Self::InterruptClassMismatch => "InterruptClassMismatch",
            Self::UnresolvedFunction => "UnresolvedFunction",
            Self::IntegerOutOfRange => "IntegerOutOfRange",
            Self::InvalidNarrowing => "InvalidNarrowing",
            Self::InvalidPointerOperation => "InvalidPointerOperation",
            Self::BadDereference => "BadDereference",
            Self::InvalidField => "InvalidField",
            Self::DuplicateField => "DuplicateField",
            Self::BadArrayLength => "BadArrayLength",
            Self::InvalidModuleImport => "InvalidModuleImport",
            Self::DuplicateModule => "DuplicateModule",
            Self::DuplicateModuleSymbol => "DuplicateModuleSymbol",
            Self::UnknownType => "UnknownType",
            Self::DuplicateType => "DuplicateType",
            Self::Encoder => "Encoder",
            Self::Elf => "Elf",
            Self::Object => "Object",
            Self::Link => "Link",
            Self::DuplicateSymbol => "DuplicateSymbol",
            Self::UndefinedSymbol => "UndefinedSymbol",
            Self::InvalidObject => "InvalidObject",
            Self::UnsupportedMachine => "UnsupportedMachine",
            Self::InvalidRelocation => "InvalidRelocation",
            Self::RelocationOutOfRange => "RelocationOutOfRange",
            Self::LayoutOverflow => "LayoutOverflow",
        };
        f.write_str(name)
    }
}

/// A deterministic user-facing native compiler diagnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub category: DiagnosticCategory,
    pub message: String,
    pub path: Option<String>,
    pub span: Option<Span>,
}

impl Diagnostic {
    pub fn new(
        category: DiagnosticCategory,
        message: impl Into<String>,
        span: impl Into<Option<Span>>,
    ) -> Self {
        Self {
            category,
            message: message.into(),
            path: None,
            span: span.into(),
        }
    }

    pub fn without_span(category: DiagnosticCategory, message: impl Into<String>) -> Self {
        Self::new(category, message, None)
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn render_with_path(&self, path: impl Into<String>) -> String {
        self.clone().with_path(path).to_string()
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.category, self.message)?;
        match (&self.path, self.span) {
            (Some(path), Some(span)) => write!(f, "\n --> {}:{}:{}", path, span.line, span.col),
            (Some(path), None) => write!(f, "\n --> {}", path),
            (None, Some(span)) => write!(f, "\n --> {}:{}", span.line, span.col),
            (None, None) => Ok(()),
        }
    }
}

impl std::error::Error for Diagnostic {}

pub type NativeResult<T> = Result<T, Diagnostic>;
