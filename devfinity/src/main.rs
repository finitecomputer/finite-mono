use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use devfinity::workos_fixture::{
    FixturePaths, prepare_if_missing as prepare_workos_fixture_if_missing,
    serve as serve_workos_fixture,
};
use devfinity::{ProcessComposeMode, Stack, StackProfile, store_inference_key};

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
    /// Run an arbitrary command with isolated baseline test infrastructure.
    Run(RunArgs),
    /// Print the current devfinity process and service status.
    Status,
    /// Best-effort cleanup for orphaned devfinity processes.
    Cleanup,
    /// Cache an existing Finite Private key read from stdin for local chat.
    #[command(name = "inference-key")]
    InferenceKey,
    /// Run the local read-only WorkOS fixture used by the dev stack.
    #[command(name = "workos-fixture")]
    WorkosFixture {
        #[arg(long)]
        listen: std::net::SocketAddr,
        #[arg(long)]
        state_dir: PathBuf,
    },
}

#[derive(Debug, Args)]
struct UpArgs {
    /// Run process-compose without the TUI.
    #[arg(long)]
    headless: bool,

    /// Validate the generated process-compose config without starting services.
    #[arg(long)]
    dry_run: bool,

    /// Start only the portable host services, without building or launching an
    /// Agent Runtime. This is intended for focused service work and Linux CI.
    #[arg(long)]
    services_only: bool,

    /// Reset persistent state before starting. Allowed only with
    /// --services-only and intended for an isolated smoke-test state root.
    #[arg(long, requires = "services_only")]
    fresh: bool,

    /// Use the real WorkOS staging tenant configured in the repository-root
    /// .env instead of the deterministic local WorkOS fixture.
    #[arg(long)]
    workos_staging: bool,

    /// Command to run after the headless stack is ready. Pass after `--`.
    #[arg(last = true)]
    command: Vec<String>,
}

#[derive(Debug, Args)]
struct RunArgs {
    /// Command and arguments to run after baseline infrastructure is ready.
    #[arg(last = true, required = true)]
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

    match cli.command {
        Command::Up(args) => {
            let profile = if args.services_only {
                StackProfile::ServicesOnly
            } else {
                StackProfile::AppleSaas
            };
            let mut stack = Stack::new(cli.state_dir)?
                .with_profile(profile)
                .with_fresh_services_state(args.fresh);
            if args.workos_staging {
                stack = stack.with_workos_staging()?;
            }
            stack.prepare_host_environment(args.dry_run)?;
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
        Command::Run(args) => {
            std::fs::create_dir_all(&cli.state_dir).with_context(|| {
                format!(
                    "failed to create devfinity state root {}",
                    cli.state_dir.display()
                )
            })?;
            let state_dir = tempfile::Builder::new()
                .prefix("run-")
                .tempdir_in(&cli.state_dir)
                .context("failed to create isolated devfinity run state")?;
            let outcome = (|| {
                let port_reservation = TcpListener::bind(("127.0.0.1", 0))
                    .context("failed to reserve an isolated Postgres port")?;
                let postgres_port = port_reservation
                    .local_addr()
                    .context("failed to inspect the reserved Postgres port")?
                    .port();
                let stack = Stack::new(state_dir.path().to_path_buf())?
                    .with_profile(StackProfile::TestInfrastructure)
                    .with_postgres_port(postgres_port)?;
                stack.write_files()?;
                // Devfinity invocations reserve distinct kernel-assigned ports
                // while generating their configuration. Release immediately
                // before process-compose binds Postgres to the selected port.
                drop(port_reservation);
                stack.run_wrapped_command(&args.command)
            })();
            let cleanup = state_dir.close();

            match (outcome, cleanup) {
                (Ok(code), Ok(())) => Ok(code),
                (Ok(code), Err(error)) if code != ExitCode::SUCCESS => {
                    eprintln!(
                        "devfinity temporary-state cleanup after failed command also failed: {error}"
                    );
                    Ok(code)
                }
                (Ok(_), Err(error)) => {
                    Err(error).context("failed to remove devfinity temporary run state")
                }
                (Err(error), Ok(())) => Err(error),
                (Err(error), Err(cleanup_error)) => Err(error).context(format!(
                    "devfinity temporary-state cleanup also failed: {cleanup_error}"
                )),
            }
        }
        Command::Status => {
            let mut stack = Stack::new(cli.state_dir)?;
            let _ = stack.prepare_host_environment(true);
            stack.status()
        }
        Command::Cleanup => Stack::new(cli.state_dir)?.cleanup(),
        Command::InferenceKey => {
            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .context("failed to read Finite Private key from stdin")?;
            let path = store_inference_key(cli.state_dir, &input)?;
            println!("saved Finite Private key to {}", path.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::WorkosFixture { listen, state_dir } => {
            let paths = FixturePaths::new(state_dir);
            prepare_workos_fixture_if_missing(&paths, &format!("http://{listen}"))?;
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(serve_workos_fixture(listen, paths))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::Parser;

    #[test]
    fn run_command_preserves_child_argv_after_delimiter() {
        let cli = Cli::try_parse_from([
            "devfinity",
            "run",
            "--",
            "cargo",
            "test",
            "--workspace",
            "--locked",
        ])
        .unwrap();

        let Command::Run(args) = cli.command else {
            panic!("expected run command");
        };
        assert_eq!(args.command, ["cargo", "test", "--workspace", "--locked"]);
    }

    #[test]
    fn run_command_requires_a_child_command() {
        assert!(Cli::try_parse_from(["devfinity", "run"]).is_err());
    }
}
