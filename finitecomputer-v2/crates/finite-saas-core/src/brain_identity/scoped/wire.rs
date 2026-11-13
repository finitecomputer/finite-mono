use super::*;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopedDescriptionsRequest {
    pub version: String,
    pub brain_server: String,
    pub brain_id: String,
    pub requested_by_public_key_hex: String,
    pub public_keys_hex: Vec<String>,
    pub participating_public_keys_hex: Vec<String>,
}

impl ScopedDescriptionsRequest {
    pub fn validate(&self, server: &str) -> Result<(), &'static str> {
        if self.version != DESCRIPTIONS_VERSION_V3 {
            return Err("unsupported descriptions version");
        }
        self.legacy_scope()
            .validate(server)
            .map_err(|_| "invalid v3 description scope or page")?;
        let page: BTreeSet<_> = self.public_keys_hex.iter().collect();
        let subset: BTreeSet<_> = self.participating_public_keys_hex.iter().collect();
        if subset.len() != self.participating_public_keys_hex.len() || !subset.is_subset(&page) {
            return Err("invalid v3 participating subset");
        }
        Ok(())
    }

    /// Internal v2 policy scope. The participating subset is still passed
    /// separately to the snapshot resolver and hint loader.
    pub(crate) fn legacy_scope(&self) -> BrainIdentityDescriptionsRequest {
        BrainIdentityDescriptionsRequest {
            version: DESCRIPTIONS_VERSION_V2.to_string(),
            brain_server: self.brain_server.clone(),
            brain_id: self.brain_id.clone(),
            requested_by_public_key_hex: self.requested_by_public_key_hex.clone(),
            public_keys_hex: self.public_keys_hex.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    HistoricalIdentity,
    InvitationKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperatorAnnotation {
    pub kind: AnnotationKind,
    pub label: String,
    pub observed_at: String,
}

impl OperatorAnnotation {
    pub fn valid(&self) -> bool {
        valid_annotation_label(&self.label) && valid_evidence_time(&self.observed_at)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperatorRecordState {
    Used,
    Suppressed,
    Conflict,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperatorRecordReason {
    AccountBindingChanged,
    IdentityEvidenceConflict,
    AgentBindingChanged,
    AnnotationSuperseded,
    NormalResultPrecedence,
    ContactUnavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperatorRecord {
    pub state: OperatorRecordState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<OperatorRecordReason>,
    pub revision: String,
    pub observed_at: String,
}

// Deliberately distinct from the strict legacy DTO. Flattening that DTO with
// deny_unknown_fields would either reject extensions or relax legacy parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopedIdentityDescription {
    pub public_key_hex: String,
    pub state: DescriptionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<DescribedKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<AgentLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responsible_account: Option<ResponsibleAccount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<DescriptionSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_annotation: Option<OperatorAnnotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_record: Option<OperatorRecord>,
}

impl From<IdentityDescription> for ScopedIdentityDescription {
    fn from(row: IdentityDescription) -> Self {
        Self {
            public_key_hex: row.public_key_hex,
            state: row.state,
            kind: row.kind,
            display_name: row.display_name,
            account_email: row.account_email,
            lifecycle: row.lifecycle,
            responsible_account: row.responsible_account,
            source: row.source,
            operator_annotation: None,
            operator_record: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopedDescriptionsResponse {
    pub version: String,
    pub brain_id: String,
    pub checked_at: String,
    pub results: Vec<ScopedIdentityDescription>,
}
