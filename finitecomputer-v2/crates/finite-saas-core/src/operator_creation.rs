//! Operator-created Agents: an admin creates an ordinary Agent owned by an
//! account named by email (migration 0047). Everything after the creation
//! request is Core's normal lease, launch and identity flow.

use crate::{
    AgentCreationRequest, HostingTier, Project, RuntimePlacement, RuntimeResourceClass, wire_enum,
};
use serde::{Deserialize, Serialize};

/// Longest operator reason Core stores (matches the migration's CHECK).
pub const MAX_OPERATOR_CREATION_REASON_CHARS: usize = 2_000;

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
    /// Stripe, no trial) and the org's creation entitlement grows by one, so
    /// the person's own self-serve allowance is unchanged.
    OperatorCreationAdmission {
    OperatorSponsored => "operator_sponsored",
    }
    parse: parse_operator_creation_admission
}

/// The admin acting, as the entry point verified it. The HTTP route takes it
/// from the WorkOS session; the root-only CLI takes it from its flags, and
/// Core then requires an existing linked account with both values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperatorIdentity {
    pub email: String,
    pub workos_user_id: String,
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
    /// The owner's hosted-chat account id (64 lowercase hex), when the
    /// operator knows it. A never-signed-in owner has none yet.
    pub owner_chat_account_id: Option<String>,
    pub profile_picture_url: Option<String>,
    /// Deployment placement override (local and conformance stacks), the
    /// same value self-serve creation receives.
    pub placement_override: Option<RuntimePlacement>,
    pub now: Option<String>,
}

/// The durable audit record (`operator_agent_creations`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperatorAgentCreation {
    pub id: String,
    pub idempotency_key: String,
    pub owner_email: String,
    pub owner_user_id: String,
    pub owner_account_created: bool,
    pub customer_org_id: String,
    pub customer_org_created: bool,
    pub project_id: String,
    pub agent_creation_request_id: String,
    pub display_name: String,
    pub hosting_tier: HostingTier,
    pub placement: RuntimePlacement,
    pub target_source_host_id: Option<String>,
    pub owner_chat_account_id: Option<String>,
    pub admission: OperatorCreationAdmission,
    pub reason: String,
    pub admin_user_id: String,
    pub admin: OperatorIdentity,
    pub surface: OperatorCreationSurface,
    pub created_at: String,
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
}
