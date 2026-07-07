use std::process::ExitCode;

fn main() -> ExitCode {
    let env = finite_brain_cli::CliEnvironment::from_process();
    match finite_brain_cli::run_from_process(env) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fbrain: {error}");
            ExitCode::from(1)
        }
    }
}
