use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use dbyte_compiler::Compiler;
use dbyte_interp::Interpreter;
use dbyte_lexer::Lexer;
use dbyte_parser::Parser;
use dbyte_project::{create_project, find_project_root, load_project, ProjectError};
use dbyte_typeck::TypeChecker;
use dbyte_vm::Vm;

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

fn cmd_run(path: &Path, type_check: bool, engine: Engine, trace: bool) {
    let path_label = path.display().to_string();
    let (src, program) = parse_file(path);
    if type_check {
        check_program(path, &src, &program);
    }

    match engine {
        Engine::Tree => {
            let mut interp = Interpreter::with_entry_path(path.to_path_buf());
            if let Err(e) = interp.run(&program) {
                print_error("RuntimeError", &e.msg, e.span, &path_label, &src);
                process::exit(1);
            }
        }
        Engine::Vm => {
            let chunk = compile_program(path, &src, &program);
            let mut vm = Vm::with_entry_path(path.to_path_buf());
            vm.set_trace(trace);
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
    cmd_run(&project.entry_path, type_check, engine, trace);
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
         dbyte run [--vm] <file>\n  \
         dbyte check <file>\n  \
         dbyte test [--engine tree|vm]\n  \
         dbyte bench [--engine tree|vm] [--compare-python]\n  \
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

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "--version" => {
            println!("DByte 1.3.1");
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
            for arg in &args[2..] {
                match arg.as_str() {
                    "--no-check" => type_check = false,
                    "--vm" => engine = Engine::Vm,
                    "--trace" => {
                        trace = true;
                        engine = Engine::Vm;
                    }
                    _ => file = Some(PathBuf::from(arg)),
                }
            }
            if let Some(path) = file {
                cmd_run(&path, type_check, engine, trace);
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
