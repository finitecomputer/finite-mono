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
    /// Run the supervised Core-authorized Iroh transport.
    Iroh,
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
    if matches!(args.command, Command::Iroh) {
        return finite_agentd::run_iroh(finite_agentd::IrohConfig::from_env()?).await;
    }
    let config = DaemonConfig::from_env()?;
    match args.command {
        Command::Iroh => unreachable!(),
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
    }
}
