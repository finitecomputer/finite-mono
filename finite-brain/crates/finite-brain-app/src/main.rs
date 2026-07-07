use std::error::Error;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    if std::env::args()
        .nth(1)
        .is_some_and(|arg| matches!(arg.as_str(), "version" | "--version" | "-V"))
    {
        println!("finite-brain {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let address = std::env::var("FINITE_BRAIN_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3015".to_owned())
        .parse::<SocketAddr>()?;
    let public_base_url = std::env::var("FINITE_BRAIN_PUBLIC_BASE_URL")
        .unwrap_or_else(|_| format!("http://{address}"));
    let database_path =
        std::env::var("FINITE_BRAIN_DB").unwrap_or_else(|_| "finite-brain.sqlite3".to_owned());
    let listener = tokio::net::TcpListener::bind(address).await?;

    println!("FiniteBrain smoke server listening on http://{address}");

    let router = finite_brain_server::router_with_sqlite_path(database_path, public_base_url)?;
    axum::serve(listener, router).await?;

    Ok(())
}
