use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use finite_saas_core::api::router_with_hosted_hermes_origins;
use finite_saas_core::auth::CoreAuth;
use finite_saas_core::hosted_hermes::HostedHermesOrigins;
use finite_saas_core::store::CoreStore;
use finite_saas_core::{
    AdminArchiveUnrecoverableRuntimeInput, AdminOffboardRetiredRuntimeInput, AdminRuntimeOverview,
    AdminRuntimeRelocateExactInput, AdminRuntimeRetireExactInput, AdminRuntimeUpgradeExactInput,
    ApproveFinitePrivateGrantInput, CoreResult, FinitePrivateApiKey, FinitePrivateGrant,
    IssueFinitePrivateApiKeyInput, IssueFinitePrivateFriendKeyInput, OffboardingPhase,
    ResetFinitePrivateUsageWindowInput, RevokeFinitePrivateApiKeyInput,
    RevokeFinitePrivateGrantInput, RotateFinitePrivateApiKeyInput, RuntimeArtifact,
    RuntimeArtifactKind, RuntimeControlRequest, RuntimeControlRequestStatus, RuntimePlacement,
    RuntimeSummaryStatus, UpsertRuntimeArtifactInput,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::{Instant, sleep};
use tower_http::trace::TraceLayer;
mod cli;
use cli::*;

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
        Command::LaunchHostReleaseExact {
            reservation_code_id,
            source_host_id,
            expected_canary_runtime_id,
            operator_email,
            operator_workos_user_id,
            execute,
        } => {
            let auth = CoreAuth::from_env()?;
            if !auth.has_kata_host(&source_host_id) {
                bail!("host must match an active host-bound Kata credential");
            }
            let input = finite_saas_core::ReleaseLaunchHostInput {
                reservation_code_id,
                source_host_id,
                expected_canary_runtime_id,
                operator_email,
                operator_workos_user_id,
            };
            let store = postgres_store_from_env(ImportMode::from_dry_run(!execute)).await?;
            store.release_launch_host_exact(&input).await?;
            print_json(&serde_json::json!({"release":input,"dryRun":!execute}))
        }
        Command::LaunchCodeTargetExact {
            code_id,
            expected_batch_id,
            target_source_host_id,
            operator_email,
            operator_workos_user_id,
        } => {
            let auth = CoreAuth::from_env()?;
            if !auth.has_kata_host(&target_source_host_id) {
                bail!("target must match an active host-bound Kata credential");
            }
            let store = postgres_store_from_env(ImportMode::Commit).await?;
            store
                .target_launch_code_exact(
                    &code_id,
                    &expected_batch_id,
                    &target_source_host_id,
                    &operator_email,
                    &operator_workos_user_id,
                )
                .await?;
            print_json(
                &serde_json::json!({"launchCodeId":code_id,"batchId":expected_batch_id,"targetSourceHostId":target_source_host_id,"targetedCreationOnly":true}),
            )
        }
        Command::LaunchCodeRetryTargetExact {
            code_id,
            expected_batch_id,
            previous_code_id,
            expected_previous_request_id,
            expected_previous_project_id,
            expected_previous_runtime_id,
            expected_previous_source_host_id,
            target_source_host_id,
            operator_email,
            operator_workos_user_id,
            execute,
        } => {
            let auth = CoreAuth::from_env()?;
            if !auth.has_kata_host(&target_source_host_id) {
                bail!("target must match an active host-bound Kata credential");
            }
            let input = finite_saas_core::RetryTargetedLaunchCodeInput {
                code_id,
                expected_batch_id,
                previous_code_id,
                expected_previous_request_id,
                expected_previous_project_id,
                expected_previous_runtime_id,
                expected_previous_source_host_id,
                target_source_host_id,
                operator_email,
                operator_workos_user_id,
            };
            let store = postgres_store_from_env(ImportMode::from_dry_run(!execute)).await?;
            store.retry_targeted_launch_code_exact(&input).await?;
            print_json(
                &serde_json::json!({"binding":input,"dryRun":!execute,"targetedCreationOnly":true}),
            )
        }
        Command::RuntimeArtifactRollout(args) => runtime_artifact_rollout_command(args).await,
        Command::RuntimeRetireExact(args) => runtime_retire_exact_command(args).await,
        Command::RuntimeColdRelocateExact(args) => {
            let request = runtime_cold_relocate_exact_command(args).await?;
            print_json(&request)
        }
        Command::RuntimeArchiveUnrecoverable(args) => {
            let receipt = runtime_archive_unrecoverable_command(args).await?;
            print_json(&receipt)
        }
        Command::RuntimeOffboardRetiredExact(args) => {
            let receipt = runtime_offboard_retired_exact_command(args).await?;
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
