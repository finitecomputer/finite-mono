use super::*;

#[derive(Debug, Parser)]
#[command(name = "finite-saas-core")]
pub(crate) struct Args {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Root-only: reserve a registered Kata host and bind one unused Launch Code.
    #[command(name = "launch-code-target-exact")]
    LaunchCodeTargetExact {
        #[arg(long)]
        code_id: String,
        #[arg(long)]
        expected_batch_id: String,
        #[arg(long)]
        target_source_host_id: String,
        #[arg(long)]
        operator_email: String,
        #[arg(long)]
        operator_workos_user_id: String,
    },
    /// Root-only: append one canary retry after an exact completed misplacement.
    #[command(name = "launch-code-retry-target-exact")]
    LaunchCodeRetryTargetExact {
        #[arg(long)]
        code_id: String,
        #[arg(long)]
        expected_batch_id: String,
        #[arg(long)]
        previous_code_id: String,
        #[arg(long)]
        expected_previous_request_id: String,
        #[arg(long)]
        expected_previous_project_id: String,
        #[arg(long)]
        expected_previous_runtime_id: String,
        #[arg(long)]
        expected_previous_source_host_id: String,
        #[arg(long)]
        target_source_host_id: String,
        #[arg(long)]
        operator_email: String,
        #[arg(long)]
        operator_workos_user_id: String,
        /// Commit the retry. Omit for rollback-only preview (schema must exist).
        #[arg(long)]
        execute: bool,
    },
    /// Root-only: end an exact canary reservation for shared-pool admission.
    #[command(name = "launch-host-release-exact")]
    LaunchHostReleaseExact {
        #[arg(long)]
        reservation_code_id: String,
        #[arg(long)]
        source_host_id: String,
        #[arg(long)]
        expected_canary_runtime_id: String,
        #[arg(long)]
        operator_email: String,
        #[arg(long)]
        operator_workos_user_id: String,
        /// Commit the release; omit for a rollback-only preview.
        #[arg(long)]
        execute: bool,
    },
    /// Run the Core HTTP API.
    Serve,
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
    /// Retire one exact Runtime through the verified Recovery Snapshot lifecycle.
    #[command(name = "runtime-retire-exact")]
    RuntimeRetireExact(RuntimeRetireExactCliArgs),
    /// Rebind one stopped Kata Runtime after its exact durable state is staged.
    #[command(name = "runtime-cold-relocate-exact")]
    RuntimeColdRelocateExact(RuntimeColdRelocateExactCliArgs),
    /// Archive a legacy Runtime only after exact binding and absence attestations.
    #[command(name = "runtime-archive-unrecoverable")]
    RuntimeArchiveUnrecoverable(RuntimeArchiveUnrecoverableCliArgs),
    /// Complete offboarding for a Runtime whose verified retirement receipt is stored.
    #[command(name = "runtime-offboard-retired-exact")]
    RuntimeOffboardRetiredExact(RuntimeOffboardRetiredExactCliArgs),
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
pub(crate) struct RuntimeArtifactRolloutCliArgs {
    /// Exact promoted runtime artifact id to deploy.
    #[arg(long)]
    pub(crate) artifact_id: String,
    /// Exact source host whose active Runtimes may appear in the plan.
    #[arg(long)]
    pub(crate) source_host_id: String,
    /// Verified email recorded as the admin actor.
    #[arg(long)]
    pub(crate) admin_email: String,
    /// WorkOS user id recorded as the admin actor.
    #[arg(long)]
    pub(crate) admin_workos_user_id: String,
    /// Project to upgrade. Repeat for a deterministic selected-project rollout.
    #[arg(long = "project-id")]
    pub(crate) project_ids: Vec<String>,
    /// Roll every eligible active Runtime. Requires an explicit canary project.
    #[arg(long, requires = "canary_project_id")]
    pub(crate) all: bool,
    /// Project upgraded first during an --all rollout.
    #[arg(long, requires = "all")]
    pub(crate) canary_project_id: Option<String>,
    /// Exact Runtime id from a prior plan. Requires one explicit project.
    #[arg(long, requires_all = ["expected_source_machine_id"])]
    pub(crate) expected_agent_runtime_id: Option<String>,
    /// Exact source machine id from a prior plan. Requires one explicit project.
    #[arg(long, requires_all = ["expected_agent_runtime_id"])]
    pub(crate) expected_source_machine_id: Option<String>,
    /// Print the deterministic plan without enqueueing any lifecycle request.
    #[arg(long)]
    pub(crate) plan_only: bool,
    /// Maximum seconds to wait for each exact lifecycle request to finish.
    #[arg(
        long = "wait-timeout-seconds",
        default_value_t = 900,
        value_parser = clap::value_parser!(u64).range(1..=3600)
    )]
    pub(crate) wait_timeout_seconds: u64,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RuntimeArchiveUnrecoverableCliArgs {
    #[arg(long)]
    pub(crate) project_id: String,
    #[arg(long)]
    pub(crate) expected_agent_runtime_id: String,
    #[arg(long)]
    pub(crate) expected_source_host_id: String,
    #[arg(long)]
    pub(crate) expected_source_machine_id: String,
    #[arg(long)]
    pub(crate) expected_owner_email: String,
    #[arg(long)]
    pub(crate) admin_email: String,
    #[arg(long)]
    pub(crate) admin_workos_user_id: String,
    #[arg(long, required = true, action = clap::ArgAction::SetTrue)]
    pub(crate) confirm_compute_absent: bool,
    #[arg(long, required = true, action = clap::ArgAction::SetTrue)]
    pub(crate) confirm_durable_state_absent: bool,
    #[arg(long, required = true, action = clap::ArgAction::SetTrue)]
    pub(crate) confirm_owner_acknowledged_unrecoverable: bool,
    #[arg(long)]
    pub(crate) now: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RuntimeOffboardRetiredExactCliArgs {
    #[arg(long)]
    pub(crate) project_id: String,
    #[arg(long)]
    pub(crate) expected_agent_runtime_id: String,
    #[arg(long)]
    pub(crate) expected_source_host_id: String,
    #[arg(long)]
    pub(crate) expected_source_machine_id: String,
    #[arg(long)]
    pub(crate) expected_owner_email: String,
    #[arg(long)]
    pub(crate) admin_email: String,
    #[arg(long)]
    pub(crate) admin_workos_user_id: String,
    /// Operator attestation that canonical compute for this exact Runtime was
    /// independently confirmed absent before offboarding completes.
    #[arg(long, required = true, action = clap::ArgAction::SetTrue)]
    pub(crate) confirm_compute_absent: bool,
    #[arg(long)]
    pub(crate) now: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RuntimeRetireExactCliArgs {
    #[arg(long)]
    pub(crate) project_id: String,
    #[arg(long)]
    pub(crate) expected_agent_runtime_id: String,
    #[arg(long)]
    pub(crate) expected_source_host_id: String,
    #[arg(long)]
    pub(crate) expected_source_machine_id: String,
    #[arg(long)]
    pub(crate) admin_email: String,
    #[arg(long)]
    pub(crate) admin_workos_user_id: String,
    #[arg(
        long = "wait-timeout-seconds",
        default_value_t = 1800,
        value_parser = clap::value_parser!(u64).range(60..=3600)
    )]
    pub(crate) wait_timeout_seconds: u64,
    #[arg(long)]
    pub(crate) now: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RuntimeColdRelocateExactCliArgs {
    #[arg(long)]
    pub(crate) project_id: String,
    #[arg(long)]
    pub(crate) expected_agent_runtime_id: String,
    #[arg(long)]
    pub(crate) expected_source_host_id: String,
    #[arg(long)]
    pub(crate) expected_source_machine_id: String,
    #[arg(long)]
    pub(crate) target_source_host_id: String,
    #[arg(long)]
    pub(crate) expected_agent_npub: String,
    #[arg(long)]
    pub(crate) durable_state_manifest_sha256: String,
    /// Recovery variant: the operator has verified via the relocation
    /// runbook's bounded probe that no container or task exists for the
    /// source machine. Accepts a `stale` source and waives the succeeded
    /// stop receipt; all exact-match checks still apply.
    #[arg(long)]
    pub(crate) source_compute_absent: bool,
    #[arg(long)]
    pub(crate) admin_email: String,
    #[arg(long)]
    pub(crate) admin_workos_user_id: String,
    #[arg(long)]
    pub(crate) now: Option<String>,
}
