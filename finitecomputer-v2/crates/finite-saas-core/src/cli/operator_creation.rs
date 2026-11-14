//! Root-only operator creation of an ordinary Agent for an account named by
//! email (migration 0047). Without `--execute` it is a rollback-only preview
//! that runs no schema DDL, so it needs Core with 0047 already applied.

use super::*;
use finite_saas_core::{
    OperatorCreateAgentInput, OperatorCreateAgentResult, OperatorCreationSurface, OperatorIdentity,
    parse_hosting_tier, parse_runtime_resource_class,
};

#[derive(Debug, clap::Args)]
pub(crate) struct AgentCreateForOwnerExactCliArgs {
    /// The person's account email. A pending account is created when Core
    /// has none; their later WorkOS sign-in with it makes them the owner.
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
    /// The owner's hosted-chat account id (64 hex), when known.
    #[arg(long)]
    pub(crate) owner_chat_account_id: Option<String>,
    #[arg(long)]
    pub(crate) profile_picture_url: Option<String>,
    /// The acting admin: an existing linked Core account with exactly this
    /// email and WorkOS user.
    #[arg(long)]
    pub(crate) admin_email: String,
    #[arg(long)]
    pub(crate) admin_workos_user_id: String,
    /// Commit the creation. Omit for a rollback-only preview.
    #[arg(long)]
    pub(crate) execute: bool,
    #[arg(long)]
    pub(crate) now: Option<String>,
}

pub(crate) fn operator_create_input(
    args: AgentCreateForOwnerExactCliArgs,
    placement_override: Option<RuntimePlacement>,
) -> Result<OperatorCreateAgentInput> {
    Ok(OperatorCreateAgentInput {
        admin: OperatorIdentity {
            email: args.admin_email,
            workos_user_id: args.admin_workos_user_id,
        },
        surface: OperatorCreationSurface::Cli,
        owner_email: args.owner_email,
        display_name: args.display_name,
        reason: args.reason,
        idempotency_key: args.idempotency_key,
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
        target_source_host_id: args.target_source_host_id,
        owner_chat_account_id: args.owner_chat_account_id,
        profile_picture_url: args.profile_picture_url,
        placement_override,
        now: args.now,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentCreateForOwnerOutput {
    pub(crate) dry_run: bool,
    pub(crate) result: OperatorCreateAgentResult,
}

pub(crate) async fn agent_create_for_owner_exact_command(
    args: AgentCreateForOwnerExactCliArgs,
) -> Result<AgentCreateForOwnerOutput> {
    if let Some(host) = args.target_source_host_id.as_deref() {
        let host = finite_saas_core::normalize_source_host_id(host)?;
        if !CoreAuth::from_env()?.has_kata_host(&host) {
            bail!("target must match an active host-bound Kata credential");
        }
    }
    let dry_run = !args.execute;
    let input = operator_create_input(args, optional_agent_creation_placement()?)?;
    let store = postgres_store_from_env(ImportMode::from_dry_run(dry_run)).await?;
    let result = store.admin_create_agent_for_owner(input).await?;
    Ok(AgentCreateForOwnerOutput { dry_run, result })
}
