//! Account, organization, Project, and chat-membership records.

use crate::{
    AgentCreationEntitlement, CustomerBillingAccount, HostingTier, RuntimePlacement, wire_enum,
};
use serde::Deserialize;
use serde::Serialize;

wire_enum! {
    BillingClass {
    Grandfathered => "grandfathered",
    Sponsored => "sponsored",
    Standard => "standard",
    }
    parse: parse_billing_class
}

wire_enum! {
    UserLinkStatus {
    Pending => "pending",
    Linked => "linked",
    }
    parse: parse_user_link_status
}

wire_enum! {
    ProjectMembershipRole {
    Owner => "owner",
    Admin => "admin",
    Member => "member",
    }
    parse: parse_project_membership_role
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreUser {
    pub id: String,
    pub email: String,
    pub status: UserLinkStatus,
    pub workos_user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomerOrganization {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub billing_class: BillingClass,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BillingOverview {
    pub customer_org: CustomerOrganization,
    pub billing_account: Option<CustomerBillingAccount>,
    pub agent_creation_entitlement: Option<AgentCreationEntitlement>,
    pub can_create_agent: bool,
    pub requires_billing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub customer_org_id: String,
    pub owner_user_id: String,
    pub display_name: String,
    /// Canonical human-facing Finite Identity for this hosted Agent Principal.
    /// Authorization continues to use the principal key resolved from it.
    #[serde(default)]
    pub agent_email: Option<String>,
    /// LEGACY ROWS ONLY. Set on projects created by the abandoned 2026-07
    /// existing-host import bridge (deleted; see git history for the
    /// reconcile/claim machinery). Production may still hold such rows from
    /// its near-ship test run. Nothing writes this anymore; a `Some` value
    /// means "hide from user-facing project lists" (`public_visible_projects`
    /// in api.rs). A future importer should define its own linkage rather
    /// than resurrecting this field's semantics.
    pub import_candidate_id: Option<String>,
    #[serde(default)]
    pub hosting_tier: Option<HostingTier>,
    #[serde(default)]
    pub placement: Option<RuntimePlacement>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRuntimeLink {
    pub id: String,
    pub project_id: String,
    pub agent_runtime_id: String,
    pub active: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatIdentity {
    pub id: String,
    pub user_id: String,
    pub kind: String,
    pub device_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRoomMembership {
    pub id: String,
    pub project_id: String,
    pub chat_identity_id: String,
    pub role: ProjectMembershipRole,
    pub created_at: String,
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinkVerifiedUserInput {
    pub verified_email: String,
    pub workos_user_id: String,
    pub now: Option<String>,
}
