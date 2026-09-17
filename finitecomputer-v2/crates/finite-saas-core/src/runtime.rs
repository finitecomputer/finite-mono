//! Runtime placement, durable identity, and contact endpoints.

use crate::{
    CoreError, CoreResult, ProviderRuntimeHandleEnvelope, RuntimeCapabilitiesEnvelope,
    RuntimeControlKind, trim_to_option, wire_enum,
};
use serde::Deserialize;
use serde::Serialize;

wire_enum! {
/// A Runtime's lifecycle-latched summary. `Online`/`Offline` are the last
/// successful up-/down-bound lifecycle outcomes, `Stale` is a failed control,
/// and `Unknown` is registered-but-unconfirmed. Every completion that brings
/// compute up latches `Online` and clears the stored health report, so the
/// user-facing status (never this latch verbatim: see
/// `derive_runtime_summary_status`) reads `unknown` until the runner's
/// standing poller first reports on the new incarnation.
///
/// Parsing is forward-tolerant: an unrecognised string reads as `Unknown`
/// (registered-but-unconfirmed), so a reader one release behind survives a
/// newly added variant.
    RuntimeSummaryStatus {
    Online => "online",
    Offline => "offline",
    Stale => "stale",
    Unknown => "unknown",
    }
    parse: parse_runtime_summary_status
    fallback: Unknown
}

wire_enum! {
/// Customer-facing hosting promise. Provider placement remains a separate,
/// Core-owned fact and is never inferred from BillingClass.
    HostingTier {
    Standard => "standard",
    Confidential => "confidential",
    }
    parse: parse_hosting_tier
}

wire_enum! {
/// Provider-neutral minimum compute shape. Runner adapters translate this
/// closed value to a provider-specific size and verify the returned capacity.
    RuntimeResourceClass {
    Vcpu4Memory8Gib => "vcpu4_memory8_gib",
    Vcpu2Memory4Gib => "vcpu2_memory4_gib",
    }
    parse: parse_runtime_resource_class
}

wire_enum! {
/// Product placement choice stored with an agent creation request. Provider
/// vocabulary stops at the runner adapter; feature behavior does not branch on
/// this value.
    RunnerClass {
    LocalDocker => "local_docker",
    AppleContainer => "apple_container",
    Kata => "kata",
    Phala => "phala",
    Enclavia => "enclavia",
    }
    parse: parse_runner_class
}

/// Immutable placement resolved by Core. Replacement and recovery copy this
/// value rather than rerunning current product policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimePlacement {
    pub runner_class: RunnerClass,
    pub runtime_resource_class: RuntimeResourceClass,
}

impl RuntimePlacement {
    pub const fn for_hosting_tier(tier: HostingTier) -> Self {
        match tier {
            HostingTier::Standard => Self {
                runner_class: RunnerClass::Kata,
                runtime_resource_class: RuntimeResourceClass::Vcpu4Memory8Gib,
            },
            HostingTier::Confidential => Self {
                runner_class: RunnerClass::Phala,
                runtime_resource_class: RuntimeResourceClass::Vcpu2Memory4Gib,
            },
        }
    }

    /// Compatibility bridge for proven Kata/Phala rows written before the
    /// placement columns existed. Other experimental adapters have no durable
    /// resource-class fact, so callers must leave the expand fields null.
    pub const fn from_legacy_runner_class(runner_class: RunnerClass) -> Option<Self> {
        match runner_class {
            RunnerClass::Kata => Some(Self::for_hosting_tier(HostingTier::Standard)),
            RunnerClass::Phala => Some(Self::for_hosting_tier(HostingTier::Confidential)),
            RunnerClass::LocalDocker | RunnerClass::AppleContainer | RunnerClass::Enclavia => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostOwnedRuntimeFacts {
    pub display_name: String,
    pub hostname: Option<String>,
    pub runtime_host: String,
    pub runtime_status: RuntimeSummaryStatus,
    pub active_inference_profile: Option<String>,
    pub hermes_available: Option<bool>,
    pub published_app_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRuntime {
    pub id: String,
    pub project_id: String,
    pub source_host_id: String,
    pub source_machine_id: String,
    pub source_import_key: String,
    pub runtime_artifact_id: Option<String>,
    pub state_schema_version: Option<String>,
    #[serde(default)]
    pub placement: Option<RuntimePlacement>,
    #[serde(default)]
    pub provider_runtime_handle: Option<ProviderRuntimeHandleEnvelope>,
    #[serde(default)]
    pub provider_runtime_handle_history: Vec<ProviderRuntimeHandleEnvelope>,
    #[serde(default)]
    pub contact_endpoint: Option<String>,
    #[serde(default)]
    pub runtime_capabilities: Option<RuntimeCapabilitiesEnvelope>,
    pub host_facts: HostOwnedRuntimeFacts,
    pub created_at: String,
    pub updated_at: String,
}

impl AgentRuntime {
    /// Fail-closed capability gate for restart/stop/upgrade/etc. Note this is
    /// also what keeps legacy rows inert: runtimes imported by the abandoned
    /// 2026-07 import bridge (and any other row without a capabilities
    /// envelope) have `runtime_capabilities: NULL` and refuse every control.
    pub fn supports_runtime_control(&self, kind: RuntimeControlKind) -> bool {
        self.runtime_capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.supports(kind))
    }
}

pub(crate) fn normalize_runtime_contact_endpoint(
    value: Option<&str>,
) -> CoreResult<Option<String>> {
    let Some(value) = trim_to_option(value) else {
        return Ok(None);
    };
    let valid_scheme = value.starts_with("https://") || value.starts_with("http://");
    if !valid_scheme
        || value.len() > 2_048
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(CoreError::InvalidRuntimeContactEndpoint);
    }
    Ok(Some(value.trim_end_matches('/').to_string()))
}

pub(crate) fn runtime_upgrade_contact_endpoint(
    published_app_urls: &[String],
) -> CoreResult<String> {
    let mut contact_endpoint = None;
    for published_url in published_app_urls {
        let normalized = normalize_runtime_contact_endpoint(Some(published_url))?
            .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
        if !normalized.ends_with("/contact") {
            continue;
        }
        if contact_endpoint.replace(normalized).is_some() {
            return Err(CoreError::RuntimeUpgradeCompletionMismatch);
        }
    }
    contact_endpoint.ok_or(CoreError::RuntimeUpgradeCompletionMismatch)
}
