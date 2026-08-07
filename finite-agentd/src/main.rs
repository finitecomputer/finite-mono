use clap::{Parser, Subcommand};
use finite_agentd::{DaemonConfig, run_daemon};

#[derive(Debug, Parser)]
#[command(name = "finite-agentd")]
#[command(about = "Agent-owned Finite platform daemon")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the resident Finite Chat command bridge and supervise Hermes.
    Serve,
    /// Print the latest redacted local daemon status.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Fetch, verify, and apply one signed skills bundle through the exact
    /// `agent.skills.sync` command path (operator/test entry, no chat sender).
    /// Either name the exact bundle (`--tarball-url` + `--manifest-url` +
    /// `--tarball-sha256`) or resolve it through the signed service directory
    /// (`--channel`).
    #[command(name = "skills-sync")]
    SkillsSync {
        #[arg(long, conflicts_with = "channel", requires = "manifest_url")]
        tarball_url: Option<String>,
        #[arg(long, conflicts_with = "channel", requires = "tarball_sha256")]
        manifest_url: Option<String>,
        #[arg(long, conflicts_with = "channel", requires = "tarball_url")]
        tarball_sha256: Option<String>,
        /// Release channel (stable or canary) whose skills bundle head to
        /// resolve from the service directory (fresh fetch, verified).
        #[arg(long)]
        channel: Option<String>,
    },
    /// Print the shell's payload-generation status (forwarded over the
    /// finite-shell control socket).
    #[command(name = "payload-status")]
    PayloadStatus,
    /// Stage a payload generation through the exact `agent.payload.stage`
    /// command path. Either name the exact bundle (`--tarball-url` +
    /// `--manifest-url` + `--tarball-sha256`) or resolve the channel's
    /// `payload_bundle` head from the signed service directory (`--channel`).
    /// Flip and rollback have no agentd CLI verbs: agentd dies mid-flip, so
    /// use `finite-shell ctl flip --wait` / `ctl rollback --wait` directly.
    #[command(name = "payload-stage")]
    PayloadStage {
        #[arg(long, conflicts_with = "channel", requires = "manifest_url")]
        tarball_url: Option<String>,
        #[arg(long, conflicts_with = "channel", requires = "tarball_sha256")]
        manifest_url: Option<String>,
        #[arg(long, conflicts_with = "channel", requires = "tarball_url")]
        tarball_sha256: Option<String>,
        /// Release channel (stable or canary) whose payload bundle head to
        /// resolve from the service directory (fresh fetch, verified).
        #[arg(long)]
        channel: Option<String>,
        /// Operator override: re-stage a bad-listed version.
        #[arg(long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        // The CLI runs locally with exec access; unlike chat-delivered command
        // results, its stderr may carry the full error for diagnosability.
        eprintln!(
            "finite-agentd: {} [{}: {error:?}]",
            error.public_message(),
            error.public_code()
        );
        std::process::exit(1);
    }
}

async fn run() -> Result<(), finite_agentd::AgentdError> {
    let args = Args::parse();
    let config = DaemonConfig::from_env()?;
    match args.command {
        Command::Serve => run_daemon(config).await,
        Command::Status { json } => {
            let status = finite_agentd::read_status(&config.status_path())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!(
                    "finite-agentd {} ({} processes, {} authorized Principals)",
                    status.version,
                    status.processes.processes.len(),
                    status.authorized_principals
                );
            }
            Ok(())
        }
        Command::SkillsSync {
            tarball_url,
            manifest_url,
            tarball_sha256,
            channel,
        } => {
            let request_id = format!(
                "cli-skills-sync-{}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_millis())
                    .unwrap_or(0),
                std::process::id()
            );
            let result = finite_agentd::run_skills_sync_cli(
                &config,
                &request_id,
                finite_agentd::SkillsSyncRequest {
                    tarball_url,
                    manifest_url,
                    tarball_sha256,
                    channel,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::PayloadStatus => {
            let result = finite_agentd::run_payload_status_cli(&config).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::PayloadStage {
            tarball_url,
            manifest_url,
            tarball_sha256,
            channel,
            force,
        } => {
            let request_id = format!(
                "cli-payload-stage-{}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_millis())
                    .unwrap_or(0),
                std::process::id()
            );
            let result = finite_agentd::run_payload_stage_cli(
                &config,
                &request_id,
                finite_agentd::PayloadStageRequest {
                    tarball_url,
                    manifest_url,
                    tarball_sha256,
                    channel,
                    force,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
    }
}
