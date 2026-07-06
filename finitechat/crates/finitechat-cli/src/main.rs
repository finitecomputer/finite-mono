fn main() {
    let mut stdout = std::io::stdout();
    if let Err(error) = finitechat_cli::run(std::env::args().skip(1), &mut stdout) {
        eprintln!("{error}");
        std::process::exit(error.exit_code());
    }
}
