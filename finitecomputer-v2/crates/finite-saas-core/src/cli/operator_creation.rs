//! Root-only operator creation of an ordinary Agent for an account named by
//! email (migration 0047). Without `--execute` it is a rollback-only preview
//! that creates nothing at WorkOS or the hosted chat device either. Neither
//! mode runs schema DDL: the Core service owns migrations, so 0047 must be
//! applied (Core deployed) first.

use super::*;
use finite_saas_core::owner_identity::{
    OperatorCreateAgentOutcome, OwnerIdentityProvisioner, operator_create_agent,
};
use finite_saas_core::{
    OperatorCreateAgentInput, OperatorCreationSurface, OperatorIdentity, parse_hosting_tier,
    parse_runtime_resource_class,
};

#[derive(Debug, clap::Args)]
pub(crate) struct AgentCreateForOwnerExactCliArgs {
    /// The person's account email. Core finds or creates their WorkOS user
    /// and hosted-chat identity, and a pending Core account when it has
    /// none; their later sign-in with that email makes them the owner.
    #[arg(long)]
    pub(crate) owner_email: String,
    #[arg(long)]
    pub(crate) display_name: String,
    /// Why the operator creates this Agent; stored in the audit record.
    #[arg(long)]
    pub(crate) reason: String,
    /// Retrying with the same key returns the same creation.
    #[arg(long)]
    pub(crate) idempotency_key: String,
    #[arg(long, value_parser = ["standard", "confidential"])]
    pub(crate) hosting_tier: Option<String>,
    #[arg(long, value_parser = ["vcpu4_memory8_gib", "vcpu2_memory4_gib"])]
    pub(crate) runtime_resource_class: Option<String>,
    /// Kata only: lease on this exact registered host.
    #[arg(long)]
    pub(crate) target_source_host_id: Option<String>,
    #[arg(long)]
    pub(crate) profile_picture_url: Option<String>,
    /// The acting admin. WorkOS must report this verified email for this
    /// user and an active operator-org membership, and Core must hold one
    /// linked account with both.
    #[arg(long)]
    pub(crate) admin_email: String,
    #[arg(long)]
    pub(crate) admin_workos_user_id: String,
    /// Required to add an Agent to an account that has already signed in.
    #[arg(long)]
    pub(crate) existing_account_confirmed: bool,
    /// Commit the creation. Omit for a rollback-only preview.
    #[arg(long)]
    pub(crate) execute: bool,
}

pub(crate) fn operator_create_input(
    args: &AgentCreateForOwnerExactCliArgs,
    cli_os_user: String,
    placement_override: Option<RuntimePlacement>,
) -> Result<OperatorCreateAgentInput> {
    Ok(OperatorCreateAgentInput {
        admin: OperatorIdentity {
            email: args.admin_email.clone(),
            workos_user_id: args.admin_workos_user_id.clone(),
        },
        surface: OperatorCreationSurface::Cli,
        owner_email: args.owner_email.clone(),
        display_name: args.display_name.clone(),
        reason: args.reason.clone(),
        idempotency_key: args.idempotency_key.clone(),
        hosting_tier: args
            .hosting_tier
            .as_deref()
            .map(|tier| parse_hosting_tier(tier).context("unknown --hosting-tier"))
            .transpose()?,
        runtime_resource_class: args
            .runtime_resource_class
            .as_deref()
            .map(|class| {
                parse_runtime_resource_class(class).context("unknown --runtime-resource-class")
            })
            .transpose()?,
        target_source_host_id: args.target_source_host_id.clone(),
        owner_identity: None,
        existing_account_confirmed: args.existing_account_confirmed,
        cli_os_user: Some(cli_os_user),
        profile_picture_url: args.profile_picture_url.clone(),
        placement_override,
        now: None,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentCreateForOwnerOutput {
    pub(crate) dry_run: bool,
    #[serde(flatten)]
    pub(crate) outcome: OperatorCreateAgentOutcome,
}

pub(crate) async fn agent_create_for_owner_exact_command(
    args: AgentCreateForOwnerExactCliArgs,
) -> Result<AgentCreateForOwnerOutput> {
    // The effective OS uid, read directly: neither PATH nor the environment
    // is an authority. Checked before any configuration or database access.
    if !rustix::process::geteuid().is_root() {
        bail!("agent-create-for-owner-exact requires OS root");
    }
    let provisioner = OwnerIdentityProvisioner::from_env()?.context(
        "agent-create-for-owner-exact needs FC_HOSTED_WEB_DEVICE_URL and \
         FINITECHAT_HOSTED_API_TOKEN (load /etc/finite/hosted-web-device.env)",
    )?;
    if let Some(host) = args.target_source_host_id.as_deref() {
        let host = finite_saas_core::normalize_source_host_id(host)?;
        if !CoreAuth::from_env()?.has_kata_host(&host) {
            bail!("target must match an active host-bound Kata credential");
        }
    }
    let database_url = required_env("FC_CORE_DATABASE_URL")?;
    let cli_os_user = env::var("SUDO_USER")
        .ok()
        .map(|user| user.trim().to_string())
        .filter(|user| !user.is_empty())
        .unwrap_or_else(|| "root".to_string());
    let placement = optional_agent_creation_placement()?;
    run_agent_create_for_owner(&args, &database_url, &provisioner, cli_os_user, placement).await
}

/// Everything after the root and environment checks, so tests can drive the
/// real preview/execute wiring against a test database and backend.
pub(crate) async fn run_agent_create_for_owner(
    args: &AgentCreateForOwnerExactCliArgs,
    database_url: &str,
    provisioner: &OwnerIdentityProvisioner,
    cli_os_user: String,
    placement_override: Option<RuntimePlacement>,
) -> Result<AgentCreateForOwnerOutput> {
    let input = operator_create_input(args, cli_os_user, placement_override)?;
    provisioner.verify_operator(&input.admin).await?;
    let timeout = optional_duration_secs("FC_CORE_POSTGRES_CONNECT_TIMEOUT_SECS", 60)?;
    let store = tokio::time::timeout(timeout, async {
        if args.execute {
            CoreStore::connect(database_url).await
        } else {
            CoreStore::connect_dry_run(database_url).await
        }
    })
    .await
    .context("Core Postgres connection timed out")??;
    let outcome = operator_create_agent(&store, provisioner, input).await?;
    Ok(AgentCreateForOwnerOutput {
        dry_run: !args.execute,
        outcome,
    })
}
