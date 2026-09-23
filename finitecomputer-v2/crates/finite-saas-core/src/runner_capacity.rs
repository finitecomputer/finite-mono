//! Runner admission policy and atomic in-flight capacity reservations.

use crate::{
    AgentCreationRequest, CoreError, CoreResult, RunnerClass, RuntimeCapabilitiesEnvelope,
    RuntimeControlKind, RuntimePlacement,
};
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunnerLeaseCapacity {
    #[serde(default)]
    pub draining: bool,
    #[serde(default)]
    pub max_sandbox_count: Option<u32>,
    #[serde(default)]
    pub active_sandbox_count: Option<u32>,
    #[serde(default)]
    pub available_memory_bytes: Option<u64>,
    /// Adapter classes this worker can actually reconcile. Empty claims no
    /// creation or lifecycle work. An omitted capacity object remains the
    /// bounded N-1 compatibility path for an old worker.
    #[serde(default)]
    pub runner_classes: Vec<RunnerClass>,
    /// Exact control operations this worker can reconcile. Omitted or an
    /// all-false envelope supports no lifecycle leases.
    #[serde(default)]
    pub runtime_capabilities: Option<RuntimeCapabilitiesEnvelope>,
}

impl RunnerLeaseCapacity {
    /// Phala provider inventory can lag an accepted paid provision. Core must
    /// therefore reserve and count the in-flight creation atomically instead
    /// of letting the worker make a second, racy capacity decision.
    pub fn requires_core_in_flight_reservation(&self) -> bool {
        self.runner_classes.as_slice() == [RunnerClass::Phala]
    }

    pub fn validate_runtime_capability_policy(&self) -> CoreResult<()> {
        let Some(capabilities) = self.runtime_capabilities.as_ref() else {
            return Ok(());
        };
        let capabilities = capabilities.v1();
        if (capabilities.recover_known_good_chat || capabilities.runtime_retirement)
            && (self.runner_classes.is_empty()
                || self
                    .runner_classes
                    .iter()
                    .any(|runner_class| *runner_class != RunnerClass::Kata))
        {
            return Err(CoreError::RuntimeCapabilitiesNotAuthorized);
        }
        if capabilities.runtime_upgrade
            && self.runner_classes.iter().any(|runner_class| {
                !matches!(runner_class, RunnerClass::Kata | RunnerClass::Substrate)
            })
        {
            return Err(CoreError::RuntimeCapabilitiesNotAuthorized);
        }
        Ok(())
    }

    pub fn accepts_runtime_control(&self) -> bool {
        !self.runner_classes.is_empty()
            && self
                .runtime_capabilities
                .as_ref()
                .is_some_and(RuntimeCapabilitiesEnvelope::supports_any_control)
    }

    pub fn accepts_agent_creation(&self) -> bool {
        !self.runner_classes.is_empty()
            && !self.draining
            && (self.requires_core_in_flight_reservation() || !self.sandbox_limit_reached())
    }

    pub fn supports_runner_class(&self, runner_class: RunnerClass) -> bool {
        self.runner_classes.contains(&runner_class)
    }

    pub fn supports_runtime_control(&self, kind: RuntimeControlKind) -> bool {
        self.runtime_capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.supports(kind))
    }

    pub fn agent_creation_rejection_reason(&self) -> Option<&'static str> {
        if self.runner_classes.is_empty() {
            Some("runner advertises no classes")
        } else if self.draining {
            Some("runner is draining")
        } else if !self.requires_core_in_flight_reservation() && self.sandbox_limit_reached() {
            Some("runner sandbox capacity is full")
        } else {
            None
        }
    }

    fn sandbox_limit_reached(&self) -> bool {
        match (self.active_sandbox_count, self.max_sandbox_count) {
            (Some(active), Some(max)) => active >= max,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct InFlightCapacityBounds {
    pub(crate) runner_class: RunnerClass,
    pub(crate) provider_inventory_count: u32,
    pub(crate) max_sandbox_count: u32,
}

pub(crate) fn in_flight_capacity_bounds(
    capacity: &RunnerLeaseCapacity,
) -> CoreResult<Option<InFlightCapacityBounds>> {
    if !capacity.requires_core_in_flight_reservation() {
        return Ok(None);
    }
    let provider_inventory_count = capacity
        .active_sandbox_count
        .ok_or(CoreError::InvalidInFlightCapacityReservation)?;
    let max_sandbox_count = capacity
        .max_sandbox_count
        .filter(|maximum| *maximum > 0)
        .ok_or(CoreError::InvalidInFlightCapacityReservation)?;
    if provider_inventory_count > max_sandbox_count {
        return Err(CoreError::InvalidInFlightCapacityReservation);
    }
    Ok(Some(InFlightCapacityBounds {
        runner_class: RunnerClass::Phala,
        provider_inventory_count,
        max_sandbox_count,
    }))
}

pub(crate) fn in_flight_capacity_reservation(
    request: &AgentCreationRequest,
    placement: Option<RuntimePlacement>,
    capacity: InFlightCapacityBounds,
    core_in_flight_count: u32,
) -> CoreResult<InFlightCapacityReservationEnvelope> {
    let placement = placement.ok_or(CoreError::InvalidInFlightCapacityReservation)?;
    if request.runner_class != capacity.runner_class
        || placement.runner_class != capacity.runner_class
        || core_in_flight_count == 0
        || core_in_flight_count > capacity.max_sandbox_count
    {
        return Err(CoreError::InvalidInFlightCapacityReservation);
    }
    Ok(InFlightCapacityReservationEnvelope::V1(
        InFlightCapacityReservationV1 {
            request_id: request.id.clone(),
            placement,
            provider_inventory_count: capacity.provider_inventory_count,
            core_in_flight_count,
            max_sandbox_count: capacity.max_sandbox_count,
        },
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "schema", content = "reservation")]
pub enum InFlightCapacityReservationEnvelope {
    #[serde(rename = "in_flight_capacity_reservation.v1")]
    V1(InFlightCapacityReservationV1),
}

impl InFlightCapacityReservationEnvelope {
    pub const fn v1(&self) -> &InFlightCapacityReservationV1 {
        match self {
            Self::V1(reservation) => reservation,
        }
    }
}

/// Core's atomic acknowledgement that one creation request owns an in-flight
/// provider-capacity slot. `provider_inventory_count` is the exact count the
/// Runner submitted with its lease request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InFlightCapacityReservationV1 {
    pub request_id: String,
    pub placement: RuntimePlacement,
    pub provider_inventory_count: u32,
    pub core_in_flight_count: u32,
    pub max_sandbox_count: u32,
}
