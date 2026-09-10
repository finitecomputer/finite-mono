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
    /// Experimental Connections listener. Run instead of Serve on scratch state.
    ControlServe {
        #[arg(long, default_value = "127.0.0.1:37634")]
        listen: std::net::SocketAddr,
        #[arg(long)]
        runtime_id: String,
        #[arg(long)]
        token_file: std::path::PathBuf,
        #[arg(long)]
        agent_home: std::path::PathBuf,
        #[arg(long)]
        hermes_home: std::path::PathBuf,
        #[arg(long)]
        hermes_command: std::path::PathBuf,
    },
    /// Print the latest redacted local daemon status.
    Status {
        #[arg(long)]
        json: bool,
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
    match args.command {
        Command::Serve => run_daemon(DaemonConfig::from_env()?).await,
        Command::ControlServe {
            listen,
            runtime_id,
            token_file,
            agent_home,
            hermes_home,
            hermes_command,
        } => {
            finite_agentd::run_control_server(finite_agentd::ControlServerConfig {
                listen,
                runtime_id,
                token_file,
                agent_home,
                hermes_home,
                hermes_command,
            })
            .await
        }
        Command::Status { json } => {
            let status = finite_agentd::read_status(&DaemonConfig::from_env()?.status_path())?;
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
    }
}
