//! Runtime capability envelopes and the policy that bounds them.

use crate::{
    AgentRuntime, CoreError, CoreResult, RunnerClass, RuntimeArtifact, RuntimeControlKind,
    RuntimePlacement,
};
use serde::Deserialize;
use serde::Serialize;

/// Provider-neutral controls the current Runtime can actually perform. This
/// deliberately excludes the not-yet-proven ensure/inspect/adopt contract;
/// internal adapter helpers are not product capabilities.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCapabilitiesV1 {
    #[serde(default)]
    pub restart: bool,
    #[serde(default)]
    pub recover_known_good_chat: bool,
    #[serde(default)]
    pub runtime_upgrade: bool,
    #[serde(default)]
    pub stop: bool,
    #[serde(default)]
    pub runtime_retirement: bool,
}

/// Versioned persisted Runtime capability advertisement. Missing and empty
/// advertisements support no controls; callers must never infer support from
/// placement, provider handles, or Runtime artifacts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "schema", content = "capabilities")]
pub enum RuntimeCapabilitiesEnvelope {
    #[serde(rename = "runtime_capabilities.v1")]
    V1(RuntimeCapabilitiesV1),
}

impl RuntimeCapabilitiesEnvelope {
    pub const fn v1(&self) -> &RuntimeCapabilitiesV1 {
        match self {
            Self::V1(capabilities) => capabilities,
        }
    }

    pub const fn supports(&self, kind: RuntimeControlKind) -> bool {
        let capabilities = self.v1();
        match kind {
            RuntimeControlKind::Restart => capabilities.restart,
            RuntimeControlKind::RecoverKnownGoodChatRuntime => capabilities.recover_known_good_chat,
            RuntimeControlKind::Upgrade => capabilities.runtime_upgrade,
            RuntimeControlKind::Stop => capabilities.stop,
            RuntimeControlKind::Destroy => capabilities.runtime_retirement,
        }
    }

    pub const fn supports_any_control(&self) -> bool {
        let capabilities = self.v1();
        capabilities.restart
            || capabilities.recover_known_good_chat
            || capabilities.runtime_upgrade
            || capabilities.stop
            || capabilities.runtime_retirement
    }
}

/// Preserve an explicit Core backfill when an N-1 worker omits the new field,
/// but never allow a current worker to change its advertisement between
/// registration retries or final completion.
pub(crate) fn merge_runtime_capabilities(
    existing: Option<&AgentRuntime>,
    incoming: Option<RuntimeCapabilitiesEnvelope>,
) -> CoreResult<Option<RuntimeCapabilitiesEnvelope>> {
    let current = existing.and_then(|runtime| runtime.runtime_capabilities.clone());
    match (current, incoming) {
        (Some(current), Some(incoming)) if current != incoming => {
            Err(CoreError::RuntimeCapabilitiesMismatch)
        }
        (Some(current), _) => Ok(Some(current)),
        (None, incoming) => Ok(incoming),
    }
}

/// Bound worker claims to product authority this Core generation has actually
/// accepted. A route-scoped worker credential is not permission to expose a
/// misleading recovery control or a destructive retirement transition.
pub(crate) fn validate_runtime_capabilities_policy(
    capabilities: Option<&RuntimeCapabilitiesEnvelope>,
    placement: Option<RuntimePlacement>,
) -> CoreResult<()> {
    let Some(capabilities) = capabilities else {
        return Ok(());
    };
    let capabilities = capabilities.v1();
    if (capabilities.recover_known_good_chat || capabilities.runtime_retirement)
        && placement.is_none_or(|placement| placement.runner_class != RunnerClass::Kata)
    {
        return Err(CoreError::RuntimeCapabilitiesNotAuthorized);
    }
    if capabilities.runtime_upgrade
        && placement.is_none_or(|placement| placement.runner_class != RunnerClass::Kata)
    {
        return Err(CoreError::RuntimeCapabilitiesNotAuthorized);
    }
    Ok(())
}

pub(crate) fn validate_runtime_capabilities_artifact_policy(
    capabilities: Option<&RuntimeCapabilitiesEnvelope>,
    placement: Option<RuntimePlacement>,
    artifact: &RuntimeArtifact,
) -> CoreResult<()> {
    validate_runtime_capabilities_policy(capabilities, placement)?;
    if capabilities.is_some_and(|capabilities| capabilities.v1().recover_known_good_chat)
        && !artifact.recover_known_good_chat
    {
        return Err(CoreError::RuntimeCapabilitiesNotAuthorized);
    }
    Ok(())
}

pub(crate) fn bound_runtime_capabilities_to_artifact(
    capabilities: Option<RuntimeCapabilitiesEnvelope>,
    artifact: &RuntimeArtifact,
) -> Option<RuntimeCapabilitiesEnvelope> {
    capabilities.map(|mut envelope| {
        let RuntimeCapabilitiesEnvelope::V1(capabilities) = &mut envelope;
        capabilities.recover_known_good_chat &= artifact.recover_known_good_chat;
        envelope
    })
}
