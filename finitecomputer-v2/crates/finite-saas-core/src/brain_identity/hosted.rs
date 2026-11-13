//! Load-only Hosted Device evidence. Registering a key creates neither Brain
//! participation nor a disclosure scope. The private writer requires the
//! dashboard credential and the Account's verified WorkOS bearer.

use super::*;

pub const HOSTED_OBSERVATION_VERSION: &str = "finite-core-hosted-account-observation-v1";
pub const HOSTED_OBSERVATION_PATH: &str = "/api/core/internal/v1/hosted-account-observations";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostedAccountObservationRequest {
    pub version: String,
    pub operation_id: String,
    pub observed_at: String,
    pub public_key_hex: String,
}

impl HostedAccountObservationRequest {
    pub fn validate(&self) -> Result<(), ObservationInputError> {
        if self.version != HOSTED_OBSERVATION_VERSION {
            return Err(ObservationInputError::Version);
        }
        if !valid_operation_id(&self.operation_id) {
            return Err(ObservationInputError::OperationId);
        }
        if crate::parse_time(&self.observed_at).is_err() {
            return Err(ObservationInputError::ObservedAt);
        }
        if !valid_public_key_hex(&self.public_key_hex) {
            return Err(ObservationInputError::PublicKey);
        }
        Ok(())
    }

    pub(crate) fn payload_sha256(&self) -> String {
        let canonical = serde_json::to_vec(self).expect("hosted observation serializes");
        Sha256::digest(canonical)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
