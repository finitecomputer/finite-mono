use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use finite_saas_core::api::router;
use finite_saas_core::auth::CoreAuth;
use finite_saas_core::store::CoreStore;
use finite_saas_core::{
    ApproveFinitePrivateGrantInput, ExistingHostProjectImport, FinitePrivateApiKey,
    FinitePrivateGrant, IssueFinitePrivateApiKeyInput, ReconcileExistingHostImportsOptions,
    ReconcileExistingHostImportsReport, ResetFinitePrivateUsageWindowInput,
    RevokeFinitePrivateApiKeyInput, RevokeFinitePrivateGrantInput, RotateFinitePrivateApiKeyInput,
    RuntimeArtifact, RuntimeArtifactKind, SourceHostRelayEndpoint, UpsertRuntimeArtifactInput,
    UpsertSourceHostRelayEndpointInput,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::env;
use std::io::{self, Read};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::{Instant, sleep};
use tower_http::trace::TraceLayer;

#[derive(Debug, Parser)]
#[command(name = "finite-saas-core")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the Core HTTP API.
    Serve,
    /// Reconcile existing-host Project import candidates from a typed manifest.
    #[command(name = "reconcile-imports")]
    ReconcileImports {
        /// Path to a manifest emitted by `finited core-import-manifest`, or `-` for stdin.
        #[arg(long)]
        manifest: PathBuf,
        /// Owner email allowed to be materialized by this import run. Repeatable.
        #[arg(long = "allow-owner")]
        allowlisted_owner_emails: Vec<String>,
        /// Optional RFC3339 timestamp for deterministic tests/operator dry runs.
        #[arg(long)]
        now: Option<String>,
        /// Validate and reconcile into an in-memory store without touching Postgres.
        #[arg(long)]
        dry_run: bool,
    },
    /// Add or update the relay endpoint for an existing source host.
    #[command(name = "source-host-relay-upsert")]
    SourceHostRelayUpsert {
        /// Source host id used in Core import keys, such as smoke, box1, or trf.
        #[arg(long)]
        source_host_id: String,
        /// Public relay base URL for that source host.
        #[arg(long)]
        url: String,
        /// Environment variable containing the host relay admin token.
        #[arg(long, default_value = "FC_RELAY_ADMIN_TOKEN")]
        admin_token_env: String,
        /// Optional RFC3339 timestamp for deterministic tests/operator dry runs.
        #[arg(long)]
        now: Option<String>,
        /// Validate and upsert into an in-memory store without touching Postgres.
        #[arg(long)]
        dry_run: bool,
    },
    /// Add or update a promoted runtime artifact record.
    #[command(name = "runtime-artifact-upsert")]
    RuntimeArtifactUpsert {
        /// Artifact id Core should store on launched Agent Runtimes.
        #[arg(long)]
        artifact_id: String,
        /// Artifact kind.
        #[arg(long, default_value = "oci_image")]
        kind: RuntimeArtifactKind,
        /// Snapshot name/path or OCI image reference.
        #[arg(long)]
        reference: String,
        /// Operator-readable artifact label.
        #[arg(long)]
        version_label: String,
        /// Runtime state schema version.
        #[arg(long, default_value = "runtime-state-v1")]
        state_schema_version: String,
        /// Base image used to create the artifact, such as python:3.11-trixie.
        #[arg(long)]
        base_image: Option<String>,
        /// Source git commit for the finitecomputer checkout.
        #[arg(long)]
        source_git_sha: Option<String>,
        /// finitec version or binary identifier.
        #[arg(long)]
        finitec_version: Option<String>,
        /// Hermes source revision/ref.
        #[arg(long)]
        hermes_source_ref: Option<String>,
        /// finite-platform plugin revision/ref.
        #[arg(long)]
        finite_platform_plugin_ref: Option<String>,
        /// Store the artifact as promoted and launchable.
        #[arg(long, default_value_t = true)]
        promoted: bool,
        /// Optional RFC3339 timestamp for deterministic tests/operator dry runs.
        #[arg(long)]
        now: Option<String>,
        /// Validate and upsert into an in-memory store without touching Postgres.
        #[arg(long)]
        dry_run: bool,
    },
    /// Approve a verified email for Finite Private without issuing a key.
    ///
    /// Break-glass path: prefer the dashboard admin page at /dashboard/admin, which calls the Core admin API.
    #[command(name = "finite-private-grant-approve")]
    FinitePrivateGrantApprove {
        /// Verified email receiving Finite Private access.
        #[arg(long)]
        email: String,
        /// Optional WorkOS user id. Omit for pre-product friend keys.
        #[arg(long)]
        workos_user_id: Option<String>,
        /// Optional limit profile id. Defaults to finite-private-generous.
        #[arg(long)]
        limit_profile_id: Option<String>,
        /// Optional RFC3339 timestamp for deterministic tests/operator dry runs.
        #[arg(long)]
        now: Option<String>,
        /// Validate and approve in an in-memory store without touching Postgres.
        #[arg(long)]
        dry_run: bool,
    },
    /// Approve a friend and issue a one-time Finite Private API key to hand out.
    ///
    /// Break-glass path: prefer the dashboard admin page at /dashboard/admin, which calls the Core admin API.
    #[command(name = "finite-private-friend-key-issue")]
    FinitePrivateFriendKeyIssue {
        /// Verified email receiving Finite Private access.
        #[arg(long)]
        email: String,
        /// Optional WorkOS user id. Omit until the friend signs into finite.computer.
        #[arg(long)]
        workos_user_id: Option<String>,
        /// Optional limit profile id. Defaults to finite-private-generous.
        #[arg(long)]
        limit_profile_id: Option<String>,
        /// Optional project id scope for the key.
        #[arg(long)]
        project_id: Option<String>,
        /// Optional runtime id scope for the key.
        #[arg(long)]
        agent_runtime_id: Option<String>,
        /// Environment variable containing caller-supplied key material. Omit to generate a key.
        #[arg(long)]
        raw_key_env: Option<String>,
        /// Optional RFC3339 timestamp for deterministic tests/operator dry runs.
        #[arg(long)]
        now: Option<String>,
        /// Validate and issue in an in-memory store without touching Postgres.
        #[arg(long)]
        dry_run: bool,
    },
    /// Issue a one-time Finite Private API key for an existing grant id.
    ///
    /// Break-glass path: prefer the dashboard admin page at /dashboard/admin, which calls the Core admin API.
    #[command(name = "finite-private-api-key-issue")]
    FinitePrivateApiKeyIssue {
        /// Existing Finite Private grant id.
        #[arg(long)]
        grant_id: String,
        /// Optional project id scope for the key.
        #[arg(long)]
        project_id: Option<String>,
        /// Optional runtime id scope for the key.
        #[arg(long)]
        agent_runtime_id: Option<String>,
        /// Environment variable containing caller-supplied key material. Omit to generate a key.
        #[arg(long)]
        raw_key_env: Option<String>,
        /// Optional RFC3339 timestamp for deterministic tests/operator dry runs.
        #[arg(long)]
        now: Option<String>,
        /// Validate and issue in an in-memory store without touching Postgres.
        #[arg(long)]
        dry_run: bool,
    },
    /// Rotate a Finite Private API key and print the new one-time raw key.
    ///
    /// Break-glass path: prefer the dashboard admin page at /dashboard/admin, which calls the Core admin API.
    #[command(name = "finite-private-api-key-rotate")]
    FinitePrivateApiKeyRotate {
        /// Existing Finite Private API key id to rotate.
        #[arg(long)]
        key_id: String,
        /// Environment variable containing caller-supplied key material. Omit to generate a key.
        #[arg(long)]
        raw_key_env: Option<String>,
        /// Optional RFC3339 timestamp for deterministic tests/operator dry runs.
        #[arg(long)]
        now: Option<String>,
        /// Validate and rotate in an in-memory store without touching Postgres.
        #[arg(long)]
        dry_run: bool,
    },
    /// Revoke a Finite Private API key.
    ///
    /// Break-glass path: prefer the dashboard admin page at /dashboard/admin, which calls the Core admin API.
    #[command(name = "finite-private-api-key-revoke")]
    FinitePrivateApiKeyRevoke {
        /// Existing Finite Private API key id to revoke.
        #[arg(long)]
        key_id: String,
        /// Optional RFC3339 timestamp for deterministic tests/operator dry runs.
        #[arg(long)]
        now: Option<String>,
        /// Validate and revoke in an in-memory store without touching Postgres.
        #[arg(long)]
        dry_run: bool,
    },
    /// Revoke a Finite Private grant and all keys under it.
    ///
    /// Break-glass path: prefer the dashboard admin page at /dashboard/admin, which calls the Core admin API.
    #[command(name = "finite-private-grant-revoke")]
    FinitePrivateGrantRevoke {
        /// Existing Finite Private grant id to revoke.
        #[arg(long)]
        grant_id: String,
        /// Optional RFC3339 timestamp for deterministic tests/operator dry runs.
        #[arg(long)]
        now: Option<String>,
        /// Validate and revoke in an in-memory store without touching Postgres.
        #[arg(long)]
        dry_run: bool,
    },
    /// Reset the current Finite Private Burst Window for a grant.
    ///
    /// Break-glass path: prefer the dashboard admin page at /dashboard/admin, which calls the Core admin API.
    #[command(name = "finite-private-window-reset")]
    FinitePrivateWindowReset {
        /// Existing Finite Private grant id to reset.
        #[arg(long)]
        grant_id: String,
        /// Optional RFC3339 timestamp for deterministic tests/operator dry runs.
        #[arg(long)]
        now: Option<String>,
        /// Validate and reset in an in-memory store without touching Postgres.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Deserialize)]
struct CoreImportManifestInput {
    source_host_id: String,
    records: Vec<ExistingHostProjectImport>,
}

/// Install a compact tracing subscriber writing to stderr, filtered by
/// `RUST_LOG` (default `info`). Kept minimal and standard: this is the crate's
/// first server-side logging, added so DB/store failures stop being invisible.
/// Ignores a duplicate-init error so CLI subcommands and tests stay safe.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .compact()
        .try_init();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let args = Args::parse();
    match args.command.unwrap_or(Command::Serve) {
        Command::Serve => serve().await,
        Command::ReconcileImports {
            manifest,
            allowlisted_owner_emails,
            now,
            dry_run,
        } => {
            let report = reconcile_imports_from_manifest(
                manifest,
                allowlisted_owner_emails,
                now,
                ImportMode::from_dry_run(dry_run),
            )
            .await?;
            print_json(&report)
        }
        Command::SourceHostRelayUpsert {
            source_host_id,
            url,
            admin_token_env,
            now,
            dry_run,
        } => {
            let endpoint = source_host_relay_upsert(
                source_host_id,
                url,
                required_env(&admin_token_env)?,
                now,
                ImportMode::from_dry_run(dry_run),
            )
            .await?;
            print_json(&redacted_source_host_relay_endpoint(endpoint))
        }
        Command::RuntimeArtifactUpsert {
            artifact_id,
            kind,
            reference,
            version_label,
            state_schema_version,
            base_image,
            source_git_sha,
            finitec_version,
            hermes_source_ref,
            finite_platform_plugin_ref,
            promoted,
            now,
            dry_run,
        } => {
            let artifact = runtime_artifact_upsert(
                UpsertRuntimeArtifactInput {
                    id: artifact_id,
                    kind,
                    reference,
                    version_label,
                    source_git_sha,
                    finitec_version,
                    hermes_source_ref,
                    finite_platform_plugin_ref,
                    state_schema_version,
                    base_image,
                    promoted,
                    now,
                },
                ImportMode::from_dry_run(dry_run),
            )
            .await?;
            print_json(&artifact)
        }
        Command::FinitePrivateGrantApprove {
            email,
            workos_user_id,
            limit_profile_id,
            now,
            dry_run,
        } => {
            let grant = finite_private_grant_approve(
                email,
                workos_user_id,
                limit_profile_id,
                now,
                ImportMode::from_dry_run(dry_run),
            )
            .await?;
            print_json(&grant)
        }
        Command::FinitePrivateFriendKeyIssue {
            email,
            workos_user_id,
            limit_profile_id,
            project_id,
            agent_runtime_id,
            raw_key_env,
            now,
            dry_run,
        } => {
            let output = finite_private_friend_key_issue(FinitePrivateFriendKeyIssueArgs {
                email,
                workos_user_id,
                limit_profile_id,
                project_id,
                agent_runtime_id,
                raw_key_env,
                now,
                mode: ImportMode::from_dry_run(dry_run),
            })
            .await?;
            print_json(&output)
        }
        Command::FinitePrivateApiKeyIssue {
            grant_id,
            project_id,
            agent_runtime_id,
            raw_key_env,
            now,
            dry_run,
        } => {
            let output = finite_private_api_key_issue(
                grant_id,
                project_id,
                agent_runtime_id,
                raw_key_env,
                now,
                ImportMode::from_dry_run(dry_run),
            )
            .await?;
            print_json(&output)
        }
        Command::FinitePrivateApiKeyRotate {
            key_id,
            raw_key_env,
            now,
            dry_run,
        } => {
            let output = finite_private_api_key_rotate(
                key_id,
                raw_key_env,
                now,
                ImportMode::from_dry_run(dry_run),
            )
            .await?;
            print_json(&output)
        }
        Command::FinitePrivateApiKeyRevoke {
            key_id,
            now,
            dry_run,
        } => {
            let key = finite_private_api_key_revoke(key_id, now, ImportMode::from_dry_run(dry_run))
                .await?;
            print_json(&key)
        }
        Command::FinitePrivateGrantRevoke {
            grant_id,
            now,
            dry_run,
        } => {
            let grant =
                finite_private_grant_revoke(grant_id, now, ImportMode::from_dry_run(dry_run))
                    .await?;
            print_json(&grant)
        }
        Command::FinitePrivateWindowReset {
            grant_id,
            now,
            dry_run,
        } => {
            let grant =
                finite_private_window_reset(grant_id, now, ImportMode::from_dry_run(dry_run))
                    .await?;
            print_json(&grant)
        }
    }
}

async fn serve() -> Result<()> {
    let auth = CoreAuth::from_env()?;
    let bind = env::var("FC_CORE_BIND").unwrap_or_else(|_| "127.0.0.1:4200".to_string());
    let addr: SocketAddr = bind.parse()?;

    let store = postgres_store_from_env().await?;
    let app = router(store, auth).layer(TraceLayer::new_for_http());
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "finite-saas-core listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn reconcile_imports_from_manifest(
    manifest_path: PathBuf,
    allowlisted_owner_emails: Vec<String>,
    now: Option<String>,
    mode: ImportMode,
) -> Result<ReconcileExistingHostImportsReport> {
    let records = read_import_manifest(&manifest_path)?;
    reconcile_imports(records, allowlisted_owner_emails, now, mode).await
}

async fn reconcile_imports(
    records: Vec<ExistingHostProjectImport>,
    allowlisted_owner_emails: Vec<String>,
    now: Option<String>,
    mode: ImportMode,
) -> Result<ReconcileExistingHostImportsReport> {
    if allowlisted_owner_emails.is_empty() {
        bail!("at least one --allow-owner email is required");
    }

    let store = match mode {
        ImportMode::DryRun => CoreStore::memory(),
        ImportMode::Commit => postgres_store_from_env().await?,
    };
    store
        .reconcile_existing_host_imports(
            records,
            ReconcileExistingHostImportsOptions {
                allowlisted_owner_emails,
                now,
            },
        )
        .await
        .map_err(Into::into)
}

async fn source_host_relay_upsert(
    source_host_id: String,
    url: String,
    admin_token: String,
    now: Option<String>,
    mode: ImportMode,
) -> Result<SourceHostRelayEndpoint> {
    let store = match mode {
        ImportMode::DryRun => CoreStore::memory(),
        ImportMode::Commit => postgres_store_from_env().await?,
    };
    store
        .upsert_source_host_relay_endpoint(UpsertSourceHostRelayEndpointInput {
            source_host_id,
            url,
            admin_token,
            now,
        })
        .await
        .map_err(Into::into)
}

async fn runtime_artifact_upsert(
    input: UpsertRuntimeArtifactInput,
    mode: ImportMode,
) -> Result<RuntimeArtifact> {
    let store = core_store_for_mode(mode).await?;
    store
        .upsert_runtime_artifact(input)
        .await
        .map_err(Into::into)
}

async fn finite_private_grant_approve(
    email: String,
    workos_user_id: Option<String>,
    limit_profile_id: Option<String>,
    now: Option<String>,
    mode: ImportMode,
) -> Result<FinitePrivateGrant> {
    let store = core_store_for_mode(mode).await?;
    store
        .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
            verified_email: email,
            workos_user_id,
            limit_profile_id,
            now,
        })
        .await
        .map_err(Into::into)
}

struct FinitePrivateFriendKeyIssueArgs {
    email: String,
    workos_user_id: Option<String>,
    limit_profile_id: Option<String>,
    project_id: Option<String>,
    agent_runtime_id: Option<String>,
    raw_key_env: Option<String>,
    now: Option<String>,
    mode: ImportMode,
}

async fn finite_private_friend_key_issue(
    input: FinitePrivateFriendKeyIssueArgs,
) -> Result<FinitePrivateIssuedKeyOutput> {
    let store = core_store_for_mode(input.mode).await?;
    let grant = store
        .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
            verified_email: input.email,
            workos_user_id: input.workos_user_id,
            limit_profile_id: input.limit_profile_id,
            now: input.now.clone(),
        })
        .await?;
    let raw_key = raw_key_from_env_or_generate(input.raw_key_env.as_deref())?;
    let api_key = store
        .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id: grant.id.clone(),
            raw_key: raw_key.value.clone(),
            project_id: input.project_id,
            agent_runtime_id: input.agent_runtime_id,
            now: input.now,
        })
        .await?;
    Ok(issued_key_output(Some(grant), api_key, raw_key))
}

async fn finite_private_api_key_issue(
    grant_id: String,
    project_id: Option<String>,
    agent_runtime_id: Option<String>,
    raw_key_env: Option<String>,
    now: Option<String>,
    mode: ImportMode,
) -> Result<FinitePrivateIssuedKeyOutput> {
    let store = core_store_for_mode(mode).await?;
    let raw_key = raw_key_from_env_or_generate(raw_key_env.as_deref())?;
    let api_key = store
        .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id,
            raw_key: raw_key.value.clone(),
            project_id,
            agent_runtime_id,
            now,
        })
        .await?;
    Ok(issued_key_output(None, api_key, raw_key))
}

async fn finite_private_api_key_rotate(
    key_id: String,
    raw_key_env: Option<String>,
    now: Option<String>,
    mode: ImportMode,
) -> Result<FinitePrivateIssuedKeyOutput> {
    let store = core_store_for_mode(mode).await?;
    let raw_key = raw_key_from_env_or_generate(raw_key_env.as_deref())?;
    let api_key = store
        .rotate_finite_private_api_key(RotateFinitePrivateApiKeyInput {
            key_id,
            raw_key: raw_key.value.clone(),
            now,
        })
        .await?;
    Ok(issued_key_output(None, api_key, raw_key))
}

async fn finite_private_api_key_revoke(
    key_id: String,
    now: Option<String>,
    mode: ImportMode,
) -> Result<FinitePrivateApiKey> {
    let store = core_store_for_mode(mode).await?;
    store
        .revoke_finite_private_api_key(RevokeFinitePrivateApiKeyInput { key_id, now })
        .await
        .map_err(Into::into)
}

async fn finite_private_grant_revoke(
    grant_id: String,
    now: Option<String>,
    mode: ImportMode,
) -> Result<FinitePrivateGrant> {
    let store = core_store_for_mode(mode).await?;
    store
        .revoke_finite_private_grant(RevokeFinitePrivateGrantInput { grant_id, now })
        .await
        .map_err(Into::into)
}

async fn finite_private_window_reset(
    grant_id: String,
    now: Option<String>,
    mode: ImportMode,
) -> Result<FinitePrivateGrant> {
    let store = core_store_for_mode(mode).await?;
    store
        .reset_finite_private_usage_window(ResetFinitePrivateUsageWindowInput { grant_id, now })
        .await
        .map_err(Into::into)
}

async fn core_store_for_mode(mode: ImportMode) -> Result<CoreStore> {
    match mode {
        ImportMode::DryRun => Ok(CoreStore::memory()),
        ImportMode::Commit => postgres_store_from_env().await,
    }
}

#[derive(Debug, serde::Serialize)]
struct FinitePrivateIssuedKeyOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    grant: Option<FinitePrivateGrant>,
    api_key: FinitePrivateApiKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_api_key: Option<String>,
    raw_api_key_generated: bool,
    raw_api_key_note: &'static str,
}

struct RawKeyMaterial {
    value: String,
    generated: bool,
}

fn issued_key_output(
    grant: Option<FinitePrivateGrant>,
    api_key: FinitePrivateApiKey,
    raw_key: RawKeyMaterial,
) -> FinitePrivateIssuedKeyOutput {
    FinitePrivateIssuedKeyOutput {
        grant,
        api_key,
        raw_api_key: raw_key.generated.then_some(raw_key.value),
        raw_api_key_generated: raw_key.generated,
        raw_api_key_note: if raw_key.generated {
            "Raw key is shown once. Store it in a secret manager before sending it."
        } else {
            "Raw key was read from raw_key_env and is not echoed."
        },
    }
}

fn raw_key_from_env_or_generate(raw_key_env: Option<&str>) -> Result<RawKeyMaterial> {
    if let Some(env_name) = raw_key_env {
        return Ok(RawKeyMaterial {
            value: required_env(env_name)?,
            generated: false,
        });
    }
    Ok(RawKeyMaterial {
        value: generate_finite_private_api_key()?,
        generated: true,
    })
}

fn generate_finite_private_api_key() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).context("failed to generate Finite Private API key")?;
    let mut key = String::with_capacity("fpk_live_".len() + bytes.len() * 2);
    key.push_str("fpk_live_");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut key, "{byte:02x}")?;
    }
    Ok(key)
}

#[derive(Debug, serde::Serialize)]
struct RedactedSourceHostRelayEndpoint {
    source_host_id: String,
    url: String,
    admin_token_configured: bool,
    created_at: String,
    updated_at: String,
}

fn redacted_source_host_relay_endpoint(
    endpoint: SourceHostRelayEndpoint,
) -> RedactedSourceHostRelayEndpoint {
    RedactedSourceHostRelayEndpoint {
        source_host_id: endpoint.source_host_id,
        url: endpoint.url,
        admin_token_configured: !endpoint.admin_token.is_empty(),
        created_at: endpoint.created_at,
        updated_at: endpoint.updated_at,
    }
}

#[derive(Debug, Clone, Copy)]
enum ImportMode {
    Commit,
    DryRun,
}

impl ImportMode {
    fn from_dry_run(dry_run: bool) -> Self {
        if dry_run { Self::DryRun } else { Self::Commit }
    }
}

async fn postgres_store_from_env() -> Result<CoreStore> {
    let database_url = required_env("FC_CORE_DATABASE_URL")?;
    let timeout = optional_duration_secs("FC_CORE_POSTGRES_CONNECT_TIMEOUT_SECS", 60)?;
    let retry_interval = optional_duration_millis("FC_CORE_POSTGRES_CONNECT_RETRY_MS", 1_000)?;
    let runtime_environment = optional_runtime_environment()?;
    postgres_store_with_retry(&database_url, timeout, retry_interval)
        .await?
        .with_runtime_environment(runtime_environment)
        .map_err(Into::into)
}

async fn postgres_store_with_retry(
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

fn read_import_manifest(path: &PathBuf) -> Result<Vec<ExistingHostProjectImport>> {
    let raw = if path == std::path::Path::new("-") {
        let mut raw = String::new();
        io::stdin()
            .read_to_string(&mut raw)
            .context("failed to read import manifest from stdin")?;
        raw
    } else {
        std::fs::read_to_string(path)
            .with_context(|| format!("failed to read import manifest {}", path.display()))?
    };

    parse_import_manifest(&raw)
}

fn parse_import_manifest(raw: &str) -> Result<Vec<ExistingHostProjectImport>> {
    let manifest: CoreImportManifestInput =
        serde_json::from_str(raw).context("failed to parse import manifest JSON")?;
    let manifest_source_host_id = manifest.source_host_id.trim().to_lowercase();
    if manifest_source_host_id.is_empty() {
        bail!("manifest source_host_id is required");
    }

    for record in &manifest.records {
        let record_source_host_id = record.source_host_id.trim().to_lowercase();
        if record_source_host_id != manifest_source_host_id {
            bail!(
                "manifest source_host_id {manifest_source_host_id} does not match record source_host_id {record_source_host_id} for {}",
                record.source_machine_id
            );
        }
    }

    Ok(manifest.records)
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn required_env(name: &str) -> Result<String> {
    let value = env::var(name).with_context(|| format!("{name} is required"))?;
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(value)
}

fn optional_runtime_environment() -> Result<BTreeMap<String, String>> {
    let raw = match env::var("FC_CORE_RUNTIME_ENV_JSON") {
        Ok(raw) if !raw.trim().is_empty() => raw,
        Ok(_) | Err(env::VarError::NotPresent) => return Ok(BTreeMap::new()),
        Err(error) => return Err(error).context("failed to read FC_CORE_RUNTIME_ENV_JSON"),
    };
    serde_json::from_str(&raw)
        .context("FC_CORE_RUNTIME_ENV_JSON must be a JSON object of string values")
}

fn optional_duration_secs(name: &str, default: u64) -> Result<Duration> {
    Ok(Duration::from_secs(optional_positive_u64(name, default)?))
}

fn optional_duration_millis(name: &str, default: u64) -> Result<Duration> {
    Ok(Duration::from_millis(optional_positive_u64(name, default)?))
}

fn optional_positive_u64(name: &str, default: u64) -> Result<u64> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use finite_saas_core::RuntimeSummaryStatus;

    #[test]
    fn parses_control_plane_import_manifest_shape() {
        let records = parse_import_manifest(
            r#"{
              "source_host_id": "box1",
              "records": [
                {
                  "source_host_id": "box1",
                  "source_machine_id": "paul-finite-2",
                  "owner_email": "paul@finite.vip",
                  "display_name": "Paul 2",
                  "hostname": "paul2-opencode.finite.vip",
                  "runtime_host": "box1",
                  "runtime_status": "unknown",
                  "active_inference_profile": "main",
                  "hermes_available": null,
                  "published_app_urls": ["https://demo.finite.vip"],
                  "known_external_channel_participants": [],
                  "admin_visible_to_emails": ["paul@finite.vip"]
                }
              ]
            }"#,
        )
        .unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].source_host_id, "box1");
        assert_eq!(records[0].source_machine_id, "paul-finite-2");
        assert_eq!(records[0].runtime_status, RuntimeSummaryStatus::Unknown);
    }

    #[test]
    fn rejects_manifest_records_from_another_host() {
        let error = parse_import_manifest(
            r#"{
              "source_host_id": "box1",
              "records": [
                {
                  "source_host_id": "trf",
                  "source_machine_id": "grant",
                  "owner_email": "rene@example.com",
                  "display_name": "Grant",
                  "hostname": null,
                  "runtime_host": "trf",
                  "runtime_status": "unknown",
                  "active_inference_profile": null,
                  "hermes_available": null,
                  "published_app_urls": [],
                  "known_external_channel_participants": [],
                  "admin_visible_to_emails": []
                }
              ]
            }"#,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("does not match record source_host_id"));
    }

    #[tokio::test]
    async fn dry_run_reconcile_uses_allowlist_without_postgres() {
        let report = reconcile_imports(
            vec![ExistingHostProjectImport {
                source_host_id: "smoke".to_string(),
                source_machine_id: "paul-smoke".to_string(),
                owner_email: Some("paul@finite.vip".to_string()),
                display_name: "Paul Smoke".to_string(),
                hostname: Some("paul-smoke-opencode.smoke.finite.computer".to_string()),
                runtime_host: Some("smoke".to_string()),
                runtime_status: RuntimeSummaryStatus::Unknown,
                active_inference_profile: Some("main".to_string()),
                hermes_available: None,
                published_app_urls: vec!["https://demo.smoke.finite.computer".to_string()],
                known_external_channel_participants: Vec::new(),
                admin_visible_to_emails: vec!["admin@finite.vip".to_string()],
            }],
            vec!["paul@finite.vip".to_string()],
            Some("2026-05-25T12:00:00Z".to_string()),
            ImportMode::DryRun,
        )
        .await
        .unwrap();

        assert_eq!(report.created_candidates.len(), 1);
        assert!(report.updated_candidates.is_empty());
        assert!(report.skipped_records.is_empty());
    }

    #[tokio::test]
    async fn import_reconcile_requires_explicit_allowlist_before_store_access() {
        let error = reconcile_imports(
            Vec::new(),
            Vec::new(),
            Some("2026-05-25T12:00:00Z".to_string()),
            ImportMode::DryRun,
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("at least one --allow-owner email is required"));
    }

    #[tokio::test]
    async fn dry_run_source_host_relay_upsert_validates_and_redacts() {
        let endpoint = source_host_relay_upsert(
            "Smoke".to_string(),
            "https://relay.smoke.finite.computer/".to_string(),
            "smoke-token".to_string(),
            Some("2026-05-25T12:00:00Z".to_string()),
            ImportMode::DryRun,
        )
        .await
        .unwrap();

        assert_eq!(endpoint.source_host_id, "smoke");
        assert_eq!(endpoint.url, "https://relay.smoke.finite.computer");
        assert_eq!(endpoint.admin_token, "smoke-token");
        assert!(redacted_source_host_relay_endpoint(endpoint).admin_token_configured);
    }

    #[tokio::test]
    async fn dry_run_finite_private_friend_key_issue_generates_one_time_key() {
        let output = finite_private_friend_key_issue(FinitePrivateFriendKeyIssueArgs {
            email: "friend@finite.vip".to_string(),
            workos_user_id: None,
            limit_profile_id: None,
            project_id: None,
            agent_runtime_id: None,
            raw_key_env: None,
            now: Some("2026-05-26T12:00:00Z".to_string()),
            mode: ImportMode::DryRun,
        })
        .await
        .unwrap();

        let raw_key = output.raw_api_key.as_deref().unwrap();
        assert!(raw_key.starts_with("fpk_live_"));
        assert!(output.raw_api_key_generated);
        assert_eq!(output.grant.as_ref().unwrap().status.as_str(), "active");
        assert_eq!(output.api_key.status.as_str(), "active");
        assert_ne!(output.api_key.key_hash, raw_key);
    }

    #[test]
    fn generated_finite_private_api_keys_are_prefixed_and_unique() {
        let first = generate_finite_private_api_key().unwrap();
        let second = generate_finite_private_api_key().unwrap();

        assert!(first.starts_with("fpk_live_"));
        assert!(second.starts_with("fpk_live_"));
        assert_ne!(first, second);
        assert_eq!(first.len(), "fpk_live_".len() + 64);
    }
}
