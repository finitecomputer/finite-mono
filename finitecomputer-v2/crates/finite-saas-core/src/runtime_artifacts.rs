//! Immutable runtime artifacts and their material compatibility checks.

use crate::{CoreError, wire_enum};
use serde::Deserialize;
use serde::Serialize;

wire_enum! {
    RuntimeArtifactKind {
    OciImage => "oci_image",
    }
    parse: parse_runtime_artifact_kind
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeArtifact {
    pub id: String,
    pub kind: RuntimeArtifactKind,
    pub reference: String,
    pub version_label: String,
    pub source_git_sha: Option<String>,
    pub finitec_version: Option<String>,
    pub hermes_source_ref: Option<String>,
    pub finite_platform_plugin_ref: Option<String>,
    pub state_schema_version: String,
    pub base_image: Option<String>,
    #[serde(default)]
    pub recover_known_good_chat: bool,
    pub created_at: String,
    /// Unpromoted artifact approved only for this existing runtime.
    #[serde(default)]
    pub canary_runtime_id: Option<String>,
    pub promoted_at: Option<String>,
    pub retired_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpsertRuntimeArtifactInput {
    pub id: String,
    pub kind: RuntimeArtifactKind,
    pub reference: String,
    pub version_label: String,
    pub source_git_sha: Option<String>,
    pub finitec_version: Option<String>,
    pub hermes_source_ref: Option<String>,
    pub finite_platform_plugin_ref: Option<String>,
    pub state_schema_version: String,
    pub base_image: Option<String>,
    #[serde(default)]
    pub recover_known_good_chat: bool,
    #[serde(default)]
    pub canary_runtime_id: Option<String>,
    pub promoted: bool,
    pub now: Option<String>,
}

impl std::str::FromStr for RuntimeArtifactKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_runtime_artifact_kind(value)
            .ok_or_else(|| format!("invalid runtime artifact kind {value}"))
    }
}

pub(crate) fn runtime_artifact_reference_is_immutable_oci(reference: &str) -> bool {
    let Some((_, digest)) = reference.rsplit_once("@sha256:") else {
        return false;
    };
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Release identity is every artifact field other than lifecycle timestamps.
/// Once promoted or mounted by a Runtime, an id may only be upserted with this
/// exact material identity; promotion remains a one-way lifecycle transition.
pub(crate) fn runtime_artifact_material_matches(
    existing: &RuntimeArtifact,
    candidate: &RuntimeArtifact,
) -> bool {
    existing.canary_runtime_id == candidate.canary_runtime_id
        && existing.id == candidate.id
        && existing.kind == candidate.kind
        && existing.reference == candidate.reference
        && existing.version_label == candidate.version_label
        && existing.source_git_sha == candidate.source_git_sha
        && existing.finitec_version == candidate.finitec_version
        && existing.hermes_source_ref == candidate.hermes_source_ref
        && existing.finite_platform_plugin_ref == candidate.finite_platform_plugin_ref
        && existing.state_schema_version == candidate.state_schema_version
        && existing.base_image == candidate.base_image
        && existing.recover_known_good_chat == candidate.recover_known_good_chat
}

pub(crate) fn runtime_upgrade_prelease_rejection_is_terminal(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::MissingRuntimeArtifactId
            | CoreError::RuntimeArtifactNotFound
            | CoreError::RuntimeArtifactNotPromoted
            | CoreError::RuntimeArtifactRetired
            | CoreError::RuntimeUpgradeUnsupported
            | CoreError::RuntimeUpgradeStateSchemaIncompatible
            | CoreError::RuntimeUpgradeCompletionMismatch
    )
}
