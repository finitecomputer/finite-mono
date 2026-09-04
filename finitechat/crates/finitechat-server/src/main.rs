use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;

use finitechat_server::{
    DEFAULT_RATE_LIMIT_PER_WINDOW, DEFAULT_RATE_LIMIT_WINDOW_SECONDS, HttpServerState, http_router,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("serve") => serve(&args[1..]).await,
        Some("snapshot") => {
            let options = ServeOptions::parse(&args[1..])?;
            let Some(path) = options.sqlite_path else {
                return Err("snapshot requires --sqlite PATH".into());
            };
            // Boot and write a fresh room-state checkpoint, so an operator
            // can compact a stopped server's boot tail.
            // without waiting for the op-interval trigger.
            let state = finitechat_server::HttpServerState::from_sqlite_path(&path)?;
            state
                .snapshot_now()
                .map_err(|error| format!("state snapshot failed: {error:?}"))?;
            println!("finitechat-server: state snapshot written to {path}");
            Ok(())
        }
        Some("rollback-check") => {
            let options = ServeOptions::parse(&args[1..])?;
            let Some(path) = options.sqlite_path else {
                return Err("rollback-check requires --sqlite PATH".into());
            };
            // Read-only verdict on whether restoring the pre-fold backup
            // over this database can still rewind no client (chat store
            // swap rollback window). Exit 0 only when it can.
            let check = finitechat_server::rollback_check(Path::new(&path))?;
            println!("{}", serde_json::to_string(&check)?);
            if check.rollback_allowed {
                Ok(())
            } else {
                eprintln!("finitechat-server: rollback refused: {}", check.reason);
                std::process::exit(1)
            }
        }
        Some("smoke") | None => {
            smoke();
            Ok(())
        }
        Some(command) => Err(format!(
            "unknown command '{command}'; expected 'serve [addr] [--sqlite PATH] [--public-url URL]', 'snapshot --sqlite PATH', 'rollback-check --sqlite PATH', or 'smoke'"
        )
        .into()),
    }
}

async fn serve(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let options = ServeOptions::parse(args)?;
    let addr = options.addr.parse::<SocketAddr>()?;
    let public_url = options
        .public_url
        .or_else(|| env::var("FINITECHAT_PUBLIC_URL").ok())
        .filter(|value| !value.trim().is_empty());
    let mut state = match options.sqlite_path {
        Some(path) => {
            create_sqlite_parent_dir(&path)?;
            HttpServerState::from_sqlite_path(path)?
        }
        None => {
            // In-memory servers have no durable state to roll back, but the
            // banner keeps the transitional window visible on every boot.
            finitechat_server::print_engine_rollout_banner();
            HttpServerState::new()
        }
    };
    if let Some(public_url) = public_url {
        state = state.with_public_url(public_url)?;
    }
    if options.require_signed_requests {
        state = state.with_require_signed_requests(true);
    }
    state = state.with_rate_limit(
        options.rate_limit_per_window,
        options.rate_limit_window_seconds,
    );
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("finitechat-server: listening on http://{addr}");
    axum::serve(
        listener,
        http_router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn create_sqlite_parent_dir(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(path);
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(parent)?;
    Ok(())
}

fn smoke() {
    let ids = finitechat_delivery::prove_http_delivery_core_orders_commit_then_message()
        .expect("HTTP delivery core smoke passes");
    println!(
        "finitechat-server: in-memory Finite Chat HTTP delivery core ready ({} smoke messages)",
        ids.len()
    );
}

#[derive(Debug)]
struct ServeOptions {
    addr: String,
    sqlite_path: Option<String>,
    public_url: Option<String>,
    require_signed_requests: bool,
    rate_limit_per_window: u32,
    rate_limit_window_seconds: u64,
}

impl ServeOptions {
    fn parse(args: &[String]) -> Result<Self, Box<dyn std::error::Error>> {
        let mut addr = None;
        let mut sqlite_path = None;
        let mut public_url = None;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--sqlite" => {
                    index += 1;
                    let Some(path) = args.get(index) else {
                        return Err("missing value for --sqlite".into());
                    };
                    sqlite_path = Some(path.clone());
                }
                "--public-url" => {
                    index += 1;
                    let Some(url) = args.get(index) else {
                        return Err("missing value for --public-url".into());
                    };
                    public_url = Some(url.clone());
                }
                value if value.starts_with("--") => {
                    return Err(format!("unknown serve option '{value}'").into());
                }
                value => {
                    if addr.replace(value.to_owned()).is_some() {
                        return Err("serve accepts at most one address".into());
                    }
                }
            }
            index += 1;
        }
        Ok(Self {
            addr: addr.unwrap_or_else(|| "127.0.0.1:8787".to_owned()),
            sqlite_path,
            public_url,
            // Mixed-version gate: old deployed clients send no NIP-98
            // Authorization header, so signed requests are opt-in until the
            // fleet upgrades.
            require_signed_requests: env::var("FINITECHAT_REQUIRE_SIGNED_REQUESTS")
                .map(|value| value.trim().eq_ignore_ascii_case("true") || value.trim() == "1")
                .unwrap_or(false),
            rate_limit_per_window: env_u32(
                "FINITECHAT_RATE_LIMIT_PER_WINDOW",
                DEFAULT_RATE_LIMIT_PER_WINDOW,
            ),
            rate_limit_window_seconds: env_u64(
                "FINITECHAT_RATE_LIMIT_WINDOW_SECONDS",
                DEFAULT_RATE_LIMIT_WINDOW_SECONDS,
            ),
        })
    }
}

fn env_u32(name: &str, default: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::{ServeOptions, create_sqlite_parent_dir};

    #[test]
    fn sqlite_parent_dir_is_created_before_open() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp
            .path()
            .join(".state")
            .join("nested")
            .join("finitechat.sqlite3");

        create_sqlite_parent_dir(db_path.to_str().expect("utf8 path")).expect("create parent dir");

        assert!(db_path.parent().expect("parent").is_dir());
    }

    #[test]
    fn serve_options_accept_canonical_public_url() {
        let options = ServeOptions::parse(&[
            "127.0.0.1:8788".to_owned(),
            "--sqlite".to_owned(),
            "/tmp/finitechat.sqlite3".to_owned(),
            "--public-url".to_owned(),
            "https://chat.finite.computer".to_owned(),
        ])
        .expect("serve options");

        assert_eq!(options.addr, "127.0.0.1:8788");
        assert_eq!(
            options.public_url.as_deref(),
            Some("https://chat.finite.computer")
        );
    }
}
