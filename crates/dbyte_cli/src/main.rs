use std::fs;

use std::process;

use dbyte_interp::Interpreter;
use dbyte_lexer::Lexer;
use dbyte_parser::Parser;
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

fn cmd_run(path: &str, type_check: bool) {
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("\x1b[1;31merror\x1b[0m: cannot read `{}`: {}", path, e);
            process::exit(1);
        }
    };

    let mut lexer = Lexer::new(&src);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(e) => {
            print_error("LexError", &e.msg, e.span, path, &src);
            process::exit(1);
        }
    };

    let mut parser = Parser::new(tokens);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(e) => {
            print_error("ParseError", &e.msg, e.span, path, &src);
            process::exit(1);
        }
    };

    if type_check {
        let mut checker = TypeChecker::new();
        if let Err(e) = checker.check_program(&program) {
            print_error("TypeError", &e.msg, e.span, path, &src);
            process::exit(1);
        }
    }

    let mut interp = Interpreter::new();
    if let Err(e) = interp.run(&program) {
        print_error("RuntimeError", &e.msg, e.span, path, &src);
        process::exit(1);
    }
}

fn cmd_check(path: &str) {
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("\x1b[1;31merror\x1b[0m: cannot read `{}`: {}", path, e);
            process::exit(1);
        }
    };

    let mut lexer = Lexer::new(&src);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(e) => { print_error("LexError", &e.msg, e.span, path, &src); process::exit(1); }
    };

    let mut parser = Parser::new(tokens);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(e) => { print_error("ParseError", &e.msg, e.span, path, &src); process::exit(1); }
    };

    let mut checker = TypeChecker::new();
    match checker.check_program(&program) {
        Ok(_) => println!("\x1b[1;32mok\x1b[0m: no type errors found in `{}`", path),
        Err(e) => { print_error("TypeError", &e.msg, e.span, path, &src); process::exit(1); }
    }
}

fn usage() {
    eprintln!(
        "\x1b[1mDByte v0.2\x1b[0m\n\
         Usage:\n\
         \x1b[1;33m  dbyte run   \x1b[0m<file.dby>           run a DByte program\n\
         \x1b[1;33m  dbyte check \x1b[0m<file.dby>           type-check only\n\
         \x1b[1;33m  dbyte test  \x1b[0m                     run all tests\n\
         \x1b[1;33m  dbyte run   \x1b[0m--no-check <file>    skip type-check\n"
    );
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' { in_escape = true; }
        else if in_escape && c == 'm' { in_escape = false; }
        else if !in_escape { out.push(c); }
    }
    out
}

fn cmd_test() {
    use std::path::Path;
    let mut passed = 0;
    let mut failed = 0;

    println!("\x1b[1mRunning DByte Tests...\x1b[0m");

    for dir in &["tests/smoke", "tests/errors"] {
        if !Path::new(dir).exists() { continue; }
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("dby") {
                continue;
            }

            let path_str = path.to_str().unwrap();
            let out_path = path.with_extension("out");
            let err_path = path.with_extension("err");

            let output = process::Command::new(std::env::current_exe().unwrap())
                .arg("run")
                .arg(&path)
                .output()
                .unwrap();

            let stdout = strip_ansi(&String::from_utf8_lossy(&output.stdout)).replace("\r\n", "\n").trim().to_string();
            let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr)).replace("\r\n", "\n").trim().to_string();

            let mut ok = true;
            let mut reason = String::new();

            if out_path.exists() {
                let expected = fs::read_to_string(&out_path).unwrap().replace("\r\n", "\n").trim().to_string();
                if stdout != expected {
                    ok = false;
                    reason = format!("Expected stdout:\n{}\nGot:\n{}", expected, stdout);
                }
            } else if err_path.exists() {
                let expected = fs::read_to_string(&err_path).unwrap().replace("\r\n", "\n").trim().to_string();
                if !stderr.contains(&expected) {
                    ok = false;
                    reason = format!("Expected stderr to contain:\n{}\nGot:\n{}", expected, stderr);
                }
            } else {
                if !output.status.success() {
                    ok = false;
                    reason = format!("Process failed with stderr:\n{}", stderr);
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
    }

    println!("\nTest result: {} passed, {} failed", passed, failed);
    if failed > 0 { process::exit(1); }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "run" => {
            let mut type_check = true;
            let mut file: Option<&str> = None;
            for arg in &args[2..] {
                if arg == "--no-check" {
                    type_check = false;
                } else {
                    file = Some(arg.as_str());
                }
            }
            let path = file.unwrap_or_else(|| { usage(); process::exit(1); });
            cmd_run(path, type_check);
        }
        "check" => {
            let path = args.get(2).map(|s| s.as_str()).unwrap_or_else(|| { usage(); process::exit(1); });
            cmd_check(path);
        }
        "test" => {
            cmd_test();
        }
        _ => { usage(); process::exit(1); }
    }
}
