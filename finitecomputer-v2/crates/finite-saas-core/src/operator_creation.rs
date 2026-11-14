//! Operator-created Agents: an admin creates an ordinary Agent owned by an
//! account named by email (migration 0047). Before creating it the operator
//! path finds or creates the person's WorkOS sign-in user and their
//! hosted-chat identity, so the Agent admits only its owner from birth.
//! Everything after the creation request is Core's normal lease, launch and
//! identity flow.

use crate::{
    AgentCreationRequest, HostingTier, Project, RuntimePlacement, RuntimeResourceClass, wire_enum,
};
use serde::{Deserialize, Serialize};

/// Longest operator reason Core stores (matches the migration's CHECK).
pub const MAX_OPERATOR_CREATION_REASON_CHARS: usize = 2_000;

/// Longest operator idempotency key; longer keys are refused, never cut.
pub const MAX_OPERATOR_IDEMPOTENCY_KEY_CHARS: usize = 128;

/// Self-serve creation refuses idempotency keys with this prefix: operator
/// creation requests carry it, so the two writers never share a key.
pub const OPERATOR_REQUEST_KEY_PREFIX: &str = "operator:";

wire_enum! {
    /// Which entry point recorded the creation.
    OperatorCreationSurface {
    AdminApi => "admin_api",
    Cli => "cli",
    }
    parse: parse_operator_creation_surface
}

wire_enum! {
    /// How an operator creation was admitted in place of a Launch Code or
    /// billing. `operator_sponsored`: a new owner org is minted sponsored (no
    /// Stripe, no trial), the Agent is left out of the org's self-serve
    /// allowance count, and the org can never redeem an event trial.
    OperatorCreationAdmission {
    OperatorSponsored => "operator_sponsored",
    }
    parse: parse_operator_creation_admission
}

/// The admin acting, as the entry point verified it. The HTTP route takes it
/// from the WorkOS session; the root-only CLI takes it from its flags and
/// verifies it with WorkOS. Core then requires an existing linked account
/// with both values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperatorIdentity {
    pub email: String,
    pub workos_user_id: String,
}

/// The owner's sign-in identity the operator path provisioned: the WorkOS
/// user for their email and the hosted-chat account that user owns. The
/// Agent is locked to `chat_account_id` at launch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OwnerSignInIdentity {
    pub workos_user_id: String,
    pub chat_account_id: String,
}

/// One operator request to create an Agent for the account named by
/// `owner_email`. Optional placement fields default to what a self-created
/// Standard Agent gets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperatorCreateAgentInput {
    pub admin: OperatorIdentity,
    pub surface: OperatorCreationSurface,
    pub owner_email: String,
    pub display_name: String,
    pub reason: String,
    pub idempotency_key: String,
    pub hosting_tier: Option<HostingTier>,
    pub runtime_resource_class: Option<RuntimeResourceClass>,
    /// Kata only: lease this creation on one exact registered host.
    pub target_source_host_id: Option<String>,
    /// Required to commit. `None` is accepted only by a rollback-only
    /// preview store, which then writes a placeholder it never commits.
    pub owner_identity: Option<OwnerSignInIdentity>,
    /// The operator confirmed adding an Agent to an account that has
    /// already signed in. Refused without it; pending and new accounts
    /// do not need it.
    pub existing_account_confirmed: bool,
    /// CLI only: the OS account that ran it (`SUDO_USER`, else `root`).
    pub cli_os_user: Option<String>,
    pub profile_picture_url: Option<String>,
    /// Deployment placement override (local and conformance stacks), the
    /// same value self-serve creation receives.
    pub placement_override: Option<RuntimePlacement>,
    /// Tests only; no entry point sets it.
    pub now: Option<String>,
}

/// Who the Agent belongs to, as this creation found or made it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperatorCreationOwner {
    pub email: String,
    pub user_id: String,
    /// True only when this creation's own INSERT minted the pending account.
    pub account_created: bool,
    pub customer_org_id: String,
    /// True only when this creation's own INSERT minted the personal org.
    pub customer_org_created: bool,
    pub workos_user_id: String,
    pub chat_account_id: String,
    pub existing_account_confirmed: bool,
}

/// Who did it and through which entry point.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperatorCreationActor {
    pub admin_user_id: String,
    pub admin: OperatorIdentity,
    pub surface: OperatorCreationSurface,
    pub cli_os_user: Option<String>,
}

/// The durable audit record (`operator_agent_creations`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperatorAgentCreation {
    pub id: String,
    pub idempotency_key: String,
    pub owner: OperatorCreationOwner,
    pub project_id: String,
    pub agent_creation_request_id: String,
    pub display_name: String,
    pub hosting_tier: HostingTier,
    pub placement: RuntimePlacement,
    pub target_source_host_id: Option<String>,
    pub admission: OperatorCreationAdmission,
    pub reason: String,
    pub actor: OperatorCreationActor,
    pub created_at: String,
}

/// A record as operators read it back, with the owner's account as it is
/// now. `signedInAsProvisionedUser` is true once the person has signed in
/// with the WorkOS user the creation provisioned, which is what makes their
/// chat identity the one the Agent admits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperatorAgentCreationListing {
    pub record: OperatorAgentCreation,
    pub owner_current_email: String,
    pub owner_linked: bool,
    pub owner_current_workos_user_id: Option<String>,
    pub signed_in_as_provisioned_user: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperatorCreateAgentResult {
    pub record: OperatorAgentCreation,
    pub project: Project,
    pub request: AgentCreationRequest,
    /// True when the idempotency key already named this creation.
    pub reused: bool,
    /// False while the owner has never signed in through WorkOS.
    pub owner_linked: bool,
    /// False only in a preview, whose owner identity is a placeholder.
    pub owner_identity_provisioned: bool,
}
