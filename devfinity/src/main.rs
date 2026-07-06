use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use devfinity::{ProcessComposeMode, Stack};

#[derive(Debug, Parser)]
#[command(name = "devfinity")]
#[command(about = "Local Finite integration harness")]
struct Cli {
    /// Root directory for generated state, logs, env, and process-compose files.
    #[arg(
        long,
        env = "DEVFINITY_STATE_DIR",
        default_value = ".local-state/devfinity"
    )]
    state_dir: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Generate config and run the local stack through process-compose.
    Up(UpArgs),
    /// Best-effort cleanup for orphaned devfinity processes.
    Cleanup,
}

#[derive(Debug, Args)]
struct UpArgs {
    /// Run process-compose without the TUI.
    #[arg(long)]
    headless: bool,

    /// Validate the generated process-compose config without starting services.
    #[arg(long)]
    dry_run: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("devfinity: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();
    let stack = Stack::new(cli.state_dir)?;

    match cli.command {
        Command::Up(args) => {
            stack.write_files()?;
            stack.print_summary();
            let mode = if args.headless {
                ProcessComposeMode::Headless
            } else {
                ProcessComposeMode::Tui
            };
            stack.run_process_compose_up(mode, args.dry_run)
        }
        Command::Cleanup => stack.cleanup(),
    }
}
