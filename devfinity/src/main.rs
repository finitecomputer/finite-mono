use std::io::Read;
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

    /// Launch the canonical Agent Runtime with the local Docker Runner. This
    /// portable full-product profile is intended for disposable CI acceptance.
    #[arg(long, conflicts_with = "services_only")]
    docker_runtime: bool,

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
            } else if args.docker_runtime {
                StackProfile::DockerSaas
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
