use dbyte_ast::{Program, Span};
use dbyte_interp::{Interpreter, RuntimeError};
use dbyte_lexer::{LexError, Lexer};
use dbyte_parser::{ParseError, Parser};
use dbyte_typeck::{TypeChecker, TypeError};
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug)]
pub enum DByteError {
    Io {
        path: PathBuf,
        msg: String,
    },
    Lex {
        label: String,
        msg: String,
        span: Span,
    },
    Parse {
        label: String,
        msg: String,
        span: Span,
    },
    Type {
        label: String,
        msg: String,
        span: Span,
    },
    Runtime {
        label: String,
        msg: String,
        span: Span,
    },
    Rc {
        path: PathBuf,
        source: Box<DByteError>,
    },
}

impl fmt::Display for DByteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DByteError::Io { path, msg } => write!(f, "IoError: {}: {}", path.display(), msg),
            DByteError::Lex { label, msg, span } => {
                write!(f, "LexError: {} at {}: {}", label, span, msg)
            }
            DByteError::Parse { label, msg, span } => {
                write!(f, "ParseError: {} at {}: {}", label, span, msg)
            }
            DByteError::Type { label, msg, span } => {
                write!(f, "TypeError: {} at {}: {}", label, span, msg)
            }
            DByteError::Runtime { label, msg, span } => {
                write!(f, "RuntimeError: {} at {}: {}", label, span, msg)
            }
            DByteError::Rc { path, source } => {
                write!(f, "RcError: failed to load {}: {}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for DByteError {}

pub struct DByteRuntime {
    checker: TypeChecker,
    interp: Interpreter,
    current_dir: PathBuf,
    loaded_rc_paths: HashSet<PathBuf>,
}

impl Default for DByteRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl DByteRuntime {
    pub fn new() -> Self {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::from_current_dir_unchecked(current_dir)
    }

    pub fn with_current_dir(path: impl Into<PathBuf>) -> Result<Self, DByteError> {
        let path = path.into();
        let current_dir = normalize_existing_dir(&path)?;
        Ok(Self::from_current_dir_unchecked(current_dir))
    }

    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    pub fn set_current_dir(&mut self, path: impl Into<PathBuf>) -> Result<(), DByteError> {
        let path = path.into();
        let path = if path.is_absolute() {
            path
        } else {
            self.current_dir.join(path)
        };
        let current_dir = normalize_existing_dir(&path)?;
        self.current_dir = current_dir;
        self.refresh_entry_path();
        Ok(())
    }

    pub fn check_source(&self, label: &str, source: &str) -> Result<(), DByteError> {
        let program = parse_source(label, source)?;
        let mut checker = self.checker.clone();
        checker
            .check_program(&program)
            .map_err(|e| type_error(label, e))
    }

    pub fn check_file(&self, path: impl AsRef<Path>) -> Result<(), DByteError> {
        let path = if path.as_ref().is_absolute() {
            path.as_ref().to_path_buf()
        } else {
            self.current_dir.join(path.as_ref())
        };
        let source = fs::read_to_string(&path).map_err(|e| DByteError::Io {
            path: path.clone(),
            msg: e.to_string(),
        })?;
        let label = path.display().to_string();
        let program = parse_source(&label, &source)?;
        let mut checker = self.checker.clone();
        checker.set_entry_path(path);
        checker
            .check_program(&program)
            .map_err(|e| type_error(&label, e))
    }

    pub fn run_source(&mut self, label: &str, source: &str) -> Result<(), DByteError> {
        self.run_source_inner(label, source, false).map(|_| ())
    }

    pub fn run_source_capture(
        &mut self,
        label: &str,
        source: &str,
    ) -> Result<RunOutput, DByteError> {
        self.run_source_inner(label, source, true)
    }

    pub fn run_file(&mut self, path: impl AsRef<Path>) -> Result<(), DByteError> {
        self.run_file_inner(path.as_ref(), false).map(|_| ())
    }

    pub fn run_file_capture(&mut self, path: impl AsRef<Path>) -> Result<RunOutput, DByteError> {
        self.run_file_inner(path.as_ref(), true)
    }

    pub fn load_rc(&mut self) -> Result<(), DByteError> {
        let rc_path = self.current_dir.join(".dbyterc");
        if !rc_path.exists() {
            return Ok(());
        }
        let rc_key = fs::canonicalize(&rc_path).unwrap_or_else(|_| rc_path.clone());
        if self.loaded_rc_paths.contains(&rc_key) {
            return Ok(());
        }

        self.run_file(&rc_path).map_err(|source| DByteError::Rc {
            path: rc_path,
            source: Box::new(source),
        })?;
        self.loaded_rc_paths.insert(rc_key);
        Ok(())
    }

    fn from_current_dir_unchecked(current_dir: PathBuf) -> Self {
        let entry_path = current_dir.join("<embed>");
        Self {
            checker: TypeChecker::with_entry_path(entry_path.clone()),
            interp: Interpreter::with_captured_output(entry_path),
            current_dir,
            loaded_rc_paths: HashSet::new(),
        }
    }

    fn refresh_entry_path(&mut self) {
        let entry_path = self.current_dir.join("<embed>");
        self.checker.set_entry_path(entry_path.clone());
        self.interp.set_entry_path(entry_path);
    }

    fn run_source_inner(
        &mut self,
        label: &str,
        source: &str,
        capture: bool,
    ) -> Result<RunOutput, DByteError> {
        self.interp.clear_captured_output();
        let program = parse_source(label, source)?;
        self.run_program(label, &program)?;
        let stdout = self.interp.take_captured_output();
        if capture {
            Ok(RunOutput {
                stdout,
                stderr: String::new(),
            })
        } else {
            Ok(RunOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    fn run_file_inner(&mut self, path: &Path, capture: bool) -> Result<RunOutput, DByteError> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.current_dir.join(path)
        };
        let source = fs::read_to_string(&path).map_err(|e| DByteError::Io {
            path: path.clone(),
            msg: e.to_string(),
        })?;
        let label = path.display().to_string();
        let previous_entry = self.current_dir.join("<embed>");
        self.checker.set_entry_path(path.clone());
        self.interp.set_entry_path(path.clone());
        let result = self.run_source_inner(&label, &source, capture);
        self.checker.set_entry_path(previous_entry.clone());
        self.interp.set_entry_path(previous_entry);
        result
    }

    fn run_program(&mut self, label: &str, program: &Program) -> Result<(), DByteError> {
        let checker_snapshot = self.checker.clone();
        let interp_snapshot = self.interp.clone();

        if let Err(error) = self.checker.check_program(program) {
            self.checker = checker_snapshot;
            return Err(type_error(label, error));
        }

        if let Err(error) = self.interp.run(program) {
            self.checker = checker_snapshot;
            self.interp = interp_snapshot;
            return Err(runtime_error(label, error));
        }

        Ok(())
    }
}

fn normalize_existing_dir(path: &Path) -> Result<PathBuf, DByteError> {
    let normalized = fs::canonicalize(path).map_err(|e| DByteError::Io {
        path: path.to_path_buf(),
        msg: e.to_string(),
    })?;
    if normalized.is_dir() {
        Ok(normalized)
    } else {
        Err(DByteError::Io {
            path: normalized,
            msg: "not a directory".into(),
        })
    }
}

fn parse_source(label: &str, source: &str) -> Result<Program, DByteError> {
    let tokens = Lexer::new(source)
        .tokenize()
        .map_err(|e| lex_error(label, e))?;
    Parser::new(tokens)
        .parse_program()
        .map_err(|e| parse_error(label, e))
}

fn lex_error(label: &str, error: LexError) -> DByteError {
    DByteError::Lex {
        label: label.to_string(),
        msg: error.msg,
        span: error.span,
    }
}

fn parse_error(label: &str, error: ParseError) -> DByteError {
    DByteError::Parse {
        label: label.to_string(),
        msg: error.msg,
        span: error.span,
    }
}

fn type_error(label: &str, error: TypeError) -> DByteError {
    DByteError::Type {
        label: label.to_string(),
        msg: error.msg,
        span: error.span,
    }
}

fn runtime_error(label: &str, error: RuntimeError) -> DByteError {
    DByteError::Runtime {
        label: label.to_string(),
        msg: error.msg,
        span: error.span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn preserves_source_state_and_captures_stdout() {
        let mut rt = DByteRuntime::new();
        rt.run_source("test", "let x: int = 40").unwrap();
        let out = rt.run_source_capture("test", "print(x + 2)").unwrap();
        assert_eq!(out.stdout.trim(), "42");
        assert_eq!(out.stderr, "");
    }

    #[test]
    fn capture_output_isolated_between_runs() {
        let mut rt = DByteRuntime::new();
        let first = rt.run_source_capture("test", "print(1)").unwrap();
        let second = rt.run_source_capture("test", "print(2)").unwrap();
        assert_eq!(first.stdout, "1\n");
        assert_eq!(second.stdout, "2\n");
        assert_eq!(first.stderr, "");
        assert_eq!(second.stderr, "");
    }

    #[test]
    fn failed_run_does_not_leak_output_to_next_capture() {
        let mut rt = DByteRuntime::new();
        let error = rt
            .run_source_capture("test", "print(1)\nlet bad: int = 1 / 0")
            .unwrap_err();
        assert!(matches!(error, DByteError::Runtime { .. }));
        let out = rt.run_source_capture("test", "print(2)").unwrap();
        assert_eq!(out.stdout, "2\n");
    }

    #[test]
    fn preserves_functions_and_imports() {
        let mut rt = DByteRuntime::new();
        rt.run_source(
            "test",
            "import std.math as math\nfn add(a: int, b: int) -> int:\n    return a + b",
        )
        .unwrap();
        let out = rt
            .run_source_capture("test", "print(math.max(add(20, 1), add(1, 2)))")
            .unwrap();
        assert_eq!(out.stdout.trim(), "21");
    }

    #[test]
    fn local_relative_import_uses_current_dir() {
        let root = temp_dir("relative-import");
        fs::write(
            root.join("helper.dby"),
            "pub fn inc(x: int) -> int:\n    return x + 1\n",
        )
        .unwrap();
        let mut rt = DByteRuntime::with_current_dir(&root).unwrap();
        rt.run_source("test", "import \"./helper.dby\" as helper")
            .unwrap();
        let out = rt
            .run_source_capture("test", "print(helper.inc(41))")
            .unwrap();
        assert_eq!(out.stdout.trim(), "42");
        cleanup(root);
    }

    #[test]
    fn set_current_dir_changes_relative_import_base() {
        let root = temp_dir("set-current-dir");
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("helper.dby"), "pub let value: int = 1\n").unwrap();
        fs::write(second.join("helper.dby"), "pub let value: int = 2\n").unwrap();

        let mut rt = DByteRuntime::with_current_dir(&first).unwrap();
        rt.run_source("test", "import \"./helper.dby\" as one")
            .unwrap();
        rt.set_current_dir(&second).unwrap();
        rt.run_source("test", "import \"./helper.dby\" as two")
            .unwrap();
        let out = rt
            .run_source_capture("test", "print(one.value + two.value)")
            .unwrap();
        assert_eq!(out.stdout.trim(), "3");
        cleanup(root);
    }

    #[test]
    fn check_source_does_not_mutate_state() {
        let rt = DByteRuntime::new();
        rt.check_source("test", "let x: int = 1").unwrap();
        let error = rt.check_source("test", "print(x)").unwrap_err();
        assert!(error.to_string().contains("undefined variable"));
    }

    #[test]
    fn failed_check_source_does_not_mutate_state() {
        let rt = DByteRuntime::new();
        let error = rt
            .check_source("test", "let bad: int = \"bad\"")
            .unwrap_err();
        assert!(matches!(error, DByteError::Type { .. }));
        let error = rt.check_source("test", "print(bad)").unwrap_err();
        assert!(error.to_string().contains("undefined variable"));
    }

    #[test]
    fn type_error_rolls_back_state() {
        let mut rt = DByteRuntime::new();
        let error = rt.run_source("test", "let bad: int = \"bad\"").unwrap_err();
        assert!(matches!(error, DByteError::Type { .. }));
        let error = rt.run_source("test", "print(bad)").unwrap_err();
        assert!(error.to_string().contains("undefined variable"));
    }

    #[test]
    fn runtime_error_rolls_back_state() {
        let mut rt = DByteRuntime::new();
        let error = rt.run_source("test", "let bad: int = 1 / 0").unwrap_err();
        assert!(matches!(error, DByteError::Runtime { .. }));
        let error = rt.run_source("test", "print(bad)").unwrap_err();
        assert!(error.to_string().contains("undefined variable"));
    }

    #[test]
    fn run_file_capture_executes_file() {
        let root = temp_dir("run-file");
        fs::write(root.join("main.dby"), "print(\"from file\")\n").unwrap();
        let mut rt = DByteRuntime::with_current_dir(&root).unwrap();
        let out = rt.run_file_capture("main.dby").unwrap();
        assert_eq!(out.stdout.trim(), "from file");
        cleanup(root);
    }

    #[test]
    fn invalid_set_current_dir_keeps_previous_dir() {
        let root = temp_dir("invalid-cwd");
        let mut rt = DByteRuntime::with_current_dir(&root).unwrap();
        let before = rt.current_dir().to_path_buf();
        let error = rt.set_current_dir(root.join("missing")).unwrap_err();
        assert!(matches!(error, DByteError::Io { .. }));
        assert_eq!(rt.current_dir(), before.as_path());
        cleanup(root);
    }

    #[test]
    fn load_rc_preserves_state_and_supports_local_imports() {
        let root = temp_dir("rc-success");
        fs::write(
            root.join("helper.dby"),
            "pub fn inc(x: int) -> int:\n    return x + 1\n",
        )
        .unwrap();
        fs::write(
            root.join(".dbyterc"),
            "import std.math as math\nimport \"./helper.dby\" as helper\nlet boot: int = math.max(helper.inc(40), 1)\n",
        )
        .unwrap();
        let mut rt = DByteRuntime::with_current_dir(&root).unwrap();
        rt.load_rc().unwrap();
        let out = rt
            .run_source_capture("test", "print(boot + 1)\nprint(helper.inc(1))")
            .unwrap();
        assert_eq!(out.stdout.trim(), "42\n2");
        cleanup(root);
    }

    #[test]
    fn load_rc_is_noop_after_success_in_same_directory() {
        let root = temp_dir("rc-repeat");
        fs::write(root.join(".dbyterc"), "let boot: int = 41\n").unwrap();
        let mut rt = DByteRuntime::with_current_dir(&root).unwrap();
        rt.load_rc().unwrap();
        rt.load_rc().unwrap();
        let out = rt.run_source_capture("test", "print(boot + 1)").unwrap();
        assert_eq!(out.stdout.trim(), "42");
        cleanup(root);
    }

    #[test]
    fn load_rc_after_current_dir_change_loads_new_rc() {
        let root = temp_dir("rc-cwd-change");
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join(".dbyterc"), "let first_value: int = 10\n").unwrap();
        fs::write(second.join(".dbyterc"), "let second_value: int = 32\n").unwrap();

        let mut rt = DByteRuntime::with_current_dir(&first).unwrap();
        rt.load_rc().unwrap();
        rt.set_current_dir(&second).unwrap();
        rt.load_rc().unwrap();
        let out = rt
            .run_source_capture("test", "print(first_value + second_value)")
            .unwrap();
        assert_eq!(out.stdout.trim(), "42");
        cleanup(root);
    }

    #[test]
    fn bad_rc_rolls_back_partial_state() {
        let root = temp_dir("rc-partial-rollback");
        fs::write(
            root.join(".dbyterc"),
            "let partial: int = 1\nlet bad: int = \"bad\"\n",
        )
        .unwrap();
        let mut rt = DByteRuntime::with_current_dir(&root).unwrap();
        let error = rt.load_rc().unwrap_err();
        assert!(matches!(error, DByteError::Rc { .. }));
        let error = rt.run_source("test", "print(partial)").unwrap_err();
        assert!(error.to_string().contains("undefined variable"));
        cleanup(root);
    }

    #[test]
    fn load_rc_reports_bad_rc() {
        let root = temp_dir("rc-error");
        fs::write(root.join(".dbyterc"), "let bad: int = \"bad\"\n").unwrap();
        let mut rt = DByteRuntime::with_current_dir(&root).unwrap();
        let error = rt.load_rc().unwrap_err();
        assert!(matches!(error, DByteError::Rc { .. }));
        assert!(error.to_string().contains("RcError"));
        cleanup(root);
    }

    #[test]
    fn error_display_prefixes_are_stable() {
        let root = temp_dir("display-prefixes");
        let io = match DByteRuntime::with_current_dir(root.join("missing")) {
            Ok(_) => panic!("missing current_dir unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(io.to_string().starts_with("IoError:"));

        let mut rt = DByteRuntime::with_current_dir(&root).unwrap();
        assert!(rt
            .run_source("test", "let x: int = \"bad\"")
            .unwrap_err()
            .to_string()
            .starts_with("TypeError:"));
        assert!(rt
            .run_source("test", "let x: int = 1 / 0")
            .unwrap_err()
            .to_string()
            .starts_with("RuntimeError:"));
        assert!(rt
            .run_source("test", "let x: int = ")
            .unwrap_err()
            .to_string()
            .starts_with("ParseError:"));
        assert!(rt
            .run_source("test", "let s: str = b\"\\xGG\"")
            .unwrap_err()
            .to_string()
            .starts_with("LexError:"));

        fs::write(root.join(".dbyterc"), "let bad: int = \"bad\"\n").unwrap();
        assert!(rt
            .load_rc()
            .unwrap_err()
            .to_string()
            .starts_with("RcError:"));
        cleanup(root);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("dbyte-embed-{}-{}", name, nanos));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn cleanup(path: PathBuf) {
        let _ = fs::remove_dir_all(path);
    }
}
