//! Experimental direct control listener. TLS terminates at a local reverse proxy.
//! No chat startup, identity, owner claim, or transport is used in this mode.
use crate::{
    AgentdError, ConfigManager, Ledger,
    connections::ConnectionManager,
    control::{ConnectionControl, ControlRequest},
    supervisor::{ProcessSpec, start_processes},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap, fs, net::SocketAddr, os::unix::fs::PermissionsExt, path::PathBuf,
    sync::Arc,
};
use tokio::sync::Semaphore;

pub struct ControlServerConfig {
    pub listen: SocketAddr,
    pub runtime_id: String,
    pub token_file: PathBuf,
    pub agent_home: PathBuf,
    pub hermes_home: PathBuf,
    /// A foreground Hermes launcher, independent of run_hermes_gateway.sh.
    pub hermes_command: PathBuf,
}

#[derive(Clone)]
struct App {
    runtime_id: String,
    token_file: PathBuf,
    control: ConnectionControl,
    ledger: Ledger,
    operation: Arc<Semaphore>,
}

pub async fn run_control_server(config: ControlServerConfig) -> Result<(), AgentdError> {
    // This spike must not accidentally expose plaintext credentials to a network.
    if !config.listen.ip().is_loopback() || config.runtime_id.is_empty() {
        return Err(AgentdError::Config(
            "control listener requires loopback and a runtime id".into(),
        ));
    }
    read_token(&config.token_file)?;
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    let ledger = Ledger::open(config.agent_home.join("agentd/agentd.sqlite3"))?;
    // Additive: old binaries never read this table. Existing chat rows, ownership,
    // config rollback history, and credentials retain their representation.
    Connection::open(ledger.path())?.execute_batch(
        "CREATE TABLE IF NOT EXISTS control_requests (
        request_id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, result_json TEXT
    );",
    )?;
    let supervisor = start_processes(
        ProcessSpec {
            name: "hermes",
            program: config.hermes_command,
            args: vec![],
            environment: BTreeMap::from([(
                "HERMES_HOME".into(),
                config.hermes_home.to_string_lossy().into_owned(),
            )]),
        },
        vec![],
    );
    let config_manager = ConfigManager::new(config.hermes_home.join("config.yaml"), ledger.clone());
    let control = ConnectionControl {
        connection_manager: ConnectionManager::new(
            config.agent_home,
            config.hermes_home.clone(),
            config_manager.clone(),
        ),
        config_manager,
        hermes_home: config.hermes_home,
        supervisor: supervisor.clone(),
    };
    let app = App {
        runtime_id: config.runtime_id,
        token_file: config.token_file,
        control,
        ledger,
        operation: Arc::new(Semaphore::new(1)),
    };
    let router = Router::new()
        .route("/v1/runtimes/{runtime}/connections", get(status))
        .route("/v1/runtimes/{runtime}/connections/commands", post(command))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(app);
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
        })
        .await;
    supervisor.shutdown().await;
    result.map_err(AgentdError::from)
}

fn read_token(path: &PathBuf) -> Result<Vec<u8>, AgentdError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 || metadata.len() > 256 {
        return Err(AgentdError::Unauthorized);
    }
    let token = fs::read_to_string(path)?;
    let token = token.trim();
    if token.len() < 32 || token.bytes().any(|b| b.is_ascii_whitespace()) {
        return Err(AgentdError::Unauthorized);
    }
    Ok(token.as_bytes().to_vec())
}

fn authorize(app: &App, runtime: &str, headers: &HeaderMap) -> Result<(), StatusCode> {
    // Read on every request: atomic replacement revokes the old credential without restart.
    let token = read_token(&app.token_file).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let supplied = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let expected = Sha256::digest(&token);
    let actual = Sha256::digest(supplied.as_bytes());
    let difference = expected
        .iter()
        .zip(actual.iter())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b));
    if difference != 0 || runtime != app.runtime_id {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

fn response(status: StatusCode, body: Value) -> Response {
    (status, [("cache-control", "no-store")], Json(body)).into_response()
}

async fn status(
    State(app): State<App>,
    Path(runtime): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &runtime, &headers) {
        return response(code, json!({"error":"unauthorized"}));
    }
    let Ok(_permit) = app.operation.clone().try_acquire_owned() else {
        return response(
            StatusCode::CONFLICT,
            json!({"error":"operation_in_progress"}),
        );
    };
    match app
        .control
        .execute(&ControlRequest {
            request_id: "inspect".into(),
            command: "agent.connections.status".into(),
            schema: "finite.agent.empty.request.v1".into(),
            body: serde_json::value::RawValue::from_string("{}".into()).expect("empty object"),
        })
        .await
    {
        Ok(value) => response(StatusCode::OK, value),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error":error.public_code()}),
        ),
    }
}

async fn command(
    State(app): State<App>,
    Path(runtime): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(code) = authorize(&app, &runtime, &headers) {
        return response(code, json!({"error":"unauthorized"}));
    }
    let Ok(mut request) = serde_json::from_slice::<ControlRequest>(&body) else {
        return response(StatusCode::BAD_REQUEST, json!({"error":"invalid_payload"}));
    };
    if request.request_id.is_empty()
        || request.request_id.len() > 128
        || !request
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return response(
            StatusCode::BAD_REQUEST,
            json!({"error":"invalid_request_id"}),
        );
    }
    if !matches!(
        request.command.as_str(),
        "agent.inference.apply"
            | "agent.simplex.connect"
            | "agent.simplex.reset"
            | "agent.simplex.approve_request"
            | "agent.telegram.connect"
            | "agent.telegram.approve"
            | "agent.telegram.home"
            | "agent.telegram.disconnect"
            | "agent.google.apply"
            | "agent.google.disconnect"
    ) {
        return response(
            StatusCode::BAD_REQUEST,
            json!({"error":"unsupported_command"}),
        );
    }
    // Separate config proposal IDs from legacy chat request IDs.
    request.request_id = format!("https-{}", request.request_id);
    let Ok(permit) = app.operation.clone().try_acquire_owned() else {
        return response(
            StatusCode::CONFLICT,
            json!({"error":"operation_in_progress"}),
        );
    };
    // Disconnecting the HTTP client must not cancel a mutation between its local
    // effect and its durable result. The permit stays with the operation.
    let task = tokio::spawn(async move {
        let _permit = permit;
        execute_recorded(&app, request).await
    });
    match task.await {
        Ok(Ok(value)) => response(StatusCode::OK, value),
        Ok(Err(error)) => response(StatusCode::CONFLICT, json!({"error":error.public_code()})),
        Err(_) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error":"internal_error"}),
        ),
    }
}

async fn execute_recorded(app: &App, request: ControlRequest) -> Result<Value, AgentdError> {
    let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&request)?));
    {
        let db = Connection::open(app.ledger.path())?;
        let prior: Option<(String, Option<String>)> = db
            .query_row(
                "SELECT fingerprint, result_json FROM control_requests WHERE request_id=?1",
                [&request.request_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((hash, result)) = prior {
            if hash != fingerprint {
                return Err(AgentdError::ConflictingRequestId(request.request_id));
            }
            // A crash between an external effect and persistence has an unknown
            // outcome. Do not automatically repeat reset, approval, or revocation.
            return result.map(|value| serde_json::from_str(&value).map_err(AgentdError::from))
                .unwrap_or_else(|| Err(AgentdError::ConfigConflict("Previous operation outcome is unknown; inspect current settings before another operation".into())));
        }
        db.execute(
            "INSERT INTO control_requests(request_id,fingerprint) VALUES (?1,?2)",
            params![request.request_id, fingerprint],
        )?;
    }
    let result = match app.control.execute(&request).await {
        Ok(value) => json!({"ok":true,"result":value}),
        Err(error) => json!({"ok":false,"error":error.public_code()}),
    };
    Connection::open(app.ledger.path())?.execute(
        "UPDATE control_requests SET result_json=?2 WHERE request_id=?1",
        params![request.request_id, serde_json::to_string(&result)?],
    )?;
    Ok(result)
}
