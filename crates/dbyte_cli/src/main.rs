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
        "\x1b[1mDByte v0.1\x1b[0m\n\
         Usage:\n\
         \x1b[1;33m  dbyte run   \x1b[0m<file.dby>           run a DByte program\n\
         \x1b[1;33m  dbyte check \x1b[0m<file.dby>           type-check only\n\
         \x1b[1;33m  dbyte run   \x1b[0m--no-check <file>    skip type-check\n"
    );
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
        _ => { usage(); process::exit(1); }
    }
}
