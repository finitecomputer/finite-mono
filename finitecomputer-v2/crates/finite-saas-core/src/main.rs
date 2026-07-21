use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use finite_saas_core::api::router_with_agent_creation_placement;
use finite_saas_core::auth::CoreAuth;
use finite_saas_core::store::CoreStore;
use finite_saas_core::{
    AdminArchiveUnrecoverableRuntimeInput, AdminRuntimeOverview, AdminRuntimeUpgradeExactInput,
    ApproveFinitePrivateGrantInput, CoreResult, ExistingHostProjectImport, FinitePrivateApiKey,
    FinitePrivateGrant, IssueFinitePrivateApiKeyInput, ReconcileExistingHostImportsOptions,
    ReconcileExistingHostImportsReport, ResetFinitePrivateUsageWindowInput,
    RevokeFinitePrivateApiKeyInput, RevokeFinitePrivateGrantInput, RotateFinitePrivateApiKeyInput,
    RuntimeArtifact, RuntimeArtifactKind, RuntimeControlRequest, RuntimeControlRequestStatus,
    RuntimePlacement, RuntimeSummaryStatus, SourceHostRelayEndpoint, UpsertRuntimeArtifactInput,
    UpsertSourceHostRelayEndpointInput,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
        /// Exact image implements recover-known-good-chat receiver semantics.
        #[arg(long, default_value_t = false)]
        recover_known_good_chat: bool,
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
    /// Roll active, upgrade-capable Agent Runtimes to one explicit artifact.
    #[command(name = "runtime-artifact-rollout")]
    RuntimeArtifactRollout(RuntimeArtifactRolloutCliArgs),
    /// Archive a legacy Runtime only after exact binding and absence attestations.
    #[command(name = "runtime-archive-unrecoverable")]
    RuntimeArchiveUnrecoverable(RuntimeArchiveUnrecoverableCliArgs),
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
        /// Optional limit profile id. Defaults to finite-private-generous-v2.
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
        /// Optional limit profile id. Defaults to finite-private-generous-v2.
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

#[derive(Debug, clap::Args)]
#[command(group(
    clap::ArgGroup::new("rollout_scope")
        .required(true)
        .multiple(false)
        .args(["project_ids", "all"])
))]
struct RuntimeArtifactRolloutCliArgs {
    /// Exact promoted runtime artifact id to deploy.
    #[arg(long)]
    artifact_id: String,
    /// Exact source host whose active Runtimes may appear in the plan.
    #[arg(long)]
    source_host_id: String,
    /// Verified email recorded as the admin actor.
    #[arg(long)]
    admin_email: String,
    /// WorkOS user id recorded as the admin actor.
    #[arg(long)]
    admin_workos_user_id: String,
    /// Project to upgrade. Repeat for a deterministic selected-project rollout.
    #[arg(long = "project-id")]
    project_ids: Vec<String>,
    /// Roll every eligible active Runtime. Requires an explicit canary project.
    #[arg(long, requires = "canary_project_id")]
    all: bool,
    /// Project upgraded first during an --all rollout.
    #[arg(long, requires = "all")]
    canary_project_id: Option<String>,
    /// Exact Runtime id from a prior plan. Requires one explicit project.
    #[arg(long, requires_all = ["expected_source_machine_id"])]
    expected_agent_runtime_id: Option<String>,
    /// Exact source machine id from a prior plan. Requires one explicit project.
    #[arg(long, requires_all = ["expected_agent_runtime_id"])]
    expected_source_machine_id: Option<String>,
    /// Print the deterministic plan without enqueueing any lifecycle request.
    #[arg(long)]
    plan_only: bool,
    /// Maximum seconds to wait for each exact lifecycle request to finish.
    #[arg(
        long = "wait-timeout-seconds",
        default_value_t = 900,
        value_parser = clap::value_parser!(u64).range(1..=3600)
    )]
    wait_timeout_seconds: u64,
}

#[derive(Debug, clap::Args)]
struct RuntimeArchiveUnrecoverableCliArgs {
    #[arg(long)]
    project_id: String,
    #[arg(long)]
    expected_agent_runtime_id: String,
    #[arg(long)]
    expected_source_host_id: String,
    #[arg(long)]
    expected_source_machine_id: String,
    #[arg(long)]
    expected_owner_email: String,
    #[arg(long)]
    admin_email: String,
    #[arg(long)]
    admin_workos_user_id: String,
    #[arg(long, required = true, action = clap::ArgAction::SetTrue)]
    confirm_compute_absent: bool,
    #[arg(long, required = true, action = clap::ArgAction::SetTrue)]
    confirm_durable_state_absent: bool,
    #[arg(long, required = true, action = clap::ArgAction::SetTrue)]
    confirm_owner_acknowledged_unrecoverable: bool,
    #[arg(long)]
    now: Option<String>,
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
            recover_known_good_chat,
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
                    recover_known_good_chat,
                    promoted,
                    now,
                },
                ImportMode::from_dry_run(dry_run),
            )
            .await?;
            print_json(&artifact)
        }
        Command::RuntimeArtifactRollout(args) => runtime_artifact_rollout_command(args).await,
        Command::RuntimeArchiveUnrecoverable(args) => {
            let receipt = runtime_archive_unrecoverable_command(args).await?;
            print_json(&receipt)
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
    let agent_creation_placement = optional_agent_creation_placement()?;
    let app = router_with_agent_creation_placement(store, auth, agent_creation_placement)
        .layer(TraceLayer::new_for_http());
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeArtifactRolloutScope {
    Projects(Vec<String>),
    All { canary_project_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeArtifactRolloutExpectedBinding {
    agent_runtime_id: String,
    source_machine_id: String,
}

#[derive(Debug)]
struct RuntimeArtifactRolloutInput {
    target_artifact_id: String,
    source_host_id: String,
    admin_email: String,
    admin_workos_user_id: String,
    scope: RuntimeArtifactRolloutScope,
    expected_binding: Option<RuntimeArtifactRolloutExpectedBinding>,
    plan_only: bool,
    wait_timeout: Duration,
}

impl TryFrom<RuntimeArtifactRolloutCliArgs> for RuntimeArtifactRolloutInput {
    type Error = anyhow::Error;

    fn try_from(args: RuntimeArtifactRolloutCliArgs) -> Result<Self> {
        let target_artifact_id = required_cli_value(args.artifact_id, "--artifact-id")?;
        let source_host_id = required_cli_value(args.source_host_id, "--source-host-id")?;
        let admin_email = required_cli_value(args.admin_email, "--admin-email")?;
        let admin_workos_user_id =
            required_cli_value(args.admin_workos_user_id, "--admin-workos-user-id")?;
        let mut project_ids = Vec::new();
        let mut seen_project_ids = BTreeSet::new();
        for project_id in args.project_ids {
            let project_id = required_cli_value(project_id, "--project-id")?;
            if seen_project_ids.insert(project_id.clone()) {
                project_ids.push(project_id);
            }
        }
        let explicit_project_count = project_ids.len();
        let scope = match (args.all, project_ids.is_empty(), args.canary_project_id) {
            (false, false, None) => RuntimeArtifactRolloutScope::Projects(project_ids),
            (true, true, Some(canary_project_id)) => RuntimeArtifactRolloutScope::All {
                canary_project_id: required_cli_value(canary_project_id, "--canary-project-id")?,
            },
            _ => bail!(
                "choose exactly one rollout scope: repeat --project-id, or use --all with --canary-project-id"
            ),
        };
        let expected_binding = match (
            args.expected_agent_runtime_id,
            args.expected_source_machine_id,
        ) {
            (None, None) => None,
            (Some(agent_runtime_id), Some(source_machine_id)) => {
                if args.all || explicit_project_count != 1 {
                    bail!(
                        "exact Runtime binding requires exactly one --project-id and cannot be used with --all"
                    );
                }
                Some(RuntimeArtifactRolloutExpectedBinding {
                    agent_runtime_id: required_cli_value(
                        agent_runtime_id,
                        "--expected-agent-runtime-id",
                    )?,
                    source_machine_id: required_cli_value(
                        source_machine_id,
                        "--expected-source-machine-id",
                    )?,
                })
            }
            _ => bail!(
                "--expected-agent-runtime-id and --expected-source-machine-id must be provided together"
            ),
        };
        Ok(Self {
            target_artifact_id,
            source_host_id,
            admin_email,
            admin_workos_user_id,
            scope,
            expected_binding,
            plan_only: args.plan_only,
            wait_timeout: Duration::from_secs(args.wait_timeout_seconds),
        })
    }
}

fn required_cli_value(value: String, flag: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{flag} must not be empty");
    }
    Ok(value.to_string())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RuntimeArtifactRolloutPlanEntry {
    project_id: String,
    agent_runtime_id: String,
    project_display_name: String,
    source_host_id: String,
    source_machine_id: String,
    target_artifact_id: String,
}

impl RuntimeArtifactRolloutPlanEntry {
    fn from_overview(overview: &AdminRuntimeOverview, target_artifact_id: &str) -> Self {
        Self {
            project_id: overview.project_id.clone(),
            agent_runtime_id: overview.agent_runtime_id.clone(),
            project_display_name: overview.project_display_name.clone(),
            source_host_id: overview.source_host_id.clone(),
            source_machine_id: overview.source_machine_id.clone(),
            target_artifact_id: target_artifact_id.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RuntimeArtifactRolloutSkippedEntry {
    project_id: String,
    agent_runtime_id: Option<String>,
    project_display_name: Option<String>,
    source_host_id: Option<String>,
    source_machine_id: Option<String>,
    reason: String,
}

impl RuntimeArtifactRolloutSkippedEntry {
    fn for_overview(overview: &AdminRuntimeOverview, reason: &str) -> Self {
        Self {
            project_id: overview.project_id.clone(),
            agent_runtime_id: Some(overview.agent_runtime_id.clone()),
            project_display_name: Some(overview.project_display_name.clone()),
            source_host_id: Some(overview.source_host_id.clone()),
            source_machine_id: Some(overview.source_machine_id.clone()),
            reason: reason.to_string(),
        }
    }

    fn missing_project(project_id: String) -> Self {
        Self {
            project_id,
            agent_runtime_id: None,
            project_display_name: None,
            source_host_id: None,
            source_machine_id: None,
            reason: "project_not_found".to_string(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RuntimeArtifactRolloutPlan {
    planned: Vec<RuntimeArtifactRolloutPlanEntry>,
    skipped: Vec<RuntimeArtifactRolloutSkippedEntry>,
    execution_blocked_reason: Option<String>,
}

fn consider_runtime_for_rollout(
    overview: &AdminRuntimeOverview,
    source_host_id: &str,
    target_artifact_id: &str,
    planned: &mut Vec<RuntimeArtifactRolloutPlanEntry>,
    skipped: &mut Vec<RuntimeArtifactRolloutSkippedEntry>,
) {
    let skip_reason = if overview.source_host_id != source_host_id {
        Some("wrong_source_host")
    } else if !overview.runtime_link_active {
        Some("inactive_runtime_link")
    } else if overview.runtime_artifact_id.as_deref() == Some(target_artifact_id) {
        Some("already_on_target_artifact")
    } else if !overview
        .runtime_capabilities
        .is_some_and(|capabilities| capabilities.runtime_upgrade)
    {
        Some("runtime_upgrade_not_supported")
    } else {
        None
    };
    if let Some(reason) = skip_reason {
        skipped.push(RuntimeArtifactRolloutSkippedEntry::for_overview(
            overview, reason,
        ));
    } else {
        planned.push(RuntimeArtifactRolloutPlanEntry::from_overview(
            overview,
            target_artifact_id,
        ));
    }
}

fn plan_runtime_artifact_rollout(
    mut overviews: Vec<AdminRuntimeOverview>,
    scope: &RuntimeArtifactRolloutScope,
    source_host_id: &str,
    target_artifact_id: &str,
) -> RuntimeArtifactRolloutPlan {
    overviews.sort_by(|left, right| {
        left.project_id
            .cmp(&right.project_id)
            .then_with(|| left.agent_runtime_id.cmp(&right.agent_runtime_id))
    });

    let mut planned = Vec::new();
    let mut skipped = Vec::new();

    match scope {
        RuntimeArtifactRolloutScope::Projects(project_ids) => {
            for project_id in project_ids {
                let matching = overviews
                    .iter()
                    .filter(|overview| overview.project_id == *project_id)
                    .collect::<Vec<_>>();
                if matching.is_empty() {
                    skipped.push(RuntimeArtifactRolloutSkippedEntry::missing_project(
                        project_id.clone(),
                    ));
                } else {
                    let matching_on_host = matching
                        .iter()
                        .copied()
                        .filter(|overview| overview.source_host_id == source_host_id)
                        .collect::<Vec<_>>();
                    if matching_on_host.is_empty() {
                        skipped.push(RuntimeArtifactRolloutSkippedEntry::for_overview(
                            matching[0],
                            "wrong_source_host",
                        ));
                    } else {
                        for overview in matching_on_host {
                            consider_runtime_for_rollout(
                                overview,
                                source_host_id,
                                target_artifact_id,
                                &mut planned,
                                &mut skipped,
                            );
                        }
                    }
                }
            }
        }
        RuntimeArtifactRolloutScope::All { .. } => {
            for overview in overviews
                .iter()
                .filter(|overview| overview.source_host_id == source_host_id)
            {
                consider_runtime_for_rollout(
                    overview,
                    source_host_id,
                    target_artifact_id,
                    &mut planned,
                    &mut skipped,
                );
            }
        }
    }

    let execution_blocked_reason = match scope {
        RuntimeArtifactRolloutScope::Projects(_) => {
            let unavailable_projects = skipped
                .iter()
                .filter(|entry| {
                    entry.reason == "project_not_found" || entry.reason == "wrong_source_host"
                })
                .map(|entry| entry.project_id.as_str())
                .collect::<Vec<_>>();
            (!unavailable_projects.is_empty()).then(|| {
                format!(
                    "explicitly requested project(s) were not found on source host {source_host_id}: {}",
                    unavailable_projects.join(", ")
                )
            })
        }
        RuntimeArtifactRolloutScope::All { canary_project_id } => {
            planned.sort_by(|left, right| {
                (left.project_id != *canary_project_id)
                    .cmp(&(right.project_id != *canary_project_id))
                    .then_with(|| left.project_id.cmp(&right.project_id))
                    .then_with(|| left.agent_runtime_id.cmp(&right.agent_runtime_id))
            });
            let canary_is_planned = planned
                .iter()
                .any(|entry| entry.project_id == *canary_project_id);
            let canary_is_already_ready = overviews.iter().any(|overview| {
                overview.project_id == *canary_project_id
                    && overview.source_host_id == source_host_id
                    && overview.runtime_link_active
                    && overview.runtime_artifact_id.as_deref() == Some(target_artifact_id)
                    && overview.runtime_status == RuntimeSummaryStatus::Online
            });
            (!canary_is_planned && !canary_is_already_ready).then(|| {
                format!(
                    "canary project {canary_project_id} has no eligible active Runtime and is not already online on the target artifact"
                )
            })
        }
    };
    skipped.sort_by(|left, right| {
        left.project_id
            .cmp(&right.project_id)
            .then_with(|| left.agent_runtime_id.cmp(&right.agent_runtime_id))
            .then_with(|| left.reason.cmp(&right.reason))
    });

    RuntimeArtifactRolloutPlan {
        planned,
        skipped,
        execution_blocked_reason,
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RuntimeArtifactRolloutOutcomeStatus {
    Succeeded,
    EnqueueFailed,
    RequestFailed,
    TimedOut,
    PollFailed,
    PostconditionFailed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RuntimeArtifactRolloutOutcome {
    project_id: String,
    agent_runtime_id: String,
    request_id: Option<String>,
    status: RuntimeArtifactRolloutOutcomeStatus,
    detail: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct RuntimeArtifactRolloutReport {
    target_artifact_id: String,
    source_host_id: String,
    plan_only: bool,
    planned: Vec<RuntimeArtifactRolloutPlanEntry>,
    skipped: Vec<RuntimeArtifactRolloutSkippedEntry>,
    outcomes: Vec<RuntimeArtifactRolloutOutcome>,
    halted: bool,
    halted_reason: Option<String>,
}

trait RuntimeArtifactRolloutStore {
    async fn rollout_admin_runtime_overviews(&self) -> CoreResult<Vec<AdminRuntimeOverview>>;
    async fn rollout_admin_request_runtime_upgrade(
        &self,
        input: AdminRuntimeUpgradeExactInput,
    ) -> CoreResult<RuntimeControlRequest>;
    async fn rollout_runtime_control_request(
        &self,
        request_id: &str,
    ) -> CoreResult<RuntimeControlRequest>;
}

impl RuntimeArtifactRolloutStore for CoreStore {
    async fn rollout_admin_runtime_overviews(&self) -> CoreResult<Vec<AdminRuntimeOverview>> {
        self.admin_runtime_overviews().await
    }

    async fn rollout_admin_request_runtime_upgrade(
        &self,
        input: AdminRuntimeUpgradeExactInput,
    ) -> CoreResult<RuntimeControlRequest> {
        self.admin_request_runtime_upgrade_exact(input).await
    }

    async fn rollout_runtime_control_request(
        &self,
        request_id: &str,
    ) -> CoreResult<RuntimeControlRequest> {
        self.runtime_control_request(request_id).await
    }
}

enum RuntimeArtifactRolloutWaitResult {
    Terminal(RuntimeControlRequest),
    TimedOut(RuntimeControlRequest),
}

async fn wait_for_runtime_artifact_upgrade<S: RuntimeArtifactRolloutStore>(
    store: &S,
    request: RuntimeControlRequest,
    wait_timeout: Duration,
    poll_interval: Duration,
) -> CoreResult<RuntimeArtifactRolloutWaitResult> {
    let deadline = Instant::now() + wait_timeout;
    let mut last = request;
    loop {
        last = store.rollout_runtime_control_request(&last.id).await?;
        if matches!(
            last.status,
            RuntimeControlRequestStatus::Succeeded | RuntimeControlRequestStatus::Failed
        ) {
            return Ok(RuntimeArtifactRolloutWaitResult::Terminal(last));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(RuntimeArtifactRolloutWaitResult::TimedOut(last));
        }
        sleep(poll_interval.min(deadline.saturating_duration_since(now))).await;
    }
}

fn rollout_outcome(
    entry: &RuntimeArtifactRolloutPlanEntry,
    request_id: Option<String>,
    status: RuntimeArtifactRolloutOutcomeStatus,
    detail: Option<String>,
) -> RuntimeArtifactRolloutOutcome {
    RuntimeArtifactRolloutOutcome {
        project_id: entry.project_id.clone(),
        agent_runtime_id: entry.agent_runtime_id.clone(),
        request_id,
        status,
        detail,
    }
}

async fn runtime_artifact_rollout<S: RuntimeArtifactRolloutStore>(
    store: &S,
    input: RuntimeArtifactRolloutInput,
    poll_interval: Duration,
) -> Result<RuntimeArtifactRolloutReport> {
    let plan = plan_runtime_artifact_rollout(
        store.rollout_admin_runtime_overviews().await?,
        &input.scope,
        &input.source_host_id,
        &input.target_artifact_id,
    );
    let exact_binding_mismatch = input.expected_binding.as_ref().and_then(|expected| {
        (plan.planned.len() != 1
            || plan.planned[0].agent_runtime_id != expected.agent_runtime_id
            || plan.planned[0].source_machine_id != expected.source_machine_id)
            .then(|| {
                "active Runtime no longer matches the exact preflighted rollout binding".to_string()
            })
    });
    let mut report = RuntimeArtifactRolloutReport {
        target_artifact_id: input.target_artifact_id.clone(),
        source_host_id: input.source_host_id.clone(),
        plan_only: input.plan_only,
        planned: plan.planned.clone(),
        skipped: plan.skipped,
        outcomes: Vec::new(),
        halted: plan.execution_blocked_reason.is_some() || exact_binding_mismatch.is_some(),
        halted_reason: plan.execution_blocked_reason.or(exact_binding_mismatch),
    };
    if input.plan_only || report.halted {
        return Ok(report);
    }

    for entry in &plan.planned {
        let request = match store
            .rollout_admin_request_runtime_upgrade(AdminRuntimeUpgradeExactInput {
                admin_verified_email: input.admin_email.clone(),
                admin_workos_user_id: input.admin_workos_user_id.clone(),
                project_id: entry.project_id.clone(),
                expected_agent_runtime_id: entry.agent_runtime_id.clone(),
                expected_source_host_id: entry.source_host_id.clone(),
                expected_source_machine_id: entry.source_machine_id.clone(),
                target_runtime_artifact_id: input.target_artifact_id.clone(),
                now: None,
            })
            .await
        {
            Ok(request) => request,
            Err(error) => {
                let detail = format!("failed to enqueue upgrade: {error}");
                report.outcomes.push(rollout_outcome(
                    entry,
                    None,
                    RuntimeArtifactRolloutOutcomeStatus::EnqueueFailed,
                    Some(detail.clone()),
                ));
                report.halted = true;
                report.halted_reason = Some(detail);
                return Ok(report);
            }
        };
        let request_id = request.id.clone();
        let terminal = match wait_for_runtime_artifact_upgrade(
            store,
            request,
            input.wait_timeout,
            poll_interval,
        )
        .await
        {
            Ok(RuntimeArtifactRolloutWaitResult::Terminal(request)) => request,
            Ok(RuntimeArtifactRolloutWaitResult::TimedOut(request)) => {
                let detail = format!(
                    "timed out while request {} remained {}; the live request was not cancelled",
                    request.id,
                    request.status.as_str()
                );
                report.outcomes.push(rollout_outcome(
                    entry,
                    Some(request.id),
                    RuntimeArtifactRolloutOutcomeStatus::TimedOut,
                    Some(detail.clone()),
                ));
                report.halted = true;
                report.halted_reason = Some(detail);
                return Ok(report);
            }
            Err(error) => {
                let detail = format!("failed to read exact request {request_id}: {error}");
                report.outcomes.push(rollout_outcome(
                    entry,
                    Some(request_id),
                    RuntimeArtifactRolloutOutcomeStatus::PollFailed,
                    Some(detail.clone()),
                ));
                report.halted = true;
                report.halted_reason = Some(detail);
                return Ok(report);
            }
        };
        if terminal.status == RuntimeControlRequestStatus::Failed {
            let detail = terminal
                .failure_message
                .clone()
                .unwrap_or_else(|| "runtime upgrade request failed without detail".to_string());
            report.outcomes.push(rollout_outcome(
                entry,
                Some(terminal.id),
                RuntimeArtifactRolloutOutcomeStatus::RequestFailed,
                Some(detail.clone()),
            ));
            report.halted = true;
            report.halted_reason = Some(detail);
            return Ok(report);
        }

        let overviews = match store.rollout_admin_runtime_overviews().await {
            Ok(overviews) => overviews,
            Err(error) => {
                let detail = format!("failed to verify upgraded Runtime overview: {error}");
                report.outcomes.push(rollout_outcome(
                    entry,
                    Some(terminal.id),
                    RuntimeArtifactRolloutOutcomeStatus::PostconditionFailed,
                    Some(detail.clone()),
                ));
                report.halted = true;
                report.halted_reason = Some(detail);
                return Ok(report);
            }
        };
        let postcondition_met = overviews.iter().any(|overview| {
            overview.project_id == entry.project_id
                && overview.agent_runtime_id == entry.agent_runtime_id
                && overview.source_host_id == entry.source_host_id
                && overview.source_machine_id == entry.source_machine_id
                && overview.runtime_artifact_id.as_deref()
                    == Some(input.target_artifact_id.as_str())
                && overview.runtime_status == RuntimeSummaryStatus::Online
        });
        if !postcondition_met {
            let detail = format!(
                "request {} succeeded, but Runtime {} is not online on artifact {}",
                terminal.id, entry.agent_runtime_id, input.target_artifact_id
            );
            report.outcomes.push(rollout_outcome(
                entry,
                Some(terminal.id),
                RuntimeArtifactRolloutOutcomeStatus::PostconditionFailed,
                Some(detail.clone()),
            ));
            report.halted = true;
            report.halted_reason = Some(detail);
            return Ok(report);
        }
        report.outcomes.push(rollout_outcome(
            entry,
            Some(terminal.id),
            RuntimeArtifactRolloutOutcomeStatus::Succeeded,
            None,
        ));
    }
    Ok(report)
}

async fn runtime_artifact_rollout_command(args: RuntimeArtifactRolloutCliArgs) -> Result<()> {
    let input = RuntimeArtifactRolloutInput::try_from(args)?;
    let store = postgres_store_from_env().await?;
    let report = runtime_artifact_rollout(&store, input, Duration::from_secs(2)).await?;
    let halted = report.halted;
    print_json(&report)?;
    if halted {
        bail!("runtime artifact rollout halted; see JSON report");
    }
    Ok(())
}

async fn runtime_archive_unrecoverable_command(
    args: RuntimeArchiveUnrecoverableCliArgs,
) -> Result<finite_saas_core::UnrecoverableRuntimeArchiveReceipt> {
    let store = postgres_store_from_env().await?;
    store
        .admin_archive_unrecoverable_runtime(AdminArchiveUnrecoverableRuntimeInput {
            admin_verified_email: args.admin_email,
            admin_workos_user_id: args.admin_workos_user_id,
            project_id: args.project_id,
            expected_agent_runtime_id: args.expected_agent_runtime_id,
            expected_source_host_id: args.expected_source_host_id,
            expected_source_machine_id: args.expected_source_machine_id,
            expected_owner_email: args.expected_owner_email,
            operator_observed_compute_absent: args.confirm_compute_absent,
            operator_observed_durable_state_absent: args.confirm_durable_state_absent,
            owner_acknowledged_unrecoverable: args.confirm_owner_acknowledged_unrecoverable,
            now: args.now,
        })
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
    let runtime_secret_references = optional_runtime_secret_references()?;
    postgres_store_with_retry(&database_url, timeout, retry_interval)
        .await?
        .with_runtime_environment(runtime_environment)
        .and_then(|store| store.with_runtime_secret_references(runtime_secret_references))
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

fn optional_runtime_secret_references() -> Result<Vec<String>> {
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

fn optional_agent_creation_placement() -> Result<Option<RuntimePlacement>> {
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
    use finite_saas_core::{
        CoreError, RuntimeCapabilitiesV1, RuntimeControlKind, RuntimeSummaryStatus,
    };
    use std::sync::Mutex;

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

    fn rollout_overview(
        project_id: &str,
        artifact_id: &str,
        runtime_link_active: bool,
        runtime_upgrade: bool,
        runtime_status: RuntimeSummaryStatus,
    ) -> AdminRuntimeOverview {
        AdminRuntimeOverview {
            project_id: project_id.to_string(),
            project_display_name: format!("Agent {project_id}"),
            owner_email: Some("owner@finite.vip".to_string()),
            agent_runtime_id: format!("runtime-{project_id}"),
            source_host_id: "lat1".to_string(),
            source_machine_id: format!("finite-kata-{project_id}"),
            runtime_artifact_id: Some(artifact_id.to_string()),
            runtime_artifact_version_label: Some(artifact_id.to_string()),
            runtime_status,
            last_heartbeat_at: Some("2026-07-15T01:00:00Z".to_string()),
            status_updated_at: Some("2026-07-15T01:00:00Z".to_string()),
            runtime_updated_at: "2026-07-15T01:00:00Z".to_string(),
            hermes_available: Some(true),
            published_app_urls: Vec::new(),
            active_finite_private_key_count: 1,
            runtime_link_active,
            runtime_capabilities: Some(RuntimeCapabilitiesV1 {
                restart: true,
                recover_known_good_chat: true,
                runtime_upgrade,
                stop: true,
                runtime_retirement: false,
            }),
        }
    }

    fn rollout_input(
        scope: RuntimeArtifactRolloutScope,
        plan_only: bool,
    ) -> RuntimeArtifactRolloutInput {
        RuntimeArtifactRolloutInput {
            target_artifact_id: "artifact-v2".to_string(),
            source_host_id: "lat1".to_string(),
            admin_email: "admin@finite.vip".to_string(),
            admin_workos_user_id: "workos-admin".to_string(),
            scope,
            expected_binding: None,
            plan_only,
            wait_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn rollout_planner_is_scoped_deterministic_and_canary_first() {
        let mut other_host = rollout_overview(
            "project-other-host",
            "artifact-v1",
            true,
            true,
            RuntimeSummaryStatus::Online,
        );
        other_host.source_host_id = "lat2".to_string();
        let overviews = vec![
            rollout_overview(
                "project-b",
                "artifact-v1",
                true,
                true,
                RuntimeSummaryStatus::Online,
            ),
            rollout_overview(
                "project-z-canary",
                "artifact-v1",
                true,
                true,
                RuntimeSummaryStatus::Online,
            ),
            rollout_overview(
                "project-inactive",
                "artifact-v1",
                false,
                true,
                RuntimeSummaryStatus::Offline,
            ),
            rollout_overview(
                "project-unsupported",
                "artifact-v1",
                true,
                false,
                RuntimeSummaryStatus::Online,
            ),
            rollout_overview(
                "project-current",
                "artifact-v2",
                true,
                true,
                RuntimeSummaryStatus::Online,
            ),
            other_host,
        ];
        let plan = plan_runtime_artifact_rollout(
            overviews.clone(),
            &RuntimeArtifactRolloutScope::All {
                canary_project_id: "project-z-canary".to_string(),
            },
            "lat1",
            "artifact-v2",
        );

        assert_eq!(
            plan.planned
                .iter()
                .map(|entry| entry.project_id.as_str())
                .collect::<Vec<_>>(),
            vec!["project-z-canary", "project-b"]
        );
        assert_eq!(
            plan.skipped
                .iter()
                .map(|entry| (entry.project_id.as_str(), entry.reason.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("project-current", "already_on_target_artifact"),
                ("project-inactive", "inactive_runtime_link"),
                ("project-unsupported", "runtime_upgrade_not_supported"),
            ]
        );
        assert!(plan.execution_blocked_reason.is_none());

        let selected = plan_runtime_artifact_rollout(
            overviews,
            &RuntimeArtifactRolloutScope::Projects(vec![
                "project-z-canary".to_string(),
                "project-b".to_string(),
                "project-missing".to_string(),
            ]),
            "lat1",
            "artifact-v2",
        );
        assert_eq!(
            selected
                .planned
                .iter()
                .map(|entry| entry.project_id.as_str())
                .collect::<Vec<_>>(),
            vec!["project-z-canary", "project-b"]
        );
        assert_eq!(selected.skipped.len(), 1);
        assert_eq!(selected.skipped[0].project_id, "project-missing");
        assert_eq!(selected.skipped[0].reason, "project_not_found");
        assert!(
            selected
                .execution_blocked_reason
                .as_deref()
                .unwrap()
                .contains("project-missing")
        );
    }

    #[test]
    fn rollout_cli_requires_one_scope_and_bounds_wait_timeout() {
        let common = [
            "finite-saas-core",
            "runtime-artifact-rollout",
            "--artifact-id",
            "artifact-v2",
            "--source-host-id",
            "lat1",
            "--admin-email",
            "admin@finite.vip",
            "--admin-workos-user-id",
            "workos-admin",
        ];
        assert!(
            Args::try_parse_from(common.into_iter().chain(["--project-id", "project-a"])).is_ok()
        );
        let exact = Args::try_parse_from(common.into_iter().chain([
            "--project-id",
            "project-a",
            "--expected-agent-runtime-id",
            "runtime-a",
            "--expected-source-machine-id",
            "finite-kata-a",
        ]))
        .unwrap();
        let Some(Command::RuntimeArtifactRollout(exact)) = exact.command else {
            panic!("expected rollout command");
        };
        assert!(RuntimeArtifactRolloutInput::try_from(exact).is_ok());
        assert!(Args::try_parse_from(common).is_err());
        assert!(
            Args::try_parse_from(common.into_iter().chain([
                "--all",
                "--canary-project-id",
                "project-a",
                "--project-id",
                "project-b",
            ]))
            .is_err()
        );
        assert!(
            Args::try_parse_from(common.into_iter().chain([
                "--project-id",
                "project-a",
                "--wait-timeout-seconds",
                "0",
            ]))
            .is_err()
        );
        assert!(
            Args::try_parse_from(common.into_iter().chain([
                "--project-id",
                "project-a",
                "--wait-timeout-seconds",
                "3601",
            ]))
            .is_err()
        );
    }

    #[test]
    fn unrecoverable_archive_cli_requires_all_three_acknowledgements() {
        let common = [
            "finite-saas-core",
            "runtime-archive-unrecoverable",
            "--project-id",
            "project-a",
            "--expected-agent-runtime-id",
            "runtime-a",
            "--expected-source-host-id",
            "lat1",
            "--expected-source-machine-id",
            "finite-kata-a",
            "--expected-owner-email",
            "owner@finite.vip",
            "--admin-email",
            "admin@finite.vip",
            "--admin-workos-user-id",
            "workos-admin",
        ];
        assert!(Args::try_parse_from(common).is_err());
        assert!(
            Args::try_parse_from(common.into_iter().chain([
                "--confirm-compute-absent",
                "--confirm-durable-state-absent",
                "--confirm-owner-acknowledged-unrecoverable",
            ]))
            .is_ok()
        );
    }

    struct FakeRolloutStore {
        initial_overviews: Vec<AdminRuntimeOverview>,
        refreshed_overviews: Vec<AdminRuntimeOverview>,
        overview_reads: Mutex<usize>,
        enqueued_projects: Mutex<Vec<String>>,
        enqueue_runtime_overrides: BTreeMap<String, String>,
        terminal_statuses: BTreeMap<String, RuntimeControlRequestStatus>,
    }

    impl FakeRolloutStore {
        fn new(
            initial_overviews: Vec<AdminRuntimeOverview>,
            refreshed_overviews: Vec<AdminRuntimeOverview>,
            terminal_statuses: BTreeMap<String, RuntimeControlRequestStatus>,
        ) -> Self {
            Self {
                initial_overviews,
                refreshed_overviews,
                overview_reads: Mutex::new(0),
                enqueued_projects: Mutex::new(Vec::new()),
                enqueue_runtime_overrides: BTreeMap::new(),
                terminal_statuses,
            }
        }

        fn with_enqueue_runtime_override(
            mut self,
            project_id: &str,
            agent_runtime_id: &str,
        ) -> Self {
            self.enqueue_runtime_overrides
                .insert(project_id.to_string(), agent_runtime_id.to_string());
            self
        }

        fn request(
            &self,
            project_id: &str,
            status: RuntimeControlRequestStatus,
        ) -> CoreResult<RuntimeControlRequest> {
            let overview = self
                .initial_overviews
                .iter()
                .find(|overview| overview.project_id == project_id)
                .ok_or(CoreError::ProjectNotFound)?;
            Ok(RuntimeControlRequest {
                id: format!("request-{project_id}"),
                project_id: project_id.to_string(),
                agent_runtime_id: overview.agent_runtime_id.clone(),
                source_host_id: overview.source_host_id.clone(),
                source_machine_id: overview.source_machine_id.clone(),
                requested_by_user_id: "user-admin".to_string(),
                kind: RuntimeControlKind::Upgrade,
                target_runtime_artifact_id: Some("artifact-v2".to_string()),
                status,
                runner_id: None,
                lease_token: None,
                lease_expires_at: None,
                failure_message: (status == RuntimeControlRequestStatus::Failed)
                    .then(|| "synthetic runner failure".to_string()),
                created_at: "2026-07-15T01:00:00Z".to_string(),
                updated_at: "2026-07-15T01:00:00Z".to_string(),
                completed_at: matches!(
                    status,
                    RuntimeControlRequestStatus::Succeeded | RuntimeControlRequestStatus::Failed
                )
                .then(|| "2026-07-15T01:00:01Z".to_string()),
            })
        }
    }

    impl RuntimeArtifactRolloutStore for FakeRolloutStore {
        async fn rollout_admin_runtime_overviews(&self) -> CoreResult<Vec<AdminRuntimeOverview>> {
            let mut reads = self.overview_reads.lock().unwrap();
            let overviews = if *reads == 0 {
                self.initial_overviews.clone()
            } else {
                self.refreshed_overviews.clone()
            };
            *reads += 1;
            Ok(overviews)
        }

        async fn rollout_admin_request_runtime_upgrade(
            &self,
            input: AdminRuntimeUpgradeExactInput,
        ) -> CoreResult<RuntimeControlRequest> {
            let actual_runtime_id = self
                .enqueue_runtime_overrides
                .get(&input.project_id)
                .cloned()
                .or_else(|| {
                    self.initial_overviews
                        .iter()
                        .find(|overview| overview.project_id == input.project_id)
                        .map(|overview| overview.agent_runtime_id.clone())
                })
                .ok_or(CoreError::ProjectNotFound)?;
            if actual_runtime_id != input.expected_agent_runtime_id {
                return Err(CoreError::RuntimeSpecMismatch);
            }
            self.enqueued_projects
                .lock()
                .unwrap()
                .push(input.project_id.clone());
            self.request(&input.project_id, RuntimeControlRequestStatus::Requested)
        }

        async fn rollout_runtime_control_request(
            &self,
            request_id: &str,
        ) -> CoreResult<RuntimeControlRequest> {
            let project_id = request_id
                .strip_prefix("request-")
                .ok_or(CoreError::RuntimeControlRequestNotFound)?;
            let status = self
                .terminal_statuses
                .get(project_id)
                .copied()
                .unwrap_or(RuntimeControlRequestStatus::Succeeded);
            self.request(project_id, status)
        }
    }

    #[tokio::test]
    async fn rollout_plan_only_performs_no_writes() {
        let initial = vec![rollout_overview(
            "project-a",
            "artifact-v1",
            true,
            true,
            RuntimeSummaryStatus::Online,
        )];
        let store = FakeRolloutStore::new(initial.clone(), initial, BTreeMap::new());
        let report = runtime_artifact_rollout(
            &store,
            rollout_input(
                RuntimeArtifactRolloutScope::Projects(vec!["project-a".to_string()]),
                true,
            ),
            Duration::from_millis(1),
        )
        .await
        .unwrap();

        assert_eq!(report.planned.len(), 1);
        assert!(report.outcomes.is_empty());
        assert!(store.enqueued_projects.lock().unwrap().is_empty());
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["target_artifact_id"], "artifact-v2");
        assert_eq!(json["planned"][0]["project_id"], "project-a");
        assert_eq!(json["planned"][0]["agent_runtime_id"], "runtime-project-a");
        assert_eq!(
            json["planned"][0]["project_display_name"],
            "Agent project-a"
        );
        assert_eq!(
            json["planned"][0]["source_machine_id"],
            "finite-kata-project-a"
        );
        assert_eq!(json["planned"][0]["source_host_id"], "lat1");
        assert_eq!(json["planned"][0]["target_artifact_id"], "artifact-v2");
    }

    #[tokio::test]
    async fn rollout_rejects_runtime_replaced_between_plan_and_enqueue() {
        let initial = vec![rollout_overview(
            "project-a",
            "artifact-v1",
            true,
            true,
            RuntimeSummaryStatus::Online,
        )];
        let store = FakeRolloutStore::new(initial.clone(), initial, BTreeMap::new())
            .with_enqueue_runtime_override("project-a", "runtime-replacement");
        let report = runtime_artifact_rollout(
            &store,
            rollout_input(
                RuntimeArtifactRolloutScope::Projects(vec!["project-a".to_string()]),
                false,
            ),
            Duration::from_millis(1),
        )
        .await
        .unwrap();

        assert!(report.halted);
        assert_eq!(report.outcomes.len(), 1);
        assert_eq!(
            report.outcomes[0].status,
            RuntimeArtifactRolloutOutcomeStatus::EnqueueFailed
        );
        assert!(store.enqueued_projects.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn rollout_stops_before_next_enqueue_on_failure() {
        let initial = vec![
            rollout_overview(
                "project-a",
                "artifact-v1",
                true,
                true,
                RuntimeSummaryStatus::Online,
            ),
            rollout_overview(
                "project-b",
                "artifact-v1",
                true,
                true,
                RuntimeSummaryStatus::Online,
            ),
        ];
        let store = FakeRolloutStore::new(
            initial.clone(),
            initial,
            BTreeMap::from([("project-a".to_string(), RuntimeControlRequestStatus::Failed)]),
        );
        let report = runtime_artifact_rollout(
            &store,
            rollout_input(
                RuntimeArtifactRolloutScope::Projects(vec![
                    "project-a".to_string(),
                    "project-b".to_string(),
                ]),
                false,
            ),
            Duration::from_millis(1),
        )
        .await
        .unwrap();

        assert!(report.halted);
        assert_eq!(report.outcomes.len(), 1);
        assert_eq!(
            report.outcomes[0].status,
            RuntimeArtifactRolloutOutcomeStatus::RequestFailed
        );
        assert_eq!(*store.enqueued_projects.lock().unwrap(), vec!["project-a"]);
    }

    #[tokio::test]
    async fn rollout_stops_before_next_enqueue_when_postcondition_is_not_met() {
        let initial = vec![
            rollout_overview(
                "project-a",
                "artifact-v1",
                true,
                true,
                RuntimeSummaryStatus::Online,
            ),
            rollout_overview(
                "project-b",
                "artifact-v1",
                true,
                true,
                RuntimeSummaryStatus::Online,
            ),
        ];
        let store = FakeRolloutStore::new(initial.clone(), initial, BTreeMap::new());
        let report = runtime_artifact_rollout(
            &store,
            rollout_input(
                RuntimeArtifactRolloutScope::Projects(vec![
                    "project-a".to_string(),
                    "project-b".to_string(),
                ]),
                false,
            ),
            Duration::from_millis(1),
        )
        .await
        .unwrap();

        assert!(report.halted);
        assert_eq!(
            report.outcomes[0].status,
            RuntimeArtifactRolloutOutcomeStatus::PostconditionFailed
        );
        assert_eq!(*store.enqueued_projects.lock().unwrap(), vec!["project-a"]);
    }

    #[tokio::test]
    async fn rollout_timeout_reports_live_request_without_cancellation_claim() {
        let initial = vec![rollout_overview(
            "project-a",
            "artifact-v1",
            true,
            true,
            RuntimeSummaryStatus::Online,
        )];
        let store = FakeRolloutStore::new(
            initial.clone(),
            initial,
            BTreeMap::from([(
                "project-a".to_string(),
                RuntimeControlRequestStatus::Running,
            )]),
        );
        let mut input = rollout_input(
            RuntimeArtifactRolloutScope::Projects(vec!["project-a".to_string()]),
            false,
        );
        input.wait_timeout = Duration::from_millis(1);
        let report = runtime_artifact_rollout(&store, input, Duration::from_millis(1))
            .await
            .unwrap();

        assert_eq!(
            report.outcomes[0].request_id.as_deref(),
            Some("request-project-a")
        );
        assert_eq!(
            report.outcomes[0].status,
            RuntimeArtifactRolloutOutcomeStatus::TimedOut
        );
        assert!(
            report.outcomes[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("was not cancelled")
        );
    }
}
