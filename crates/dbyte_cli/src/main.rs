use std::fs;
use std::path::{Path, PathBuf};

use std::process;

use dbyte_interp::Interpreter;
use dbyte_lexer::Lexer;
use dbyte_parser::Parser;
use dbyte_project::{create_project, find_project_root, load_project, ProjectError};
use dbyte_typeck::TypeChecker;

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

fn cmd_run(path: &Path, type_check: bool) {
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

    let mut lexer = Lexer::new(&src);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(e) => {
            print_error("LexError", &e.msg, e.span, &path_label, &src);
            process::exit(1);
        }
    };

    let mut parser = Parser::new(tokens);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(e) => {
            print_error("ParseError", &e.msg, e.span, &path_label, &src);
            process::exit(1);
        }
    };

    if type_check {
        let mut checker = TypeChecker::with_entry_path(path.to_path_buf());
        if let Err(e) = checker.check_program(&program) {
            print_error("TypeError", &e.msg, e.span, &path_label, &src);
            process::exit(1);
        }
    }

    let mut interp = Interpreter::with_entry_path(path.to_path_buf());
    if let Err(e) = interp.run(&program) {
        print_error("RuntimeError", &e.msg, e.span, &path_label, &src);
        process::exit(1);
    }
}

fn cmd_run_project(type_check: bool) {
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
    cmd_run(&project.entry_path, type_check);
}

fn cmd_check(path: &Path) {
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

    let mut lexer = Lexer::new(&src);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(e) => {
            print_error("LexError", &e.msg, e.span, &path_label, &src);
            process::exit(1);
        }
    };

    let mut parser = Parser::new(tokens);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(e) => {
            print_error("ParseError", &e.msg, e.span, &path_label, &src);
            process::exit(1);
        }
    };

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

fn usage() {
    eprintln!(
        "\x1b[1mDByte v0.4\x1b[0m\n\
         Usage:\n\
         \x1b[1;33m  dbyte new   \x1b[0m<name>               create a DByte project\n\
         \x1b[1;33m  dbyte run   \x1b[0m[--no-check] [file]  run a file or project entry\n\
         \x1b[1;33m  dbyte check \x1b[0m[file]               type-check a file or project entry\n\
         \x1b[1;33m  dbyte test  \x1b[0m                     run all tests\n"
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

fn cmd_test() {
    let mut passed = 0;
    let mut failed = 0;

    println!("\x1b[1mRunning DByte Tests...\x1b[0m");

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
        let path_str = path.to_str().unwrap();
        let out_path = path.with_extension("out");
        let err_path = path.with_extension("err");
        if !out_path.exists() && !err_path.exists() {
            continue;
        }

        let output = process::Command::new(std::env::current_exe().unwrap())
            .arg("run")
            .arg(&path)
            .current_dir(&test_root)
            .output()
            .unwrap();

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

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "new" => {
            let name = args.get(2).map(|s| s.as_str()).unwrap_or_else(|| {
                usage();
                process::exit(1);
            });
            cmd_new(name);
        }
        "run" => {
            let mut type_check = true;
            let mut file: Option<PathBuf> = None;
            for arg in &args[2..] {
                if arg == "--no-check" {
                    type_check = false;
                } else {
                    file = Some(PathBuf::from(arg));
                }
            }
            if let Some(path) = file {
                cmd_run(&path, type_check);
            } else {
                cmd_run_project(type_check);
            }
        }
        "check" => {
            if let Some(path) = args.get(2) {
                cmd_check(Path::new(path));
            } else {
                cmd_check_project();
            }
        }
        "test" => {
            cmd_test();
        }
        _ => {
            usage();
            process::exit(1);
        }
    }
}
