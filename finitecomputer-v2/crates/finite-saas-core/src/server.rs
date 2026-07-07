use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tokio::time::{Instant, sleep};
use tower_http::trace::TraceLayer;

use crate::api::router;
use crate::store::CoreStore;

#[derive(Debug, Clone)]
pub struct CoreServeOptions {
    pub bind: SocketAddr,
    pub database_url: String,
    pub api_token: String,
    pub postgres_connect_timeout: Duration,
    pub postgres_connect_retry_interval: Duration,
}

pub async fn serve_core(options: CoreServeOptions) -> Result<()> {
    let store = connect_postgres_with_retry(
        &options.database_url,
        options.postgres_connect_timeout,
        options.postgres_connect_retry_interval,
    )
    .await?;
    serve_core_with_store(options.bind, store, options.api_token).await
}

pub async fn serve_core_with_store(
    bind: SocketAddr,
    store: CoreStore,
    api_token: impl Into<String>,
) -> Result<()> {
    let app = router(store, api_token).layer(TraceLayer::new_for_http());
    let listener = TcpListener::bind(bind).await?;
    tracing::info!(addr = %bind, "finite-saas-core listening");
    axum::serve(listener, app).await?;
    Ok(())
}

pub async fn connect_postgres_with_retry(
    database_url: &str,
    timeout: Duration,
    retry_interval: Duration,
) -> Result<CoreStore> {
    let started = Instant::now();
    let mut attempts = 0usize;

    loop {
        attempts += 1;
        match connect_and_migrate_postgres(database_url).await {
            Ok(store) => return Ok(store),
            Err(error) => {
                if started.elapsed() >= timeout {
                    return Err(error).with_context(|| {
                        format!("Core Postgres was not ready after {attempts} attempts")
                    });
                }

                eprintln!("finite-saas-core waiting for Core Postgres: {error}");
                sleep(retry_interval).await;
            }
        }
    }
}

async fn connect_and_migrate_postgres(database_url: &str) -> Result<CoreStore> {
    let store = CoreStore::connect_postgres(database_url).await?;
    store.migrate().await?;
    Ok(store)
}
