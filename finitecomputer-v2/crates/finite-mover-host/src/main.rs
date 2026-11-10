use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use finite_mover_host::command;
use finite_mover_host::error::HostError;

const USAGE: &str = "finite-mover-host --work-root <dir> (--ssh | <operation>...)";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("finite-mover-host: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}

fn run() -> Result<(), HostError> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--work-root") {
        return Err(HostError::Usage(USAGE.into()));
    }
    let work_root = PathBuf::from(args.next().ok_or_else(|| HostError::Usage(USAGE.into()))?);
    let rest: Vec<String> = args.collect();
    let operation = match rest.as_slice() {
        [ssh] if ssh == "--ssh" => {
            let original = std::env::var("SSH_ORIGINAL_COMMAND")
                .map_err(|_| HostError::Usage("no SSH_ORIGINAL_COMMAND".into()))?;
            command::parse_ssh(&original)?
        }
        words => command::parse(words.iter().map(String::as_str))?,
    };
    command::run(&work_root, operation, io::stdout().lock())
}
