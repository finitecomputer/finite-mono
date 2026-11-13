use super::*;
use std::collections::BTreeSet;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopedRecordKind {
    ManagedAgent,
    AccountChatKey,
    HistoricalIdentity,
    InvitationKey,
}
impl ScopedRecordKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManagedAgent => "managed_agent",
            Self::AccountChatKey => "account_chat_key",
            Self::HistoricalIdentity => "historical_identity",
            Self::InvitationKey => "invitation_key",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopedEvidence {
    pub sha256: String,
    pub reference: String,
    pub observed_at: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopedRecordInput {
    pub public_key_hex: String,
    pub kind: ScopedRecordKind,
    pub account_id: Option<String>,
    pub workos_subject: Option<String>,
    pub project_id: Option<String>,
    pub annotation_label: Option<String>,
    pub evidence: ScopedEvidence,
}

impl ScopedRecordInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_public_key_hex(&self.public_key_hex)
            || !valid_public_key_hex(&self.evidence.sha256)
            || !bounded_text(&self.evidence.reference, 1024)
            || !valid_evidence_time(&self.evidence.observed_at)
        {
            return Err("invalid scoped record key or evidence");
        }
        let account = self
            .account_id
            .as_deref()
            .is_some_and(|id| bounded_text(id, 200));
        let subject = self
            .workos_subject
            .as_deref()
            .is_some_and(|id| bounded_text(id, 200));
        let project = self
            .project_id
            .as_deref()
            .is_some_and(|id| bounded_text(id, 200));
        let valid = match self.kind {
            ScopedRecordKind::ManagedAgent => {
                account && subject && project && self.annotation_label.is_none()
            }
            ScopedRecordKind::AccountChatKey => {
                account && subject && self.project_id.is_none() && self.annotation_label.is_none()
            }
            ScopedRecordKind::HistoricalIdentity | ScopedRecordKind::InvitationKey => {
                self.account_id.is_none()
                    && self.workos_subject.is_none()
                    && self.project_id.is_none()
                    && self
                        .annotation_label
                        .as_deref()
                        .is_some_and(valid_annotation_label)
            }
        };
        if !valid {
            return Err("invalid scoped record kind-specific fields");
        }
        Ok(())
    }
    pub fn digest(&self) -> String {
        sha256(&serde_json::to_vec(self).expect("record serializes"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopedOperator {
    pub email: String,
    pub workos_subject: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScopedAction {
    Apply,
    Retire,
}
impl ScopedAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Retire => "retire",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopedManifest {
    pub version: String,
    pub operation_id: String,
    pub action: ScopedAction,
    pub brain_server: String,
    pub brain_id: String,
    pub operator: ScopedOperator,
    pub evidence_pack_sha256: String,
    pub not_before: String,
    pub expires_at: String,
    pub target_operation_id: Option<String>,
    pub records: Vec<ScopedRecordInput>,
}

impl ScopedManifest {
    /// Structural validation also runs on replay. Freshness applies only to
    /// new operations: an exact completed operation returns its old receipt.
    pub fn validate(&self, configured_server: &str) -> Result<(), &'static str> {
        if self.version != SCOPED_MANIFEST_VERSION
            || !valid_operation_id(&self.operation_id)
            || canonical_brain_server(configured_server).as_deref() != Some(configured_server)
            || self.brain_server != configured_server
            || !valid_brain_id(&self.brain_id)
            || !bounded_text(&self.operator.email, MAX_CONTACT_BYTES)
            || !bounded_text(&self.operator.workos_subject, 200)
            || !valid_public_key_hex(&self.evidence_pack_sha256)
            || !valid_evidence_time(&self.not_before)
            || !valid_evidence_time(&self.expires_at)
        {
            return Err("invalid scoped manifest or configured server mismatch");
        }
        if crate::parse_time(&self.expires_at).unwrap()
            <= crate::parse_time(&self.not_before).unwrap()
        {
            return Err("invalid scoped manifest window");
        }
        match self.action {
            ScopedAction::Apply => {
                if self.target_operation_id.is_some()
                    || self.records.is_empty()
                    || self.records.len() > MAX_DESCRIPTION_KEYS
                {
                    return Err("apply requires 1-100 records");
                }
                let mut keys = BTreeSet::new();
                for record in &self.records {
                    record.validate()?;
                    if !keys.insert(&record.public_key_hex) {
                        return Err("duplicate scoped key");
                    }
                }
            }
            ScopedAction::Retire => {
                if !self.records.is_empty()
                    || !self
                        .target_operation_id
                        .as_deref()
                        .is_some_and(valid_operation_id)
                    || self.target_operation_id.as_ref() == Some(&self.operation_id)
                {
                    return Err("retire requires one distinct creating operation and no records");
                }
            }
        }
        Ok(())
    }
    pub fn fresh(&self, now: OffsetDateTime) -> bool {
        crate::parse_time(&self.not_before).is_ok_and(|start| start <= now)
            && crate::parse_time(&self.expires_at).is_ok_and(|end| now < end)
            && self
                .records
                .iter()
                .all(|r| crate::parse_time(&r.evidence.observed_at).is_ok_and(|t| t <= now))
    }
    pub fn digest(&self) -> String {
        sha256(&serde_json::to_vec(self).expect("manifest serializes"))
    }
}

fn bounded_text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.trim() == value
        && !value.chars().any(char::is_control)
        && !["TODO", "TBD", "UNKNOWN", "PLACEHOLDER"].contains(&value.to_uppercase().as_str())
}

// Persisted afterimages/receipts below are a recovery contract. Future readers
// must accept existing receipts (new fields need Option/default); a writer
// format change also needs an explicit old-reader replay/retirement proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopedStoredRecord {
    pub id: String,
    pub revision: String,
    pub brain_server: String,
    pub brain_id: String,
    pub creating_operation_id: String,
    pub created_at: String,
    pub retired_at: Option<String>,
    pub record: ScopedRecordInput,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScopedDisposition {
    Created,
    Existing,
    Retired,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopedRecordReceipt {
    pub disposition: ScopedDisposition,
    pub record_sha256: String,
    pub afterimage: ScopedStoredRecord,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopedReceipt {
    pub operation_id: String,
    pub manifest_sha256: String,
    pub operator_account_id: String,
    pub action: ScopedAction,
    pub brain_server: String,
    pub brain_id: String,
    pub target_operation_id: Option<String>,
    pub recorded_at: String,
    pub records: Vec<ScopedRecordReceipt>,
}
