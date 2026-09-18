//! Provider handles and the fenced, monotonic creation-operation ledger.

use crate::{AgentRuntime, CoreError, CoreResult, RunnerClass, RuntimePlacement};
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRuntimeHandleV1 {
    pub runner_class: RunnerClass,
    /// Adapter-owned JSON. Core stores and returns it without interpreting
    /// provider ids or copying them into source host/machine identity.
    pub opaque: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "schema", content = "handle")]
pub enum ProviderRuntimeHandleEnvelope {
    #[serde(rename = "provider_runtime_handle.v1")]
    V1(ProviderRuntimeHandleV1),
}

impl ProviderRuntimeHandleEnvelope {
    pub const fn runner_class(&self) -> RunnerClass {
        match self {
            Self::V1(handle) => handle.runner_class,
        }
    }
}

/// Versioned, provider-neutral journal for one creation request. Provider
/// identifiers remain opaque to Core; the request, placement, correlation, and
/// transition order are Core-owned invariants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "schema", content = "operation")]
pub enum ProviderOperationEnvelope {
    #[serde(rename = "provider_operation.v1")]
    V1(ProviderOperationV1),
}

impl ProviderOperationEnvelope {
    pub const fn v1(&self) -> &ProviderOperationV1 {
        match self {
            Self::V1(operation) => operation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOperationV1 {
    pub agent_creation_request_id: String,
    pub correlation_id: String,
    pub placement: RuntimePlacement,
    pub transitions: Vec<ProviderOperationTransitionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOperationTransitionRecord {
    pub sequence: u32,
    pub transition: ProviderOperationTransition,
    pub recorded_at: String,
}

/// Append-only creation states. `provider_facts` is deliberately untyped and
/// bounded: adapters can persist reconciliation evidence without teaching Core
/// provider vocabulary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderOperationTransition {
    CorrelationReserved,
    /// Core acknowledged that the runner is about to perform the first
    /// provider mutation. A crash after this boundary must reconcile the
    /// reserved correlation; it may never be treated as a pre-provider
    /// failure merely because no response facts were persisted yet.
    ProvisionStarted,
    Provisioned {
        provider_facts: Value,
    },
    ProvisionUnknown {
        provider_facts: Value,
    },
    CommitStarted,
    ProviderHandleRecorded {
        provider_runtime_handle: ProviderRuntimeHandleEnvelope,
    },
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordProviderOperationTransitionInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub correlation_id: String,
    pub placement: RuntimePlacement,
    pub transition: ProviderOperationTransition,
}

pub(crate) fn merge_provider_runtime_handle(
    existing: Option<&AgentRuntime>,
    incoming: Option<ProviderRuntimeHandleEnvelope>,
    placement: Option<RuntimePlacement>,
) -> CoreResult<(
    Option<ProviderRuntimeHandleEnvelope>,
    Vec<ProviderRuntimeHandleEnvelope>,
)> {
    let mut current = existing.and_then(|runtime| runtime.provider_runtime_handle.clone());
    let mut history = existing
        .map(|runtime| runtime.provider_runtime_handle_history.clone())
        .unwrap_or_default();
    if let Some(incoming) = incoming {
        let placement = placement.ok_or(CoreError::ProviderRuntimeHandlePlacementMismatch)?;
        if incoming.runner_class() != placement.runner_class {
            return Err(CoreError::ProviderRuntimeHandlePlacementMismatch);
        }
        if history.last() != Some(&incoming) {
            history.push(incoming.clone());
        }
        current = Some(incoming);
    }
    Ok((current, history))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderOperationTransitionKind {
    CorrelationReserved,
    ProvisionStarted,
    Provisioned,
    ProvisionUnknown,
    CommitStarted,
    ProviderHandleRecorded,
    Ready,
}

impl ProviderOperationTransition {
    fn kind(&self) -> ProviderOperationTransitionKind {
        match self {
            Self::CorrelationReserved => ProviderOperationTransitionKind::CorrelationReserved,
            Self::ProvisionStarted => ProviderOperationTransitionKind::ProvisionStarted,
            Self::Provisioned { .. } => ProviderOperationTransitionKind::Provisioned,
            Self::ProvisionUnknown { .. } => ProviderOperationTransitionKind::ProvisionUnknown,
            Self::CommitStarted => ProviderOperationTransitionKind::CommitStarted,
            Self::ProviderHandleRecorded { .. } => {
                ProviderOperationTransitionKind::ProviderHandleRecorded
            }
            Self::Ready => ProviderOperationTransitionKind::Ready,
        }
    }
}

fn normalize_provider_operation_correlation(value: &str) -> CoreResult<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(CoreError::InvalidProviderOperationCorrelation);
    }
    Ok(value.to_string())
}

fn provider_operation_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    [
        "secret",
        "token",
        "password",
        "credential",
        "authorization",
        "api_key",
        "private_key",
        "environment",
        "env",
    ]
    .iter()
    .any(|term| key == *term || key.ends_with(&format!("_{term}")))
}

fn validate_provider_operation_facts(value: &Value) -> CoreResult<()> {
    if !value.is_object()
        || serde_json::to_vec(value)
            .map_err(|_| CoreError::InvalidProviderOperationFacts)?
            .len()
            > 16 * 1024
    {
        return Err(CoreError::InvalidProviderOperationFacts);
    }

    fn visit(value: &Value, depth: usize, entries: &mut usize) -> bool {
        if depth > 8 {
            return false;
        }
        match value {
            Value::Object(object) => object.iter().all(|(key, value)| {
                *entries += 1;
                *entries <= 256
                    && key.len() <= 128
                    && !provider_operation_secret_key(key)
                    && visit(value, depth + 1, entries)
            }),
            Value::Array(array) => {
                *entries += array.len();
                *entries <= 256 && array.iter().all(|value| visit(value, depth + 1, entries))
            }
            Value::String(value) => value.len() <= 2_048,
            Value::Null | Value::Bool(_) | Value::Number(_) => true,
        }
    }

    if visit(value, 0, &mut 0) {
        Ok(())
    } else {
        Err(CoreError::InvalidProviderOperationFacts)
    }
}

fn validate_provider_operation_transition(
    transition: &ProviderOperationTransition,
) -> CoreResult<()> {
    match transition {
        ProviderOperationTransition::Provisioned { provider_facts }
        | ProviderOperationTransition::ProvisionUnknown { provider_facts } => {
            validate_provider_operation_facts(provider_facts)
        }
        ProviderOperationTransition::CorrelationReserved
        | ProviderOperationTransition::ProvisionStarted
        | ProviderOperationTransition::CommitStarted
        | ProviderOperationTransition::ProviderHandleRecorded { .. }
        | ProviderOperationTransition::Ready => Ok(()),
    }
}

pub(crate) fn append_provider_operation_transition(
    existing: Option<&ProviderOperationEnvelope>,
    request_id: &str,
    correlation_id: &str,
    placement: RuntimePlacement,
    transition: ProviderOperationTransition,
    recorded_at: &str,
) -> CoreResult<ProviderOperationEnvelope> {
    let correlation_id = normalize_provider_operation_correlation(correlation_id)?;
    validate_provider_operation_transition(&transition)?;

    let mut operation = match existing {
        Some(ProviderOperationEnvelope::V1(operation)) => {
            if operation.agent_creation_request_id != request_id
                || operation.correlation_id != correlation_id
                || operation.placement != placement
            {
                return Err(CoreError::ProviderOperationIdentityMismatch);
            }
            operation.clone()
        }
        None => ProviderOperationV1 {
            agent_creation_request_id: request_id.to_string(),
            correlation_id,
            placement,
            transitions: Vec::new(),
        },
    };

    if let Some(persisted) = operation
        .transitions
        .iter()
        .find(|persisted| persisted.transition.kind() == transition.kind())
    {
        if persisted.transition == transition {
            return Ok(ProviderOperationEnvelope::V1(operation));
        }
        return Err(CoreError::ProviderOperationTransitionConflict);
    }

    let previous = operation
        .transitions
        .last()
        .map(|record| record.transition.kind());
    let legal = matches!(
        (previous, transition.kind()),
        (None, ProviderOperationTransitionKind::CorrelationReserved)
            | (
                Some(ProviderOperationTransitionKind::CorrelationReserved),
                ProviderOperationTransitionKind::ProvisionStarted
            )
            | (
                Some(ProviderOperationTransitionKind::ProvisionStarted),
                ProviderOperationTransitionKind::Provisioned
                    | ProviderOperationTransitionKind::ProvisionUnknown
            )
            | (
                Some(ProviderOperationTransitionKind::ProvisionUnknown),
                ProviderOperationTransitionKind::Provisioned
            )
            | (
                Some(ProviderOperationTransitionKind::Provisioned),
                ProviderOperationTransitionKind::CommitStarted
            )
            | (
                Some(ProviderOperationTransitionKind::CommitStarted),
                ProviderOperationTransitionKind::ProviderHandleRecorded
            )
            | (
                Some(ProviderOperationTransitionKind::ProviderHandleRecorded),
                ProviderOperationTransitionKind::Ready
            )
    );
    if !legal {
        return Err(CoreError::ProviderOperationTransitionConflict);
    }

    operation
        .transitions
        .push(ProviderOperationTransitionRecord {
            sequence: operation.transitions.len() as u32,
            transition,
            recorded_at: recorded_at.to_string(),
        });
    Ok(ProviderOperationEnvelope::V1(operation))
}

pub(crate) fn provider_operation_at_runtime_boundary(
    existing: Option<&ProviderOperationEnvelope>,
    provider_runtime_handle: Option<&ProviderRuntimeHandleEnvelope>,
    ready: bool,
    recorded_at: &str,
) -> CoreResult<Option<ProviderOperationEnvelope>> {
    let Some(existing) = existing else {
        return Ok(None);
    };
    let operation = existing.v1();
    let handle = provider_runtime_handle
        .cloned()
        .ok_or(CoreError::ProviderOperationBoundaryNotReached)?;
    let mut updated = append_provider_operation_transition(
        Some(existing),
        &operation.agent_creation_request_id,
        &operation.correlation_id,
        operation.placement,
        ProviderOperationTransition::ProviderHandleRecorded {
            provider_runtime_handle: handle,
        },
        recorded_at,
    )?;
    if ready {
        updated = append_provider_operation_transition(
            Some(&updated),
            &operation.agent_creation_request_id,
            &operation.correlation_id,
            operation.placement,
            ProviderOperationTransition::Ready,
            recorded_at,
        )?;
    }
    Ok(Some(updated))
}

pub(crate) fn provider_operation_allows_generic_failure(
    operation: &ProviderOperationEnvelope,
) -> bool {
    matches!(
        operation
            .v1()
            .transitions
            .last()
            .map(|record| &record.transition),
        Some(ProviderOperationTransition::CorrelationReserved)
    )
}
