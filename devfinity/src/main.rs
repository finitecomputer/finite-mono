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
    /// Print the current devfinity process, container, and service status.
    Status,
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

    /// Command to run after the headless stack is ready. Pass after `--`.
    #[arg(last = true)]
    command: Vec<String>,
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
            if !args.command.is_empty() {
                if args.dry_run {
                    anyhow::bail!("`devfinity up -- <command>` cannot be combined with --dry-run");
                }
                return stack.run_wrapped_command(&args.command);
            }
            let mode = if args.headless {
                ProcessComposeMode::Headless
            } else {
                ProcessComposeMode::Tui
            };
            stack.run_process_compose_up(mode, args.dry_run)
        }
        Command::Status => stack.status(),
        Command::Cleanup => stack.cleanup(),
    }
}
