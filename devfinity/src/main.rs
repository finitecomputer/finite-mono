use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use devfinity::workos_fixture::{
    FixturePaths, prepare_if_missing as prepare_workos_fixture_if_missing,
    serve as serve_workos_fixture,
};
use devfinity::{
    ProcessComposeMode, Stack, StackProfile, is_retryable_postgres_startup, store_inference_key,
};

const MAX_POSTGRES_PORT_ATTEMPTS: usize = 5;

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
    /// Pack, sign, host, and register the local skills bundle, then point the
    /// canary channel at it. Requires the devfinity stack to be running.
    #[command(name = "publish-skills")]
    PublishSkills(PublishSkillsArgs),
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

#[derive(Debug, Args)]
struct RunArgs {
    /// Command and arguments to run after baseline infrastructure is ready.
    #[arg(last = true, required = true)]
    command: Vec<String>,
}

#[derive(Debug, Args)]
struct PublishSkillsArgs {
    /// Skills tree to pack; defaults to the in-repo finite-skills/skills.
    #[arg(long)]
    source: Option<PathBuf>,

    /// Operator-readable label; defaults to a content-addressed
    /// devfinity-<digest prefix> label (no wall clock).
    #[arg(long)]
    version_label: Option<String>,
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
        Command::Run(args) => run_isolated_command(&cli.state_dir, &args.command),
        Command::Status => {
            let mut stack = Stack::new(cli.state_dir)?;
            let _ = stack.prepare_host_environment(true);
            stack.status()
        }
        Command::Cleanup => Stack::new(cli.state_dir)?.cleanup(),
        Command::PublishSkills(args) => {
            // Reuse the running stack's profile so the bundle reference URL is
            // container-reachable exactly the way the stack's other services
            // are; the stack exports DEVFINITY_PROFILE to wrapped commands.
            let profile = match std::env::var("DEVFINITY_PROFILE") {
                Ok(value) => StackProfile::from_name(value.trim())
                    .with_context(|| format!("DEVFINITY_PROFILE has unknown profile {value:?}"))?,
                Err(_) => StackProfile::AppleSaas,
            };
            if matches!(profile, StackProfile::TestInfrastructure) {
                anyhow::bail!("publish-skills is not available in the test-infrastructure profile");
            }
            let mut stack = Stack::new(cli.state_dir)?.with_profile(profile);
            stack.prepare_host_environment(true)?;
            stack.publish_skills(args.source.as_deref(), args.version_label.as_deref())?;
            Ok(ExitCode::SUCCESS)
        }
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

fn run_isolated_command(state_root: &Path, command: &[String]) -> anyhow::Result<ExitCode> {
    run_isolated_command_with(
        state_root,
        |state_dir, port_reservation| {
            let postgres_port = port_reservation
                .local_addr()
                .context("failed to inspect the reserved Postgres port")?
                .port();
            let stack = Stack::new(state_dir.to_path_buf())?
                .with_profile(StackProfile::TestInfrastructure)
                .with_postgres_port(postgres_port)?;
            stack.write_files()?;
            stack.run_wrapped_command_with_postgres_port_reservation(command, port_reservation)
        },
        is_retryable_postgres_startup,
    )
}

fn run_isolated_command_with<RunAttempt, Retryable>(
    state_root: &Path,
    mut run_attempt: RunAttempt,
    retryable: Retryable,
) -> anyhow::Result<ExitCode>
where
    RunAttempt: FnMut(&Path, TcpListener) -> anyhow::Result<ExitCode>,
    Retryable: Fn(&anyhow::Error) -> bool,
{
    std::fs::create_dir_all(state_root).with_context(|| {
        format!(
            "failed to create devfinity state root {}",
            state_root.display()
        )
    })?;

    for attempt in 1..=MAX_POSTGRES_PORT_ATTEMPTS {
        let state_dir = tempfile::Builder::new()
            .prefix("run-")
            .tempdir_in(state_root)
            .context("failed to create isolated devfinity run state")?;
        let outcome = (|| {
            let port_reservation = TcpListener::bind(("127.0.0.1", 0))
                .context("failed to reserve an isolated Postgres port")?;
            run_attempt(state_dir.path(), port_reservation)
        })();
        let retry_startup = outcome.as_ref().is_err_and(&retryable);
        let cleanup = state_dir.close();

        if retry_startup {
            let error = outcome.expect_err("startup retry requires a failed attempt");
            cleanup.context("failed to remove retryable devfinity run state")?;
            if attempt == MAX_POSTGRES_PORT_ATTEMPTS {
                return Err(error).context(format!(
                    "failed to start an owned Postgres instance after {attempt} isolated attempts"
                ));
            }
            eprintln!(
                "devfinity Postgres did not claim its reserved port; retrying with fresh state and a new port ({attempt}/{MAX_POSTGRES_PORT_ATTEMPTS}): {error}"
            );
            continue;
        }

        return match (outcome, cleanup) {
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
        };
    }

    unreachable!("bounded Postgres port attempts always return")
}

#[cfg(test)]
mod tests {
    use std::net::TcpStream;
    use std::process::ExitCode;

    use anyhow::anyhow;
    use clap::Parser;

    use super::{Cli, Command, run_isolated_command_with};

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

    #[test]
    fn isolated_command_retries_only_after_fresh_state_and_port_reservation() {
        let state_root = tempfile::tempdir().unwrap();
        let mut attempts = 0;
        let mut attempt_paths = Vec::new();

        let code = run_isolated_command_with(
            state_root.path(),
            |state_dir, port_reservation| {
                attempts += 1;
                attempt_paths.push(state_dir.to_path_buf());
                TcpStream::connect(port_reservation.local_addr().unwrap()).unwrap();
                if attempts == 1 {
                    Err(anyhow!("retry selected Postgres port"))
                } else {
                    Ok(ExitCode::SUCCESS)
                }
            },
            |error| error.to_string() == "retry selected Postgres port",
        )
        .unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(attempts, 2);
        assert_ne!(attempt_paths[0], attempt_paths[1]);
        assert!(attempt_paths.iter().all(|path| !path.exists()));
        assert_eq!(std::fs::read_dir(state_root.path()).unwrap().count(), 0);
    }

    #[test]
    fn isolated_command_does_not_retry_an_unrelated_failure() {
        let state_root = tempfile::tempdir().unwrap();
        let mut attempts = 0;

        let error = run_isolated_command_with(
            state_root.path(),
            |_state_dir, _port_reservation| {
                attempts += 1;
                Err(anyhow!("fatal infrastructure failure"))
            },
            |_error| false,
        )
        .unwrap_err();

        assert_eq!(attempts, 1);
        assert!(error.to_string().contains("fatal infrastructure failure"));
        assert_eq!(std::fs::read_dir(state_root.path()).unwrap().count(), 0);
    }
}
