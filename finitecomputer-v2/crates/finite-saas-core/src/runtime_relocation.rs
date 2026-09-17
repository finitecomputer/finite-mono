//! Persisted cold-relocation plans and exact registration binding checks.

use crate::{AgentCreationRequest, AgentRuntime, CoreError, CoreResult, RuntimeSummaryStatus};
use serde::Deserialize;
use serde::Serialize;

pub const RUNTIME_RELOCATION_SCHEMA: &str = "runtime_relocation.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRelocationV1 {
    pub source_host_id: String,
    pub source_machine_id: String,
    pub target_source_host_id: String,
    pub expected_agent_npub: String,
    pub durable_state_manifest_sha256: String,
    /// Operator-attested recovery variant: the source compute no longer
    /// exists (container/task absent at the provider), so there is no stop
    /// receipt to present and the runtime reads `stale`, not `offline`.
    /// Absence is a stronger single-writer guarantee than a stop receipt —
    /// the runbook's bounded absence probe is the attestation's basis.
    /// Additive within runtime_relocation.v1; absent means false.
    #[serde(default)]
    pub source_compute_absent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "schema", content = "relocation")]
pub enum RuntimeRelocationEnvelope {
    #[serde(rename = "runtime_relocation.v1")]
    V1(RuntimeRelocationV1),
}

impl RuntimeRelocationEnvelope {
    pub const fn v1(&self) -> &RuntimeRelocationV1 {
        match self {
            Self::V1(relocation) => relocation,
        }
    }
}

/// Exact operator boundary for a stopped Runtime cold relocation. Durable
/// state transfer remains a separately observable step; the target Runner
/// refuses to launch unless its tree hashes to this request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminRuntimeRelocateExactInput {
    pub admin_verified_email: String,
    pub admin_workos_user_id: String,
    pub project_id: String,
    pub expected_agent_runtime_id: String,
    pub expected_source_host_id: String,
    pub expected_source_machine_id: String,
    pub target_source_host_id: String,
    pub expected_agent_npub: String,
    pub durable_state_manifest_sha256: String,
    /// Recovery variant (same attestation pattern as
    /// `AdminArchiveUnrecoverableRuntimeInput`): the operator has verified
    /// via the runbook's bounded probe that no container or task exists for
    /// the source machine. Relaxes exactly two gates — `stale` is accepted
    /// alongside `offline`, and the succeeded-stop-receipt requirement is
    /// waived (stopping absent compute fails by definition). Every other
    /// exact-match check still applies.
    #[serde(default)]
    pub operator_observed_compute_absent: bool,
    pub now: Option<String>,
}

pub(crate) fn validate_runtime_relocation_registration(
    request: &AgentCreationRequest,
    existing_runtime: Option<&AgentRuntime>,
    reported_source_host_id: &str,
    reported_source_machine_id: &str,
) -> CoreResult<()> {
    let Some(relocation) = request
        .relocation
        .as_ref()
        .map(RuntimeRelocationEnvelope::v1)
    else {
        return Ok(());
    };
    let existing_runtime = existing_runtime.ok_or(CoreError::RuntimeSpecMismatch)?;
    // `offline` is the cleanly-stopped case; `stale` and `online` are
    // acceptable only when the envelope itself was minted under the
    // operator's compute-absent attestation (a failed control marks a
    // runtime stale, and absent compute can never produce the stop
    // receipt that would make it offline; `online` is the pre-death last
    // report, equally frozen once the operator attests the compute is
    // absent — keep in sync with the enqueue gate in store.rs).
    let source_status_frozen = match existing_runtime.host_facts.runtime_status {
        RuntimeSummaryStatus::Offline => true,
        RuntimeSummaryStatus::Online => relocation.source_compute_absent,
        RuntimeSummaryStatus::Stale => relocation.source_compute_absent,
        _ => false,
    };
    let source_is_frozen = relocation.source_host_id == existing_runtime.source_host_id
        && relocation.source_machine_id == existing_runtime.source_machine_id
        && source_status_frozen;
    let target_is_registered = relocation.target_source_host_id == existing_runtime.source_host_id
        && relocation.source_machine_id == existing_runtime.source_machine_id;
    if request.agent_runtime_id.as_deref() != Some(existing_runtime.id.as_str())
        || request.target_source_host_id.as_deref()
            != Some(relocation.target_source_host_id.as_str())
        || relocation.target_source_host_id != reported_source_host_id
        || relocation.source_machine_id != reported_source_machine_id
        || (!source_is_frozen && !target_is_registered)
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    Ok(())
}
