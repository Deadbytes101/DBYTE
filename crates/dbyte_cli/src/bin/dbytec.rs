use dbyte_native::cli::run_args;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(error) = run_args(&args) {
        eprintln!("{error}");
        std::process::exit(error.exit_code());
    }
}
