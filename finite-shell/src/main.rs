//! `finite-shell`: PID 1 of the Agent Runtime container (`run`), plus the
//! `ctl` client that speaks the control socket.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::{Value, json};

use finite_shell::control::{ShellRuntime, ctl_roundtrip};
use finite_shell::{SHELL_VERSION, ShellError, ShellSettings};

#[derive(Debug, Parser)]
#[command(name = "finite-shell", version = SHELL_VERSION)]
#[command(about = "The fixed layer of a Finite Agent Runtime: boot, verify, flip, supervise")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run as PID 1: boot /data, supervise agentd, serve /healthz and the
    /// control socket. Configuration comes from FINITE_SHELL_* environment.
    Run,
    /// Send one verb to a running shell's control socket and print the JSON
    /// response. Exit code follows the response's "ok".
    Ctl {
        /// Control socket path (default <FINITE_SHELL_DATA>/shell/shell.sock).
        #[arg(long)]
        socket: Option<PathBuf>,
        #[command(subcommand)]
        verb: CtlVerb,
    },
}

#[derive(Debug, Subcommand)]
enum CtlVerb {
    /// Shell + generation + supervision status.
    Status,
    /// Stage a payload bundle from URLs or local files.
    Stage {
        #[arg(long, conflicts_with = "tarball_path", requires = "manifest_url")]
        tarball_url: Option<String>,
        #[arg(long, conflicts_with = "manifest_path", requires = "tarball_sha256")]
        manifest_url: Option<String>,
        #[arg(long, requires = "tarball_url")]
        tarball_sha256: Option<String>,
        #[arg(long, requires = "manifest_path")]
        tarball_path: Option<PathBuf>,
        #[arg(long, requires = "tarball_path")]
        manifest_path: Option<PathBuf>,
        /// Re-stage a version on the bad list (operator override).
        #[arg(long)]
        force: bool,
    },
    /// Flip to the staged generation (responds immediately; the flip runs in
    /// the shell).
    Flip {
        /// Poll status until the flip resolves; exit 0 on success, 1 on
        /// rollback.
        #[arg(long)]
        wait: bool,
    },
    /// Flip back to the previous generation.
    Rollback {
        #[arg(long)]
        wait: bool,
    },
    /// Set the release channel name (/data/shell/channel).
    SetChannel { channel: String },
}

fn main() {
    let args = Args::parse();
    let exit = match args.command {
        Command::Run => run(),
        Command::Ctl { socket, verb } => ctl(socket, verb),
    };
    match exit {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("finite-shell: {error}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<i32, ShellError> {
    let settings = ShellSettings::from_env()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let shell = ShellRuntime::boot(settings.clone()).await?;
        let socket = shell.bind_socket()?;
        let http = tokio::net::TcpListener::bind(&settings.http_addr)
            .await
            .map_err(|error| {
                ShellError::Config(format!("cannot bind {}: {error}", settings.http_addr))
            })?;
        tokio::spawn(shell.clone().serve_socket(socket));
        tokio::spawn(shell.clone().serve_http(http));

        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = sigterm.recv() => {}
            result = tokio::signal::ctrl_c() => { result?; }
        }
        shell.shutdown().await;
        Ok(0)
    })
}

fn ctl(socket: Option<PathBuf>, verb: CtlVerb) -> Result<i32, ShellError> {
    let socket = socket.unwrap_or_else(|| {
        let data = std::env::var(finite_shell::DATA_ENV).unwrap_or_else(|_| "/data".to_owned());
        PathBuf::from(data).join("shell").join("shell.sock")
    });
    let (request, wait) = match verb {
        CtlVerb::Status => (json!({ "verb": "status" }), false),
        CtlVerb::Stage {
            tarball_url,
            manifest_url,
            tarball_sha256,
            tarball_path,
            manifest_path,
            force,
        } => {
            let mut request = json!({ "verb": "stage" });
            let object = request.as_object_mut().expect("literal object");
            if let Some(value) = tarball_url {
                object.insert("tarballUrl".to_owned(), Value::from(value));
            }
            if let Some(value) = manifest_url {
                object.insert("manifestUrl".to_owned(), Value::from(value));
            }
            if let Some(value) = tarball_sha256 {
                object.insert("tarballSha256".to_owned(), Value::from(value));
            }
            if let Some(value) = tarball_path {
                object.insert(
                    "tarballPath".to_owned(),
                    Value::from(value.display().to_string()),
                );
            }
            if let Some(value) = manifest_path {
                object.insert(
                    "manifestPath".to_owned(),
                    Value::from(value.display().to_string()),
                );
            }
            if force {
                object.insert("force".to_owned(), Value::from(true));
            }
            (request, false)
        }
        CtlVerb::Flip { wait } => (json!({ "verb": "flip" }), wait),
        CtlVerb::Rollback { wait } => (json!({ "verb": "rollback" }), wait),
        CtlVerb::SetChannel { channel } => {
            (json!({ "verb": "set-channel", "channel": channel }), false)
        }
    };

    let response = ctl_roundtrip(&socket, &request)?;
    println!("{response}");
    let ok = response.get("ok") == Some(&Value::Bool(true));
    if !ok {
        return Ok(1);
    }
    if !wait {
        return Ok(0);
    }

    // Poll status until last_flip resolves.
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let status = ctl_roundtrip(&socket, &json!({ "verb": "status" }))?;
        let outcome = status
            .get("state")
            .and_then(|state| state.get("last_flip"))
            .and_then(|flip| flip.get("outcome"))
            .and_then(Value::as_str)
            .unwrap_or("in_progress")
            .to_owned();
        if outcome == "in_progress" {
            continue;
        }
        println!("{status}");
        return Ok(if outcome == "success" { 0 } else { 1 });
    }
}
