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
    /// Fetch Core-managed public flags, then start the canonical runtime.
    #[cfg(unix)]
    Bootstrap,
    /// Run the optional native Hermes backend with trusted launch credentials.
    HostedHermes,
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
    if matches!(args.command, Command::HostedHermes) {
        return finite_agentd::run_hosted_hermes();
    }
    #[cfg(unix)]
    if matches!(args.command, Command::Bootstrap) {
        return finite_agentd::bootstrap().await;
    }
    let config = DaemonConfig::from_env()?;
    match args.command {
        Command::HostedHermes => unreachable!(),
        #[cfg(unix)]
        Command::Bootstrap => unreachable!(),
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
