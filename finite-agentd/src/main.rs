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
    #[command(name = "skills-sync")]
    SkillsSync {
        #[arg(long)]
        tarball_url: String,
        #[arg(long)]
        manifest_url: String,
        #[arg(long)]
        tarball_sha256: String,
    },
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("finite-agentd: {}", error.public_message());
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
                &tarball_url,
                &manifest_url,
                &tarball_sha256,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
    }
}
