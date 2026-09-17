use super::*;

#[derive(Debug)]
pub(super) struct ApiError {
    status: StatusCode,
    message: String,
    /// Set only for internal errors we logged server-side; echoed back so a
    /// user can quote it in a support request and we can grep it out of the logs.
    correlation_id: Option<String>,
}

impl ApiError {
    pub(super) fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
            correlation_id: None,
        }
    }

    pub(super) fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
            correlation_id: None,
        }
    }

    pub(super) fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
            correlation_id: None,
        }
    }

    pub(super) fn service_unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: message.into(),
            correlation_id: None,
        }
    }

    pub(super) fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
            correlation_id: None,
        }
    }

    pub(super) fn payment_required(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::PAYMENT_REQUIRED,
            message: message.into(),
            correlation_id: None,
        }
    }

    /// A generic 500 that has already been logged server-side under
    /// `correlation_id`. The user never sees the underlying store detail.
    fn internal(correlation_id: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "internal error".to_string(),
            correlation_id: Some(correlation_id),
        }
    }
}

/// Monotonic, restart-derived correlation id for internal errors. Avoids random
/// UUIDs (which are constrained in some deploys); a process-start-relative nanos
/// value hashed with a per-process counter is unique enough to grep for and
/// quote, and mirrors the sha256-derived id style used in `lib.rs`.
fn next_correlation_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(seq.to_le_bytes());
    let digest = hasher.finalize();
    let hex = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("cid_{hex}")
}

impl From<CoreError> for ApiError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::MissingSourceHostId
            | CoreError::InvalidSourceHostId
            | CoreError::MissingAgentDisplayName
            | CoreError::MissingAgentCreationIdempotencyKey
            | CoreError::MissingLaunchCode
            | CoreError::InvalidLaunchCode
            | CoreError::MissingLaunchCodeBatchName
            | CoreError::InvalidLaunchCodeBatchName
            | CoreError::InvalidLaunchCodeBatchSize
            | CoreError::InvalidLaunchCodeBatchExpiry
            | CoreError::MissingStripeCustomerId
            | CoreError::MissingStripeSubscriptionId
            | CoreError::InvalidBillingSubscriptionStatus
            | CoreError::MissingAgentCreationRunnerId
            | CoreError::MissingAgentCreationLeaseToken
            | CoreError::InvalidAgentCreationLeaseDuration
            | CoreError::InvalidOwnerChatAccountId
            | CoreError::MissingSourceMachineId
            | CoreError::MissingRuntimeArtifactId
            | CoreError::MissingRuntimeArtifactReference
            | CoreError::MissingRuntimeArtifactVersionLabel
            | CoreError::MissingRuntimeArtifactStateSchemaVersion
            | CoreError::MissingFinitePrivateApiKey
            | CoreError::MissingStripeStandardPriceId
            | CoreError::StripeSubscriptionPriceMismatch
            | CoreError::InvalidFinitePrivateUsageEstimate
            | CoreError::MissingAgentCreationFailureMessage
            | CoreError::MissingRuntimeControlFailureMessage
            | CoreError::InvalidProviderOperationCorrelation
            | CoreError::InvalidProviderOperationFacts
            | CoreError::MissingAgentRuntimeId
            | CoreError::InvalidRuntimeHealthReport
            | CoreError::InvalidTimestamp => Self {
                status: StatusCode::BAD_REQUEST,
                message: error.to_string(),
                correlation_id: None,
            },
            CoreError::AgentCreationRequestNotFound
            | CoreError::RuntimeArtifactNotFound
            | CoreError::ProjectNotFound
            | CoreError::ProjectRuntimeNotFound
            | CoreError::RuntimeControlRequestNotFound
            | CoreError::BillingAccountNotFound
            | CoreError::FinitePrivateGrantNotFound
            | CoreError::FinitePrivateLimitProfileNotFound
            | CoreError::FinitePrivateReservationNotFound
            | CoreError::LaunchCodeBatchNotFound => Self::not_found(error.to_string()),
            CoreError::InvalidFinitePrivateApiKey => Self::unauthorized(error.to_string()),
            CoreError::BillingRequired => Self::payment_required(error.to_string()),
            CoreError::HostingTierNotAuthorized => Self::forbidden(error.to_string()),
            CoreError::AccountEmailChangeConflict
            | CoreError::AgentCreationEntitlementExhausted
            | CoreError::AgentCreationRequestUnavailable
            | CoreError::AgentCreationRequestLeaseConflict
            | CoreError::AgentCreationRequestNotLaunching
            | CoreError::AgentCreationRequestNotCancellable
            | CoreError::ProviderOperationIdentityMismatch
            | CoreError::ProviderOperationTransitionConflict
            | CoreError::ProviderOperationBoundaryNotReached
            | CoreError::RuntimeArtifactNotPromoted
            | CoreError::RuntimeArtifactRetired
            | CoreError::RuntimeArtifactImmutable
            | CoreError::RuntimeCapabilitiesMismatch
            | CoreError::RuntimeCapabilitiesNotAuthorized
            | CoreError::RuntimeRestartUnsupported
            | CoreError::RuntimeControlUnsupported
            | CoreError::RuntimeUpgradeUnsupported
            | CoreError::RuntimeUpgradeNotEnabled
            | CoreError::RuntimeUpgradeStateSchemaIncompatible
            | CoreError::RuntimeUpgradeTargetConflict
            | CoreError::RuntimeControlOperationConflict
            | CoreError::RuntimeHealthReportPrincipalMismatch
            | CoreError::RuntimeUpgradeCompletionMismatch
            | CoreError::RuntimeRetirementSnapshotMismatch
            | CoreError::RuntimeRetirementSnapshotConflict
            | CoreError::RuntimeRetirementNotEnabled
            | CoreError::RuntimeControlRequestNotLaunching
            | CoreError::RuntimeControlRequestLeaseConflict
            | CoreError::FinitePrivateGrantNotActive
            | CoreError::FinitePrivateReservationAlreadySettled
            | CoreError::StripeCustomerConflict => Self::conflict(error.to_string()),
            // A store/DB failure with structured detail. Log the FULL detail
            // server-side (SQLSTATE code, constraint, table, column, DETAIL)
            // under a correlation id, then hand the user a generic 500 carrying
            // only that id. This is the arm whose absence turned "there is no
            // unique constraint matching the ON CONFLICT" into a bare "db error"
            // in the user's browser.
            CoreError::Database(detail) => {
                let correlation_id = next_correlation_id();
                tracing::error!(
                    correlation_id = %correlation_id,
                    code = detail.code.as_deref().unwrap_or("-"),
                    constraint = detail.constraint.as_deref().unwrap_or("-"),
                    table = detail.table.as_deref().unwrap_or("-"),
                    column = detail.column.as_deref().unwrap_or("-"),
                    detail = detail.detail.as_deref().unwrap_or("-"),
                    message = %detail.message,
                    "core store database error"
                );
                Self::internal(correlation_id)
            }
            // Remaining internal-ish variants (e.g. Store(String) invariants,
            // TimeFormat). Log under a correlation id and stay generic.
            other => {
                let correlation_id = next_correlation_id();
                tracing::error!(
                    correlation_id = %correlation_id,
                    error = %other,
                    "core internal error"
                );
                Self::internal(correlation_id)
            }
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        let correlation_id = next_correlation_id();
        tracing::error!(
            correlation_id = %correlation_id,
            error = %error,
            "core request failed"
        );
        Self::internal(correlation_id)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        let correlation_id = next_correlation_id();
        tracing::error!(
            correlation_id = %correlation_id,
            error = %error,
            "core request body deserialization failed"
        );
        Self::internal(correlation_id)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = match &self.correlation_id {
            Some(correlation_id) => json!({
                "error": self.message,
                "correlation_id": correlation_id,
            }),
            None => json!({
                "error": self.message,
            }),
        };
        (self.status, Json(body)).into_response()
    }
}
