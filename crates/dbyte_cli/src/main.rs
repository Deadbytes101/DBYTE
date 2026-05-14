use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process;

use dbyte_compiler::Compiler;
use dbyte_embed::{DByteError, DByteRuntime};
use dbyte_interp::Interpreter;
use dbyte_lexer::Lexer;
use dbyte_parser::Parser;
use dbyte_project::{create_project, find_project_root, load_project, ProjectError};
use dbyte_typeck::TypeChecker;
use dbyte_vm::Vm;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    Tree,
    Vm,
}

impl Engine {
    fn label(self) -> &'static str {
        match self {
            Engine::Tree => "tree",
            Engine::Vm => "vm",
        }
    }
}

fn print_error(label: &str, msg: &str, span: dbyte_ast::Span, path: &str, src: &str) {
    let line_content = src.lines().nth(span.line - 1).unwrap_or("");
    eprintln!("\x1b[1;31m{}\x1b[0m: {}", label, msg);
    eprintln!(" \x1b[1;34m-->\x1b[0m {}:{}:{}", path, span.line, span.col);
    eprintln!("  \x1b[1;34m|\x1b[0m");
    eprintln!("\x1b[1;34m{:>3} |\x1b[0m {}", span.line, line_content);
    let arrow = " ".repeat(span.col.saturating_sub(1));
    eprintln!("  \x1b[1;34m|\x1b[0m {}\x1b[1;31m^\x1b[0m", arrow);
    eprintln!();
}

fn print_project_error(error: ProjectError) -> ! {
    eprintln!("ProjectError: {}", error);
    process::exit(1);
}

fn resolve_rc_path(cwd: &Path, rc_override: Option<&Path>) -> PathBuf {
    match rc_override {
        None => cwd.join(".dbyterc"),
        Some(p) if p.is_absolute() => p.to_path_buf(),
        Some(p) => cwd.join(p),
    }
}

/// When the shell cwd is the repository root, `run bin/foo.dby` should still find
/// scripts under `examples/dbyteos/` so DByteOS aliases work with `shell --rc`.
fn resolve_shell_run_script(cwd: &Path, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    let primary = cwd.join(rel);
    if primary.exists() {
        return primary;
    }
    let under_os = cwd.join("examples").join("dbyteos").join(rel);
    if under_os.exists() {
        under_os
    } else {
        primary
    }
}

/// DByteOS command search roots; keep in sync with `examples/dbyteos/etc/cmd_path_roots.txt`
/// and `command_search_roots()` in `examples/dbyteos/sys/session.dby`.
fn dbyteos_cmd_search_roots(cwd: &Path) -> Vec<String> {
    let candidates = [
        cwd.join("etc").join("cmd_path_roots.txt"),
        cwd.join("examples")
            .join("dbyteos")
            .join("etc")
            .join("cmd_path_roots.txt"),
    ];
    let mut found_path: Option<PathBuf> = None;
    for p in candidates {
        if p.is_file() {
            found_path = Some(p);
            break;
        }
    }
    let Some(path) = found_path else {
        return vec![
            "bin".to_string(),
            "tmp".to_string(),
            "home/deadbyte".to_string(),
        ];
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return vec![
            "bin".to_string(),
            "tmp".to_string(),
            "home/deadbyte".to_string(),
        ];
    };
    let roots: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect();
    if roots.is_empty() {
        vec![
            "bin".to_string(),
            "tmp".to_string(),
            "home/deadbyte".to_string(),
        ]
    } else {
        roots
    }
}

fn command_name_to_script_candidates(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let primary = format!("{name}.dby");
    let underscored = name.replace('-', "_") + ".dby";
    out.push(primary.clone());
    if underscored != primary {
        out.push(underscored);
    }
    out
}

fn validate_autopath_command_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("command name cannot be empty".into());
    }
    if name.starts_with('.') || name.starts_with(':') || name.starts_with('@') {
        return Err(format!("invalid command name: {}", name));
    }
    for ch in name.chars() {
        if ch.is_whitespace()
            || matches!(
                ch,
                '/' | '\\' | '<' | '>' | '|' | ';' | '&' | '(' | ')' | '"' | '\''
            )
        {
            return Err(format!("invalid command name: {}", name));
        }
    }
    Ok(())
}

fn resolve_dbyteos_command_script(cwd: &Path, name: &str) -> Option<PathBuf> {
    validate_autopath_command_name(name).ok()?;
    let roots = dbyteos_cmd_search_roots(cwd);
    let scripts = command_name_to_script_candidates(name);
    for root in &roots {
        for script in &scripts {
            let rel = format!("{root}/{script}");
            let candidate = resolve_shell_run_script(cwd, &rel);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn format_shell_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn parse_file(path: &Path) -> (String, dbyte_ast::Program) {
    let path_label = path.display().to_string();
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "\x1b[1;31merror\x1b[0m: cannot read `{}`: {}",
                path_label, e
            );
            process::exit(1);
        }
    };

    let tokens = match Lexer::new(&src).tokenize() {
        Ok(t) => t,
        Err(e) => {
            print_error("LexError", &e.msg, e.span, &path_label, &src);
            process::exit(1);
        }
    };

    let program = match Parser::new(tokens).parse_program() {
        Ok(p) => p,
        Err(e) => {
            print_error("ParseError", &e.msg, e.span, &path_label, &src);
            process::exit(1);
        }
    };

    (src, program)
}

fn lex_file(path: &Path) -> (String, Vec<dbyte_lexer::Token>) {
    let path_label = path.display().to_string();
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "\x1b[1;31merror\x1b[0m: cannot read `{}`: {}",
                path_label, e
            );
            process::exit(1);
        }
    };
    let tokens = match Lexer::new(&src).tokenize() {
        Ok(t) => t,
        Err(e) => {
            print_error("LexError", &e.msg, e.span, &path_label, &src);
            process::exit(1);
        }
    };
    (src, tokens)
}

fn check_program(path: &Path, src: &str, program: &dbyte_ast::Program) {
    let path_label = path.display().to_string();
    let mut checker = TypeChecker::with_entry_path(path.to_path_buf());
    if let Err(e) = checker.check_program(program) {
        print_error("TypeError", &e.msg, e.span, &path_label, src);
        process::exit(1);
    }
}

fn compile_program(path: &Path, src: &str, program: &dbyte_ast::Program) -> dbyte_bytecode::Chunk {
    let path_label = path.display().to_string();
    match Compiler::with_entry_path(path.to_path_buf()).compile_program(program) {
        Ok(chunk) => chunk,
        Err(e) => {
            print_error("CompileError", &e.msg, e.span, &path_label, src);
            process::exit(1);
        }
    }
}

fn parse_source(path_label: &str, src: &str) -> Result<dbyte_ast::Program, ()> {
    let tokens = match Lexer::new(src).tokenize() {
        Ok(tokens) => tokens,
        Err(e) => {
            print_error("LexError", &e.msg, e.span, path_label, src);
            return Err(());
        }
    };

    Parser::new(tokens).parse_program().map_err(|e| {
        print_error("ParseError", &e.msg, e.span, path_label, src);
    })
}

#[derive(Clone)]
struct InteractiveSession {
    checker: TypeChecker,
    interp: Interpreter,
    entry_path: PathBuf,
}

impl InteractiveSession {
    fn new(cwd: &Path) -> Self {
        let entry_path = cwd.join("<interactive>");
        Self {
            checker: TypeChecker::with_entry_path(entry_path.clone()),
            interp: Interpreter::with_entry_path(entry_path.clone()),
            entry_path,
        }
    }

    fn reset(&mut self) {
        let entry_path = self.entry_path.clone();
        self.checker = TypeChecker::with_entry_path(entry_path.clone());
        self.interp = Interpreter::with_entry_path(entry_path);
    }

    fn eval(&mut self, label: &str, src: &str) -> bool {
        let Ok(program) = parse_source(label, src) else {
            return false;
        };

        let checker_snapshot = self.checker.clone();
        let interp_snapshot = self.interp.clone();

        if let Err(e) = self.checker.check_program(&program) {
            print_error("TypeError", &e.msg, e.span, label, src);
            self.checker = checker_snapshot;
            return false;
        }

        if let Err(e) = self.interp.run(&program) {
            print_error("RuntimeError", &e.msg, e.span, label, src);
            self.checker = checker_snapshot;
            self.interp = interp_snapshot;
            return false;
        }

        true
    }

    fn load_rc(&mut self, cwd: &Path, rc_override: Option<&Path>) -> bool {
        let rc_path = resolve_rc_path(cwd, rc_override);
        if rc_override.is_some() && !rc_path.exists() {
            eprintln!("RcError: --rc file not found: {}", rc_path.display());
            return false;
        }
        if !rc_path.exists() {
            return true;
        }
        let src = match fs::read_to_string(&rc_path) {
            Ok(src) => src,
            Err(e) => {
                eprintln!("RcError: failed to read {}: {}", rc_path.display(), e);
                return false;
            }
        };
        let src = strip_shell_directives(&src);
        if self.eval(&rc_path.display().to_string(), &src) {
            true
        } else {
            eprintln!("RcError: failed to load {}", rc_path.display());
            false
        }
    }
}

fn starts_multiline_block(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.ends_with(':')
        && (trimmed.starts_with("fn ")
            || trimmed.starts_with("if ")
            || trimmed.starts_with("while ")
            || trimmed.starts_with("for "))
}

fn print_repl_help() {
    println!("DByte REPL commands:");
    println!("  .help          show this help");
    println!("  .reset         clear variables, functions, imports, and module state");
    println!("  .quit, .exit   leave the REPL");
    println!("Use a blank line to finish multiline fn/if/while/for blocks.");
}

fn repl_loop(session: &mut InteractiveSession) {
    let interactive = io::stdin().is_terminal();
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut buffer = String::new();
    let mut collecting_block = false;

    loop {
        if interactive {
            let prompt = if collecting_block { "... " } else { "dbyte> " };
            print!("{}", prompt);
            let _ = io::stdout().flush();
        }

        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("ReplError: failed to read input: {}", e);
                break;
            }
        }

        let line_no_newline = line
            .trim_end_matches(['\r', '\n'])
            .trim_start_matches('\u{feff}');
        let trimmed = line_no_newline.trim();

        if !collecting_block && trimmed.starts_with('.') {
            match trimmed {
                ".help" => print_repl_help(),
                ".quit" | ".exit" => break,
                ".reset" => {
                    session.reset();
                    println!("reset");
                }
                _ => eprintln!("ReplError: unknown command: {}", trimmed),
            }
            continue;
        }

        if collecting_block {
            if trimmed.is_empty() {
                let src = buffer.trim_end().to_string();
                if !src.is_empty() {
                    session.eval("<repl>", &src);
                }
                buffer.clear();
                collecting_block = false;
            } else {
                buffer.push_str(line_no_newline);
                buffer.push('\n');
            }
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        if starts_multiline_block(line_no_newline) {
            collecting_block = true;
            buffer.push_str(line_no_newline);
            buffer.push('\n');
        } else {
            session.eval("<repl>", line_no_newline);
        }
    }
}

fn split_shell_command(line: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }

    if in_quotes {
        return Err("unterminated quote".into());
    }
    if !current.is_empty() {
        args.push(current);
    }
    Ok(args)
}

fn strip_shell_directives(src: &str) -> String {
    src.lines()
        .filter(|line| !line.trim_start().starts_with("@shell "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Clone, Copy)]
struct ShellCommand {
    name: &'static str,
    usage: &'static str,
    description: &'static str,
}

const SHELL_COMMANDS: &[ShellCommand] = &[
    ShellCommand {
        name: "help",
        usage: "help",
        description: "show shell command help",
    },
    ShellCommand {
        name: "version",
        usage: "version",
        description: "print DByte version",
    },
    ShellCommand {
        name: "pwd",
        usage: "pwd",
        description: "print shell current directory",
    },
    ShellCommand {
        name: "cd",
        usage: "cd <path>",
        description: "change shell current directory",
    },
    ShellCommand {
        name: "ls",
        usage: "ls",
        description: "list shell current directory",
    },
    ShellCommand {
        name: "run",
        usage: "run <file.dby>",
        description: "run a DByte file in persistent shell state",
    },
    ShellCommand {
        name: "check",
        usage: "check <file.dby>",
        description: "type-check a DByte file",
    },
    ShellCommand {
        name: "test",
        usage: "test",
        description: "run dbyte test from the shell current directory",
    },
    ShellCommand {
        name: "repl",
        usage: "repl",
        description: "enter the DByte REPL",
    },
    ShellCommand {
        name: "alias",
        usage: "alias <name> = <command>",
        description: "define or replace a shell alias",
    },
    ShellCommand {
        name: "unalias",
        usage: "unalias <name>",
        description: "remove a shell alias",
    },
    ShellCommand {
        name: "aliases",
        usage: "aliases",
        description: "list shell aliases",
    },
    ShellCommand {
        name: "which",
        usage: "which <name>",
        description: "show whether a name is built-in or alias",
    },
    ShellCommand {
        name: "clear",
        usage: "clear",
        description: "clear the terminal",
    },
    ShellCommand {
        name: "exit",
        usage: "exit",
        description: "leave the shell",
    },
    ShellCommand {
        name: "quit",
        usage: "quit",
        description: "leave the shell",
    },
];

const ALIAS_EXPANSION_LIMIT: usize = 16;

fn shell_command(name: &str) -> Option<&'static ShellCommand> {
    SHELL_COMMANDS.iter().find(|command| command.name == name)
}

fn print_shell_help() {
    println!("DByte shell commands:");
    for command in SHELL_COMMANDS {
        println!("  {:<24} {}", command.usage, command.description);
    }
    println!(
        "  {:<24} execute DByte code in persistent shell state",
        ": <code>"
    );
}

struct ShellSession {
    runtime: DByteRuntime,
    aliases: HashMap<String, String>,

    dbyteos_autopath: bool,
}

impl ShellSession {
    fn new(cwd: &Path) -> Result<Self, DByteError> {
        Ok(Self {
            runtime: DByteRuntime::with_current_dir(cwd)?,
            aliases: HashMap::new(),
            dbyteos_autopath: false,
        })
    }

    fn load_rc(&mut self, rc_override: Option<&Path>) -> bool {
        let cwd = self.runtime.current_dir();
        let rc_path = resolve_rc_path(cwd, rc_override);
        if rc_override.is_some() && !rc_path.exists() {
            eprintln!("RcError: --rc file not found: {}", rc_path.display());
            return false;
        }
        if !rc_path.exists() {
            return true;
        }
        let src = match fs::read_to_string(&rc_path) {
            Ok(src) => src,
            Err(e) => {
                eprintln!("RcError: failed to read {}: {}", rc_path.display(), e);
                return false;
            }
        };

        let mut dbyte_lines = Vec::new();
        for (idx, line) in src.lines().enumerate() {
            let trimmed = line.trim_start();
            if let Some(directive) = trimmed.strip_prefix("@shell ") {
                if let Err(e) = self.apply_shell_directive(directive) {
                    eprintln!(
                        "ShellError: {} line {}: {}\n  {}",
                        rc_path.display(),
                        idx + 1,
                        e,
                        line.trim_end()
                    );
                    return false;
                }
            } else {
                dbyte_lines.push(line);
            }
        }

        let dbyte_src = dbyte_lines.join("\n");
        if dbyte_src.trim().is_empty() {
            return true;
        }
        match self
            .runtime
            .run_source(&rc_path.display().to_string(), &dbyte_src)
        {
            Ok(()) => true,
            Err(e) => {
                eprintln!("RcError: failed to load {}: {}", rc_path.display(), e);
                false
            }
        }
    }

    fn apply_shell_directive(&mut self, directive: &str) -> Result<(), String> {
        let d = directive.trim();
        if let Some(alias_def) = d.strip_prefix("alias ") {
            let args = split_shell_command(alias_def)?;
            let (name, command) = parse_alias_definition(&args)?;
            self.set_alias(name, command)
        } else if d == "dbyteos_autopath on" {
            self.dbyteos_autopath = true;
            Ok(())
        } else if d == "dbyteos_autopath off" {
            self.dbyteos_autopath = false;
            Ok(())
        } else {
            Err(format!("unknown shell directive: {}", d))
        }
    }

    fn set_alias(&mut self, name: String, command: String) -> Result<(), String> {
        validate_alias_name(&name)?;

        self.aliases.insert(name, command);
        Ok(())
    }

    fn eval_code(&mut self, label: &str, code: &str) {
        match self.runtime.run_source_capture(label, code) {
            Ok(output) => print!("{}", output.stdout),
            Err(e) => eprintln!("{}", e),
        }
    }

    fn execute_line(&mut self, line: &str) -> bool {
        if let Some(code) = line.strip_prefix(':') {
            self.eval_code("<shell>", code.trim_start());
            return true;
        }

        let args = match split_shell_command(line) {
            Ok(args) => args,
            Err(e) => {
                eprintln!("ShellError: {}", e);
                return true;
            }
        };
        self.execute_args(args)
    }

    fn expand_aliases(&self, args: Vec<String>) -> Result<Vec<String>, String> {
        let mut current = args;
        let mut seen = Vec::<String>::new();
        let mut hops = 0usize;

        loop {
            if current.is_empty() {
                return Ok(current);
            }
            let name = current[0].clone();
            let Some(expansion) = self.aliases.get(&name) else {
                return Ok(current);
            };

            if let Some(first_seen) = seen.iter().position(|seen_name| seen_name == &name) {
                let mut cycle = seen[first_seen..].to_vec();
                cycle.push(name);
                return Err(format!(
                    "alias expansion cycle detected: {}",
                    cycle.join(" -> ")
                ));
            }
            if hops >= ALIAS_EXPANSION_LIMIT {
                return Err("alias expansion limit exceeded".into());
            }
            seen.push(name.clone());
            hops += 1;

            let mut expanded = split_shell_command(expansion)
                .map_err(|e| format!("alias `{}` is invalid: {}", name, e))?;
            expanded.extend_from_slice(&current[1..]);
            current = expanded;
        }
    }

    fn execute_args(&mut self, args: Vec<String>) -> bool {
        let args = match self.expand_aliases(args) {
            Ok(args) => args,
            Err(e) => {
                eprintln!("ShellError: {}", e);
                return true;
            }
        };

        if args.is_empty() {
            return true;
        }

        match args[0].as_str() {
            "help" => print_shell_help(),
            "quit" | "exit" => return false,
            "clear" => print!("\x1b[2J\x1b[H"),
            "pwd" => println!("{}", self.runtime.current_dir().display()),
            "cd" => self.command_cd(&args),
            "ls" => self.command_ls(&args),
            "run" => self.command_run(&args),
            "check" => self.command_check(&args),
            "test" => self.command_test(&args),
            "version" => print_version(),
            "repl" => self.command_repl(&args),
            "alias" => self.command_alias(&args),
            "unalias" => self.command_unalias(&args),
            "aliases" => self.command_aliases(&args),
            "which" => self.command_which(&args),
            other => {
                if self.dbyteos_autopath {
                    if let Err(e) = validate_autopath_command_name(other) {
                        eprintln!("ShellError: {}", e);
                    } else if let Some(script) =
                        resolve_dbyteos_command_script(self.runtime.current_dir(), other)
                    {
                        match self
                            .runtime
                            .run_file_capture_with_args(&script, args[1..].to_vec())
                        {
                            Ok(output) => print!("{}", output.stdout),
                            Err(e) => eprintln!("{}", e),
                        }
                    } else {
                        eprintln!("ShellError: unknown command: {}", other);
                    }
                } else {
                    eprintln!("ShellError: unknown command: {}", other);
                }
            }
        }
        true
    }

    fn command_cd(&mut self, args: &[String]) {
        if args.len() != 2 {
            eprintln!("ShellError: cd expects 1 path");
            return;
        }
        let path = PathBuf::from(&args[1]);
        if let Err(e) = self.runtime.set_current_dir(&path) {
            eprintln!("ShellError: failed to cd `{}`: {}", path.display(), e);
        }
    }

    fn command_ls(&self, args: &[String]) {
        if args.len() != 1 {
            eprintln!("ShellError: ls expects 0 args");
            return;
        }
        match fs::read_dir(self.runtime.current_dir()) {
            Ok(entries) => {
                let mut names = entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                names.sort();
                for name in names {
                    println!("{}", name);
                }
            }
            Err(e) => eprintln!("ShellError: failed to list directory: {}", e),
        }
    }

    fn command_run(&mut self, args: &[String]) {
        if args.len() < 2 {
            eprintln!("ShellError: run expects a file");
            return;
        }
        let script_path = resolve_shell_run_script(self.runtime.current_dir(), &args[1]);
        match self
            .runtime
            .run_file_capture_with_args(&script_path, args[2..].to_vec())
        {
            Ok(output) => print!("{}", output.stdout),
            Err(e) => eprintln!("{}", e),
        }
    }

    fn command_check(&self, args: &[String]) {
        if args.len() != 2 {
            eprintln!("ShellError: check expects 1 file");
            return;
        }
        match self.runtime.check_file(&args[1]) {
            Ok(()) => println!(
                "\x1b[1;32mok\x1b[0m: no type errors found in `{}`",
                self.runtime.current_dir().join(&args[1]).display()
            ),
            Err(e) => eprintln!("{}", e),
        }
    }

    fn command_test(&self, args: &[String]) {
        if args.len() != 1 {
            eprintln!("ShellError: test expects 0 args");
            return;
        }
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(e) => {
                eprintln!("ShellError: failed to find current executable: {}", e);
                return;
            }
        };
        match process::Command::new(exe)
            .arg("test")
            .current_dir(self.runtime.current_dir())
            .output()
        {
            Ok(output) => {
                print!("{}", String::from_utf8_lossy(&output.stdout));
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
                if !output.status.success() {
                    eprintln!("ShellError: test failed");
                }
            }
            Err(e) => eprintln!("ShellError: failed to run test: {}", e),
        }
    }

    fn command_repl(&self, args: &[String]) {
        if args.len() != 1 {
            eprintln!("ShellError: repl expects 0 args");
            return;
        }
        let mut session = InteractiveSession::new(self.runtime.current_dir());
        repl_loop(&mut session);
    }

    fn command_alias(&mut self, args: &[String]) {
        let (name, command) = match parse_alias_definition(&args[1..]) {
            Ok(definition) => definition,
            Err(e) => {
                eprintln!("ShellError: {}", e);
                return;
            }
        };
        if let Err(e) = self.set_alias(name, command) {
            eprintln!("ShellError: {}", e);
        }
    }

    fn command_unalias(&mut self, args: &[String]) {
        if args.len() != 2 {
            eprintln!("ShellError: unalias expects 1 name");
            return;
        }
        if self.aliases.remove(&args[1]).is_none() {
            eprintln!("ShellError: alias not found: {}", args[1]);
        }
    }

    fn command_aliases(&self, args: &[String]) {
        if args.len() != 1 {
            eprintln!("ShellError: aliases expects 0 args");
            return;
        }
        let mut aliases = self.aliases.iter().collect::<Vec<_>>();
        aliases.sort_by(|a, b| a.0.cmp(b.0));
        for (name, command) in aliases {
            println!("{} = {}", name, command);
        }
    }

    fn command_which(&self, args: &[String]) {
        if args.len() != 2 {
            eprintln!("ShellError: which expects 1 name");
            return;
        }
        let name = &args[1];
        if let Some(command) = self.aliases.get(name) {
            println!("{}: alias -> {}", name, command);
        } else if shell_command(name).is_some() {
            println!("{}: built-in", name);
        } else if self.dbyteos_autopath {
            if let Some(script) = resolve_dbyteos_command_script(self.runtime.current_dir(), name) {
                println!("{}: dbyteos -> {}", name, format_shell_path(&script));
            } else {
                println!("{}: not found", name);
            }
        } else {
            println!("{}: not found", name);
        }
    }
}

fn parse_alias_definition(args: &[String]) -> Result<(String, String), String> {
    if args.len() < 3 {
        return Err("alias expects: alias <name> = <command>".into());
    }
    if args[1] != "=" {
        return Err("alias expects `=` between name and command".into());
    }
    let command = args[2..].join(" ");
    if command.trim().is_empty() {
        return Err("alias command cannot be empty".into());
    }
    Ok((args[0].clone(), command))
}

fn validate_alias_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("alias name cannot be empty".into());
    }
    if name.starts_with('.') || name.starts_with(':') || name.starts_with('@') {
        return Err(format!("invalid alias name: {}", name));
    }
    if name.chars().any(char::is_whitespace) {
        return Err(format!("invalid alias name: {}", name));
    }
    Ok(())
}

fn shell_loop(session: &mut ShellSession) {
    let interactive = io::stdin().is_terminal();
    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        if interactive {
            print!("dbyte-shell> ");
            let _ = io::stdout().flush();
        }

        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("ShellError: failed to read input: {}", e);
                break;
            }
        }

        let line = line
            .trim_end_matches(['\r', '\n'])
            .trim_start_matches('\u{feff}');
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if !session.execute_line(trimmed) {
            break;
        }
    }
}

fn cmd_repl(no_rc: bool, rc_path: Option<&Path>) {
    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("ReplError: failed to read current directory: {}", e);
        process::exit(1);
    });
    let mut session = InteractiveSession::new(&cwd);
    if !no_rc && !session.load_rc(&cwd, rc_path) {
        process::exit(1);
    }
    repl_loop(&mut session);
}

fn cmd_shell(no_rc: bool, rc_path: Option<&Path>) {
    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("ShellError: failed to read current directory: {}", e);
        process::exit(1);
    });
    let mut session = ShellSession::new(&cwd).unwrap_or_else(|e| {
        eprintln!("ShellError: failed to create shell runtime: {}", e);
        process::exit(1);
    });
    if !no_rc && !session.load_rc(rc_path) {
        process::exit(1);
    }
    shell_loop(&mut session);
}

fn cmd_run(path: &Path, type_check: bool, engine: Engine, trace: bool, script_args: Vec<String>) {
    let path_label = path.display().to_string();
    let (src, program) = parse_file(path);
    if type_check {
        check_program(path, &src, &program);
    }

    match engine {
        Engine::Tree => {
            let mut interp = Interpreter::with_entry_path(path.to_path_buf());
            interp.set_script_args(script_args);
            if let Err(e) = interp.run(&program) {
                print_error("RuntimeError", &e.msg, e.span, &path_label, &src);
                process::exit(1);
            }
        }
        Engine::Vm => {
            let chunk = compile_program(path, &src, &program);
            let mut vm = Vm::with_entry_path(path.to_path_buf());
            vm.set_trace(trace);
            vm.set_script_args(script_args);
            if let Err(e) = vm.run(&chunk) {
                print_error("RuntimeError", &e.msg, e.span, &path_label, &src);
                process::exit(1);
            }
        }
    }
}

fn cmd_run_project(type_check: bool, engine: Engine, trace: bool) {
    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("ProjectError: failed to read current directory: {}", e);
        process::exit(1);
    });
    let project = match load_project(&cwd) {
        Ok(project) => project,
        Err(error) => print_project_error(error),
    };
    std::env::set_current_dir(&project.root).unwrap_or_else(|e| {
        eprintln!("ProjectError: failed to enter project root: {}", e);
        process::exit(1);
    });
    cmd_run(&project.entry_path, type_check, engine, trace, Vec::new());
}

fn cmd_check(path: &Path) {
    let path_label = path.display().to_string();
    let (src, program) = parse_file(path);
    let mut checker = TypeChecker::with_entry_path(path.to_path_buf());
    match checker.check_program(&program) {
        Ok(_) => println!(
            "\x1b[1;32mok\x1b[0m: no type errors found in `{}`",
            path_label
        ),
        Err(e) => {
            print_error("TypeError", &e.msg, e.span, &path_label, &src);
            process::exit(1);
        }
    }
}

fn cmd_check_project() {
    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("ProjectError: failed to read current directory: {}", e);
        process::exit(1);
    });
    let project = match load_project(&cwd) {
        Ok(project) => project,
        Err(error) => print_project_error(error),
    };
    std::env::set_current_dir(&project.root).unwrap_or_else(|e| {
        eprintln!("ProjectError: failed to enter project root: {}", e);
        process::exit(1);
    });
    cmd_check(&project.entry_path);
}

fn cmd_new(name: &str) {
    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("ProjectError: failed to read current directory: {}", e);
        process::exit(1);
    });
    if let Err(error) = create_project(&cwd, name) {
        print_project_error(error);
    }
    println!("created DByte project `{}`", name);
}

fn cmd_disasm(path: &Path) {
    let (src, program) = parse_file(path);
    check_program(path, &src, &program);
    let chunk = compile_program(path, &src, &program);
    print!("{}", chunk.disassemble());
}

fn cmd_tokens(path: &Path) {
    let (_src, tokens) = lex_file(path);
    for token in tokens {
        println!("{} {}", token.span, token.kind);
    }
}

fn cmd_ast(path: &Path) {
    let (_src, program) = parse_file(path);
    println!("{:#?}", program);
}

fn usage() {
    println!(
        "DByte - low-level scripting language\n\n\
         Usage:\n  \
         dbyte run [--vm] <file> [arg ...]\n  \
         dbyte check <file>\n  \
         dbyte test [--engine tree|vm]\n  \
         dbyte bench [--engine tree|vm] [--compare-python]\n  \
         dbyte repl [--no-rc] [--rc <path>]\n  \
         dbyte shell [--no-rc] [--rc <path>]\n  \
         dbyte disasm <file>\n  \
         dbyte tokens <file>\n  \
         dbyte ast <file>\n  \
         dbyte new <name>\n  \
         dbyte --version"
    );
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape && c == 'm' {
            in_escape = false;
        } else if !in_escape {
            out.push(c);
        }
    }
    out
}

fn cmd_test(engine: Engine) {
    let mut passed = 0;
    let mut failed = 0;

    println!(
        "\x1b[1mRunning DByte Tests [engine={}]...\x1b[0m",
        engine.label()
    );

    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("TestError: failed to read current directory: {}", e);
        process::exit(1);
    });
    let test_root = find_project_root(&cwd).unwrap_or(cwd);
    let test_dir = test_root.join("tests");

    let mut cases = Vec::new();
    collect_tests(&test_dir, &mut cases);
    cases.sort();

    if cases.is_empty() {
        eprintln!("TestError: no DByte tests found");
        process::exit(1);
    }

    for path in cases {
        let out_path = path.with_extension("out");
        let err_path = path.with_extension("err");
        if !out_path.exists() && !err_path.exists() {
            continue;
        }
        let mut command = process::Command::new(std::env::current_exe().unwrap());
        command.arg("run");
        if engine == Engine::Vm {
            command.arg("--vm");
        }
        let output = command.arg(&path).current_dir(&test_root).output().unwrap();

        let stdout = strip_ansi(&String::from_utf8_lossy(&output.stdout))
            .replace("\r\n", "\n")
            .trim()
            .to_string();
        let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr))
            .replace("\r\n", "\n")
            .trim()
            .to_string();

        let mut ok = true;
        let mut reason = String::new();

        if out_path.exists() {
            let expected = fs::read_to_string(&out_path)
                .unwrap()
                .replace("\r\n", "\n")
                .trim()
                .to_string();
            if stdout != expected {
                ok = false;
                reason = format!("Expected stdout:\n{}\nGot:\n{}", expected, stdout);
            }
        } else if err_path.exists() {
            let expected = fs::read_to_string(&err_path)
                .unwrap()
                .replace("\r\n", "\n")
                .trim()
                .to_string();
            if !stderr.contains(&expected) {
                ok = false;
                reason = format!(
                    "Expected stderr to contain:\n{}\nGot:\n{}",
                    expected, stderr
                );
            }
        }

        let path_str = path.to_string_lossy();
        if ok {
            println!("test {} ... \x1b[32mok\x1b[0m", path_str);
            passed += 1;
        } else {
            println!("test {} ... \x1b[31mFAILED\x1b[0m", path_str);
            println!("{}", reason);
            failed += 1;
        }
    }

    if passed + failed == 0 {
        eprintln!("TestError: no DByte tests found");
        process::exit(1);
    }

    println!("\nTest result: {} passed, {} failed", passed, failed);
    if failed > 0 {
        process::exit(1);
    }
}

fn run_benchmark(path: &Path, engine: Engine) -> f64 {
    let (_src, program) = parse_file(path);
    let start = std::time::Instant::now();

    match engine {
        Engine::Tree => {
            let mut interp = Interpreter::with_entry_path(path.to_path_buf());
            let _ = interp.run(&program);
        }
        Engine::Vm => {
            let compiler = Compiler::with_entry_path(path.to_path_buf());
            if let Ok(chunk) = compiler.compile_program(&program) {
                let mut vm = Vm::with_entry_path(path.to_path_buf());
                let _ = vm.run(&chunk);
            }
        }
    }

    start.elapsed().as_secs_f64() * 1000.0
}

fn python_executable() -> Option<&'static str> {
    ["python", "py"].into_iter().find(|candidate| {
        process::Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    })
}

fn run_python_benchmark(python: &str, path: &Path) -> Result<f64, String> {
    let output = process::Command::new(python)
        .arg(path)
        .output()
        .map_err(|e| format!("failed to run python benchmark `{}`: {}", path.display(), e))?;
    if !output.status.success() {
        return Err(format!(
            "python benchmark `{}` failed: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .trim()
        .parse::<f64>()
        .map_err(|e| format!("invalid python benchmark output `{}`: {}", stdout.trim(), e))
}

fn cmd_bench(engine_override: Option<Engine>, compare_python: bool) {
    let benchmarks_dir = Path::new("benchmarks");
    if !benchmarks_dir.exists() {
        eprintln!("BenchError: benchmarks directory not found");
        process::exit(1);
    }

    let mut cases = Vec::new();
    collect_tests(benchmarks_dir, &mut cases);
    cases.sort();

    if compare_python {
        let Some(python) = python_executable() else {
            eprintln!("BenchError: python executable not found");
            process::exit(1);
        };
        println!(
            "{:<20} {:>12} {:>12} {:>10}",
            "Benchmark", "Python", "DByte VM", "Ratio"
        );
        println!("{:-<60}", "");
        for path in cases {
            let name = path.file_stem().unwrap().to_string_lossy();
            let py_path = benchmarks_dir.join("python").join(format!("{}.py", name));
            if !py_path.exists() {
                eprintln!(
                    "BenchError: python benchmark not found: {}",
                    py_path.display()
                );
                process::exit(1);
            }
            let python_ms = run_python_benchmark(python, &py_path).unwrap_or_else(|e| {
                eprintln!("BenchError: {}", e);
                process::exit(1);
            });
            let dbyte_ms = run_benchmark(&path, Engine::Vm);
            let ratio = if dbyte_ms > 0.0 {
                python_ms / dbyte_ms
            } else {
                0.0
            };
            println!(
                "{:<20} {:>9.2} ms {:>9.2} ms {:>9.2}x",
                name, python_ms, dbyte_ms, ratio
            );
        }
        return;
    }

    println!("{:<20} {:<10} {:>12}", "Benchmark", "Engine", "Time (ms)");
    println!("{:-<45}", "");

    for path in cases {
        let name = path.file_stem().unwrap().to_string_lossy();
        let engines = if let Some(e) = engine_override {
            vec![e]
        } else {
            vec![Engine::Tree, Engine::Vm]
        };

        for engine in engines {
            println!(
                "{:<20} {:<10} {:>12.2} ms",
                name,
                engine.label(),
                run_benchmark(&path, engine)
            );
        }
    }
}

fn collect_tests(dir: &std::path::Path, cases: &mut Vec<std::path::PathBuf>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_tests(&path, cases);
        } else if path.extension().and_then(|s| s.to_str()) == Some("dby") {
            cases.push(path);
        }
    }
}

fn parse_engine(args: &[String]) -> Engine {
    let mut engine = Engine::Tree;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--engine" {
            engine = match iter.next().map(String::as_str) {
                Some("tree") => Engine::Tree,
                Some("vm") => Engine::Vm,
                _ => {
                    usage();
                    process::exit(1);
                }
            };
        }
    }
    engine
}

fn print_version() {
    println!("DByte {}", env!("CARGO_PKG_VERSION"));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "--version" => {
            print_version();
            process::exit(0);
        }
        "--help" | "-h" => {
            usage();
            process::exit(0);
        }
        "new" => {
            let name = args.get(2).map(|s| s.as_str()).unwrap_or_else(|| {
                usage();
                process::exit(1);
            });
            cmd_new(name);
        }
        "run" => {
            let mut type_check = true;
            let mut engine = Engine::Tree;
            let mut trace = false;
            let mut file: Option<PathBuf> = None;
            let mut script_args = Vec::new();
            let mut seen_file = false;
            for arg in &args[2..] {
                if seen_file {
                    script_args.push(arg.clone());
                    continue;
                }
                match arg.as_str() {
                    "--no-check" => type_check = false,
                    "--vm" => engine = Engine::Vm,
                    "--trace" => {
                        trace = true;
                        engine = Engine::Vm;
                    }
                    _ => {
                        file = Some(PathBuf::from(arg));
                        seen_file = true;
                    }
                }
            }
            if let Some(path) = file {
                cmd_run(&path, type_check, engine, trace, script_args);
            } else {
                cmd_run_project(type_check, engine, trace);
            }
        }
        "check" => {
            if let Some(path) = args.get(2) {
                cmd_check(Path::new(path));
            } else {
                cmd_check_project();
            }
        }
        "test" => cmd_test(parse_engine(&args[2..])),
        "repl" => {
            let mut no_rc = false;
            let mut rc_path = None;
            let mut iter = args.iter().skip(2);
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "--no-rc" => no_rc = true,
                    "--rc" => {
                        rc_path = iter.next().map(Path::new);
                        if rc_path.is_none() {
                            usage();
                            process::exit(1);
                        }
                    }
                    _ => {
                        usage();
                        process::exit(1);
                    }
                }
            }
            cmd_repl(no_rc, rc_path);
        }
        "shell" => {
            let mut no_rc = false;
            let mut rc_path = None;
            let mut iter = args.iter().skip(2);
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "--no-rc" => no_rc = true,
                    "--rc" => {
                        rc_path = iter.next().map(Path::new);
                        if rc_path.is_none() {
                            usage();
                            process::exit(1);
                        }
                    }
                    _ => {
                        usage();
                        process::exit(1);
                    }
                }
            }
            cmd_shell(no_rc, rc_path);
        }
        "bench" => {
            let mut engine = None;
            let mut compare_python = false;
            let mut iter = args.iter().skip(2);
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "--engine" => {
                        engine = match iter.next().map(String::as_str) {
                            Some("tree") => Some(Engine::Tree),
                            Some("vm") => Some(Engine::Vm),
                            _ => {
                                usage();
                                process::exit(1);
                            }
                        };
                    }
                    "--compare-python" => compare_python = true,
                    _ => {
                        usage();
                        process::exit(1);
                    }
                }
            }
            cmd_bench(engine, compare_python);
        }
        "disasm" => {
            let path = args.get(2).map(PathBuf::from).unwrap_or_else(|| {
                usage();
                process::exit(1);
            });
            cmd_disasm(&path);
        }
        "tokens" => {
            let path = args.get(2).map(PathBuf::from).unwrap_or_else(|| {
                usage();
                process::exit(1);
            });
            cmd_tokens(&path);
        }
        "ast" => {
            let path = args.get(2).map(PathBuf::from).unwrap_or_else(|| {
                usage();
                process::exit(1);
            });
            cmd_ast(&path);
        }
        _ => {
            usage();
            process::exit(1);
        }
    }
}
