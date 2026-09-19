use std::process::ExitCode;

fn main() -> ExitCode {
    let inventory_json = std::env::args().any(|arg| arg == "--existing-identity")
        && std::env::args().any(|arg| arg == "--json");
    let env = finite_brain_cli::CliEnvironment::from_process();
    match finite_brain_cli::run_from_process(env) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if inventory_json {
                use finite_brain_cli::CliError;
                let kind = match &error {
                    CliError::Identity(_)
                    | CliError::InvalidSigner(_)
                    | CliError::HttpStatus {
                        status: 401 | 403 | 404,
                        ..
                    } => "access",
                    _ => "request",
                };
                eprintln!(
                    "{}",
                    serde_json::json!({ "inventory_error_version": 1, "kind": kind })
                );
            } else {
                eprintln!("fbrain: {error}");
            }
            ExitCode::from(1)
        }
    }
}
