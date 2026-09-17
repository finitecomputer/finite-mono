use super::*;
mod arguments;
pub(super) use arguments::*;
mod lifecycle;
pub(super) use lifecycle::*;
mod rollout;
pub(super) use rollout::*;
mod finite_private;
pub(super) use finite_private::*;
#[cfg(test)]
mod tests;

pub(crate) async fn serve() -> Result<()> {
    let auth = CoreAuth::from_env()?;
    let bind = env::var("FC_CORE_BIND").unwrap_or_else(|_| "127.0.0.1:4200".to_string());
    let addr: SocketAddr = bind.parse()?;

    let hosted_hermes_origins = match env::var("FC_CORE_HOSTED_HERMES_ORIGINS_JSON") {
        Ok(value) => HostedHermesOrigins::from_json(&value).map_err(anyhow::Error::msg)?,
        Err(env::VarError::NotPresent) => HostedHermesOrigins::default(),
        Err(env::VarError::NotUnicode(_)) => {
            bail!("FC_CORE_HOSTED_HERMES_ORIGINS_JSON must be UTF-8")
        }
    };
    let store = postgres_store_from_env(ImportMode::Commit).await?;
    let agent_creation_placement = optional_agent_creation_placement()?;
    let app = router_with_hosted_hermes_origins(
        store.clone(),
        auth,
        agent_creation_placement,
        hosted_hermes_origins.clone(),
    )
    .layer(TraceLayer::new_for_http());
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "finite-saas-core listening");
    if let Ok(bind) = env::var("FC_CORE_RUNTIME_BIND") {
        let runtime_listener = TcpListener::bind(bind.parse::<SocketAddr>()?).await?;
        let runtime_app = finite_saas_core::api::runtime_router(store, hosted_hermes_origins);
        tokio::try_join!(async { axum::serve(listener, app).await }, async {
            axum::serve(runtime_listener, runtime_app).await
        },)?;
    } else {
        axum::serve(listener, app).await?;
    }
    Ok(())
}

/// Commit-vs-dry-run switch for every admin CLI write. The name is a leftover
/// from the deleted existing-host import bridge (its reconcile command was the
/// first dry-runnable write); the mechanism is live and unrelated to imports.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ImportMode {
    Commit,
    DryRun,
}

impl ImportMode {
    pub(crate) fn from_dry_run(dry_run: bool) -> Self {
        if dry_run { Self::DryRun } else { Self::Commit }
    }
}

pub(crate) async fn postgres_store_from_env(mode: ImportMode) -> Result<CoreStore> {
    let database_url = required_env("FC_CORE_DATABASE_URL")?;
    let timeout = optional_duration_secs("FC_CORE_POSTGRES_CONNECT_TIMEOUT_SECS", 60)?;
    let retry_interval = optional_duration_millis("FC_CORE_POSTGRES_CONNECT_RETRY_MS", 1_000)?;
    let runtime_environment = optional_runtime_environment()?;
    let runtime_secret_references = optional_runtime_secret_references()?;
    postgres_store_with_retry(&database_url, timeout, retry_interval, mode)
        .await?
        .with_runtime_environment(runtime_environment)
        .and_then(|store| store.with_runtime_secret_references(runtime_secret_references))
        .map_err(Into::into)
}

pub(crate) async fn postgres_store_with_retry(
    database_url: &str,
    timeout: Duration,
    retry_interval: Duration,
    mode: ImportMode,
) -> Result<CoreStore> {
    let started = Instant::now();
    let mut attempts = 0usize;

    loop {
        attempts += 1;
        match connect_and_migrate_postgres(database_url, mode).await {
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

pub(crate) async fn connect_and_migrate_postgres(
    database_url: &str,
    mode: ImportMode,
) -> Result<CoreStore> {
    match mode {
        ImportMode::Commit => {
            let store = CoreStore::connect(database_url).await?;
            store.migrate().await?;
            Ok(store)
        }
        // A dry run previews row writes only. It must not run schema DDL,
        // which commits outside the rolled-back transaction.
        ImportMode::DryRun => Ok(CoreStore::connect_dry_run(database_url).await?),
    }
}

pub(crate) fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub(crate) fn required_env(name: &str) -> Result<String> {
    let value = env::var(name).with_context(|| format!("{name} is required"))?;
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(value)
}

pub(crate) fn optional_runtime_environment() -> Result<BTreeMap<String, String>> {
    let raw = match env::var("FC_CORE_RUNTIME_ENV_JSON") {
        Ok(raw) if !raw.trim().is_empty() => raw,
        Ok(_) | Err(env::VarError::NotPresent) => return Ok(BTreeMap::new()),
        Err(error) => return Err(error).context("failed to read FC_CORE_RUNTIME_ENV_JSON"),
    };
    serde_json::from_str(&raw)
        .context("FC_CORE_RUNTIME_ENV_JSON must be a JSON object of string values")
}

pub(crate) fn optional_runtime_secret_references() -> Result<Vec<String>> {
    let raw = match env::var("FC_CORE_RUNTIME_SECRET_REFERENCES_JSON") {
        Ok(raw) if !raw.trim().is_empty() => raw,
        Ok(_) | Err(env::VarError::NotPresent) => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).context("failed to read FC_CORE_RUNTIME_SECRET_REFERENCES_JSON");
        }
    };
    serde_json::from_str(&raw)
        .context("FC_CORE_RUNTIME_SECRET_REFERENCES_JSON must be a JSON array of strings")
}

pub(crate) fn optional_agent_creation_placement() -> Result<Option<RuntimePlacement>> {
    let raw = match env::var("FC_CORE_AGENT_CREATION_PLACEMENT_JSON") {
        Ok(raw) if !raw.trim().is_empty() => raw,
        Ok(_) | Err(env::VarError::NotPresent) => return Ok(None),
        Err(error) => {
            return Err(error).context("failed to read FC_CORE_AGENT_CREATION_PLACEMENT_JSON");
        }
    };
    serde_json::from_str(&raw)
        .map(Some)
        .context("FC_CORE_AGENT_CREATION_PLACEMENT_JSON must be a RuntimePlacement JSON object")
}

pub(crate) fn optional_duration_secs(name: &str, default: u64) -> Result<Duration> {
    Ok(Duration::from_secs(optional_positive_u64(name, default)?))
}

pub(crate) fn optional_duration_millis(name: &str, default: u64) -> Result<Duration> {
    Ok(Duration::from_millis(optional_positive_u64(name, default)?))
}

pub(crate) fn optional_positive_u64(name: &str, default: u64) -> Result<u64> {
    match env::var(name) {
        Ok(raw) => {
            let value = raw
                .trim()
                .parse::<u64>()
                .with_context(|| format!("{name} must be a positive integer"))?;
            if value == 0 {
                bail!("{name} must be greater than zero");
            }
            Ok(value)
        }
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error).with_context(|| format!("failed to read {name}")),
    }
}
