use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;

use finitechat_server::{HttpServerState, http_router};

mod push;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("serve") => serve(&args[1..]).await,
        Some("push-drain") => {
            let command = push::parse_push_drain_command(&args[1..])?;
            push::run_push_drain(command)?;
            Ok(())
        }
        Some("smoke") | None => {
            smoke();
            Ok(())
        }
        Some(command) => Err(format!(
            "unknown command '{command}'; expected 'serve [addr] [--sqlite PATH]', 'push-drain [options]', or 'smoke'"
        )
        .into()),
    }
}

async fn serve(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let options = ServeOptions::parse(args)?;
    let addr = options.addr.parse::<SocketAddr>()?;
    let state = match options.sqlite_path {
        Some(path) => {
            create_sqlite_parent_dir(&path)?;
            HttpServerState::from_sqlite_path(path)?
        }
        None => HttpServerState::default(),
    };
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("finitechat-server: listening on http://{addr}");
    axum::serve(listener, http_router(state)).await?;
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
}

impl ServeOptions {
    fn parse(args: &[String]) -> Result<Self, Box<dyn std::error::Error>> {
        let mut addr = None;
        let mut sqlite_path = None;
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::create_sqlite_parent_dir;

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
}
