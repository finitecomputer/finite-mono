use std::env;

use finitechat_server::{ChatServeOptions, serve_chat};

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
    serve_chat(ServeOptions::parse(args)?).await?;
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

struct ServeOptions;

impl ServeOptions {
    fn parse(args: &[String]) -> Result<ChatServeOptions, Box<dyn std::error::Error>> {
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
        Ok(ChatServeOptions {
            addr: addr.unwrap_or_else(|| "127.0.0.1:8787".to_owned()),
            sqlite_path,
        })
    }
}
