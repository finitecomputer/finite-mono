fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let json_errors = finitechat_cli::json_errors_requested(&args);
    let mut stdout = std::io::stdout();
    if let Err(error) = finitechat_cli::run(args, &mut stdout) {
        if json_errors {
            eprintln!("{}", error.to_json());
        } else {
            eprintln!("{error}");
        }
        std::process::exit(error.exit_code());
    }
}
