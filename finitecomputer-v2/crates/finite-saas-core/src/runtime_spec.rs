//! Versioned runtime specifications, boot intent, and environment validation.

use crate::{
    CoreError, CoreResult, RuntimeArtifact, RuntimeArtifactKind, RuntimePlacement,
    runtime_artifact_reference_is_immutable_oci,
};
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEndpointContractV1 {
    pub service_port: u16,
    pub health_path: String,
    pub contact_path: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBootIntent {
    #[default]
    Normal,
    RecoverKnownGood,
}

/// Complete immutable launch input owned by Core. The envelope is explicitly
/// versioned so readers reject an unknown contract instead of guessing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSpecV1 {
    pub operation_id: String,
    pub project_id: String,
    pub agent_runtime_id: String,
    pub placement: RuntimePlacement,
    pub runtime_artifact_id: String,
    pub runtime_image_digest: String,
    pub state_schema_version: String,
    pub durable_state_id: String,
    pub endpoints: RuntimeEndpointContractV1,
    #[serde(default)]
    pub boot_intent: RuntimeBootIntent,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub secret_references: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "schema", content = "spec")]
pub enum RuntimeSpecEnvelope {
    #[serde(rename = "runtime_spec.v1")]
    V1(RuntimeSpecV1),
}

const RUNTIME_SPEC_SERVICE_PORT: u16 = 8080;

const RUNTIME_SPEC_HEALTH_PATH: &str = "/healthz";

const RUNTIME_SPEC_CONTACT_PATH: &str = "/contact";

pub(crate) const FINITE_PRIVATE_SECRET_REFERENCE: &str = "FINITE_PRIVATE_API_KEY";

pub(crate) struct RuntimeSpecIdentity<'a> {
    pub operation_id: &'a str,
    pub project_id: &'a str,
    pub agent_runtime_id: &'a str,
    pub placement: RuntimePlacement,
}

pub(crate) fn build_runtime_spec_v1(
    identity: RuntimeSpecIdentity<'_>,
    artifact: &RuntimeArtifact,
    durable_state_id: &str,
    environment: BTreeMap<String, String>,
    secret_references: Vec<String>,
    boot_intent: RuntimeBootIntent,
) -> CoreResult<RuntimeSpecEnvelope> {
    let spec = RuntimeSpecV1 {
        operation_id: identity.operation_id.to_string(),
        project_id: identity.project_id.to_string(),
        agent_runtime_id: identity.agent_runtime_id.to_string(),
        placement: identity.placement,
        runtime_artifact_id: artifact.id.clone(),
        // This is the complete immutable OCI reference, including its digest;
        // adapters must not reconstruct a repository from process state.
        runtime_image_digest: artifact.reference.clone(),
        state_schema_version: artifact.state_schema_version.clone(),
        durable_state_id: durable_state_id.to_string(),
        endpoints: RuntimeEndpointContractV1 {
            service_port: RUNTIME_SPEC_SERVICE_PORT,
            health_path: RUNTIME_SPEC_HEALTH_PATH.to_string(),
            contact_path: RUNTIME_SPEC_CONTACT_PATH.to_string(),
        },
        boot_intent,
        environment,
        secret_references,
    };
    validate_runtime_spec_v1(&spec, artifact)?;
    Ok(RuntimeSpecEnvelope::V1(spec))
}

pub(crate) fn runtime_spec_secret_references(
    configured_references: &[String],
) -> CoreResult<Vec<String>> {
    let mut seen = BTreeSet::from([FINITE_PRIVATE_SECRET_REFERENCE.to_string()]);
    let mut references = vec![FINITE_PRIVATE_SECRET_REFERENCE.to_string()];

    for reference in configured_references {
        if !runtime_spec_environment_key_is_valid(reference)
            || runtime_spec_reserved_environment_key(reference)
            || !runtime_spec_secret_environment_key(reference)
            || !seen.insert(reference.clone())
        {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        references.push(reference.clone());
    }

    if references.len() > 64 {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    Ok(references)
}

pub(crate) fn runtime_spec_v1(spec: &RuntimeSpecEnvelope) -> &RuntimeSpecV1 {
    match spec {
        RuntimeSpecEnvelope::V1(spec) => spec,
    }
}

pub(crate) fn validate_runtime_spec_binding(
    envelope: &RuntimeSpecEnvelope,
    operation_id: Option<&str>,
    project_id: &str,
    agent_runtime_id: &str,
    placement: RuntimePlacement,
    artifact: &RuntimeArtifact,
) -> CoreResult<()> {
    let spec = runtime_spec_v1(envelope);
    validate_runtime_spec_v1(spec, artifact)?;
    if operation_id.is_some_and(|operation_id| spec.operation_id != operation_id)
        || spec.project_id != project_id
        || spec.agent_runtime_id != agent_runtime_id
        || spec.placement != placement
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    Ok(())
}

pub(crate) fn runtime_operation_spec_v1(
    current: &RuntimeSpecEnvelope,
    identity: RuntimeSpecIdentity<'_>,
    current_artifact: &RuntimeArtifact,
    desired_artifact: &RuntimeArtifact,
    boot_intent: RuntimeBootIntent,
    refreshed_environment: Option<&BTreeMap<String, String>>,
    refreshed_secret_references: Option<&[String]>,
) -> CoreResult<RuntimeSpecEnvelope> {
    validate_runtime_spec_binding(
        current,
        None,
        identity.project_id,
        identity.agent_runtime_id,
        identity.placement,
        current_artifact,
    )?;
    let current = runtime_spec_v1(current);
    let secret_references = if let Some(configured) = refreshed_secret_references {
        runtime_spec_secret_references(configured)?
    } else {
        current.secret_references.clone()
    };
    // No carry-forward of `OWNER_CHAT_NPUBS_ENV` here: the value is only a
    // birth-time seed. The sidecar's SQLite store consumes it once into the
    // Welcome admission policy on first boot, so an upgrade-time environment
    // refresh that drops it cannot reopen admission after that first boot.
    let environment = match refreshed_environment {
        Some(configured) => configured.clone(),
        None => current.environment.clone(),
    };
    build_runtime_spec_v1(
        identity,
        desired_artifact,
        &current.durable_state_id,
        environment,
        secret_references,
        boot_intent,
    )
}

fn validate_runtime_spec_v1(spec: &RuntimeSpecV1, artifact: &RuntimeArtifact) -> CoreResult<()> {
    let ids_valid = [
        spec.operation_id.as_str(),
        spec.project_id.as_str(),
        spec.agent_runtime_id.as_str(),
        spec.runtime_artifact_id.as_str(),
        spec.durable_state_id.as_str(),
    ]
    .iter()
    .all(|value| {
        !value.trim().is_empty()
            && value.len() <= 256
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    });
    if !ids_valid
        || artifact.kind != RuntimeArtifactKind::OciImage
        || !runtime_artifact_reference_is_immutable_oci(&artifact.reference)
        || spec.runtime_artifact_id != artifact.id
        || spec.runtime_image_digest != artifact.reference
        || spec.state_schema_version != artifact.state_schema_version
        || spec.endpoints.service_port != RUNTIME_SPEC_SERVICE_PORT
        || spec.endpoints.health_path != RUNTIME_SPEC_HEALTH_PATH
        || spec.endpoints.contact_path != RUNTIME_SPEC_CONTACT_PATH
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }

    validate_runtime_spec_environment(&spec.environment)?;

    let mut references = BTreeSet::new();
    for reference in &spec.secret_references {
        if !runtime_spec_environment_key_is_valid(reference)
            || spec.environment.contains_key(reference)
            || !references.insert(reference)
            || (reference != FINITE_PRIVATE_SECRET_REFERENCE
                && !runtime_spec_secret_environment_key(reference))
        {
            return Err(CoreError::RuntimeSpecMismatch);
        }
    }
    if references.len() > 64 {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    Ok(())
}

pub(crate) fn validate_runtime_spec_environment(
    environment: &BTreeMap<String, String>,
) -> CoreResult<()> {
    let mut total_environment_bytes = 0usize;
    for (key, value) in environment {
        if !runtime_spec_environment_key_is_valid(key)
            || runtime_spec_reserved_environment_key(key)
            || runtime_spec_secret_environment_key(key)
            || value.is_empty()
            || value.len() > 4 * 1024
            || value.contains('\0')
        {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        total_environment_bytes = total_environment_bytes
            .saturating_add(key.len())
            .saturating_add(value.len());
    }
    if environment.len() > 64 || total_environment_bytes > 32 * 1024 {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    Ok(())
}

fn runtime_spec_environment_key_is_valid(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_uppercase() || (index > 0 && byte.is_ascii_digit())
        })
}

fn runtime_spec_reserved_environment_key(key: &str) -> bool {
    matches!(
        key,
        "FINITE_SERVER_URL"
            | "FINITECHAT_SERVER_URL"
            | "FINITECHAT_HOME"
            | "FINITE_HOME"
            | "HERMES_HOME"
            | "FINITECHAT_WORKSPACE"
            | "FINITE_AGENT_HTTP_HOST"
            | "FINITE_AGENT_HTTP_PORT"
            | "FINITECHAT_HERMES_AGENT_DEVICE_ID"
            | "FINITE_AGENT_ID"
            | "FINITE_AGENT_NAME"
            | "FINITECHAT_HERMES_AGENT_NAME"
            | "FINITECHAT_HERMES_ROOM_NAME"
            | "FINITECHAT_HERMES_AGENT_PICTURE_URL"
            | "FINITECHAT_HERMES_INBOUND_STREAM"
            | "FINITECHAT_ALLOW_ALL_USERS"
            | "FINITE_ALLOW_ALL_USERS"
            | "GATEWAY_ALLOW_ALL_USERS"
            | "FINITE_DEFAULT_INFERENCE_PROFILE"
            | "FINITE_PRIVATE_MODEL"
            | "FINITE_PRIVATE_BASE_URL"
            | "FINITE_PRIVATE_API_KEY"
            | "FINITECHAT_HERMES_MODEL"
            | "FINITECHAT_HERMES_PROVIDER"
            | "FINITECHAT_HERMES_BASE_URL"
            | "FINITECHAT_HERMES_API_MODE"
            | "FINITE_AGENT_BOOT_INTENT_JSON"
            | "FINITE_AGENT_STATE_ROOT"
            | "OPENAI_API_KEY"
    )
}

fn runtime_spec_secret_environment_key(key: &str) -> bool {
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"]
        .iter()
        .any(|part| key.split('_').any(|segment| segment == *part))
}

/// Lease-time spec-environment key carrying the owner chat account id list
/// (comma-separated 64-hex account ids). The runtime image derives the Hermes
/// adapter allowlist and the sidecar Welcome allowlist from it. Deliberately
/// not a reserved key: it is Core-managed per-request spec state, not
/// Core-global operator configuration.
pub(crate) const OWNER_CHAT_NPUBS_ENV: &str = "FINITECHAT_OWNER_NPUBS";
