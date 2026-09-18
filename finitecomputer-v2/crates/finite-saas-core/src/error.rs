//! Core domain failures and structured store-error details.

use crate::OffboardingPhase;

/// Structured detail captured from a failed store operation. The full detail
/// is meant for server-side logs only; the user-facing surface stays generic.
///
/// For Postgres failures the fields mirror `tokio_postgres::error::DbError`
/// (`as_db_error`): SQLSTATE code, violated constraint, table, column, and the
/// server `DETAIL` line. These are exactly the fields that were being discarded
/// by the old `error.to_string()` == "db error" path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreErrorDetail {
    /// Full server-side message (Postgres primary message or serde error).
    pub message: String,
    /// SQLSTATE code, e.g. "23505" for a unique violation.
    pub code: Option<String>,
    /// Name of the violated constraint, when the failure is a constraint error.
    pub constraint: Option<String>,
    /// Table the failure references.
    pub table: Option<String>,
    /// Column the failure references.
    pub column: Option<String>,
    /// Postgres `DETAIL` line (e.g. "Key (customer_org_id)=(...) already exists.").
    pub detail: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("verified email is required")]
    MissingVerifiedEmail,
    #[error("WorkOS user id is required")]
    MissingWorkosUserId,
    #[error("source host id is required")]
    MissingSourceHostId,
    #[error("source host id must contain only lowercase letters, digits, and hyphens")]
    InvalidSourceHostId,
    #[error("agent display name is required")]
    MissingAgentDisplayName,
    #[error("agent creation idempotency key is required")]
    MissingAgentCreationIdempotencyKey,
    #[error("agent profile picture URL is invalid")]
    InvalidAgentProfilePictureUrl,
    #[error("owner chat account id must be 64 lowercase hex characters")]
    InvalidOwnerChatAccountId,
    #[error("runtime contact endpoint is invalid")]
    InvalidRuntimeContactEndpoint,
    #[error("agent runtime id is required")]
    MissingAgentRuntimeId,
    #[error("runtime health report is invalid or out of bounds")]
    InvalidRuntimeHealthReport,
    #[error("runtime health report presents a different Agent Principal than the one on record")]
    RuntimeHealthReportPrincipalMismatch,
    #[error("provider runtime handle does not match the persisted placement")]
    ProviderRuntimeHandlePlacementMismatch,
    #[error("provider operation correlation id is required or invalid")]
    InvalidProviderOperationCorrelation,
    #[error("provider operation facts are invalid or contain secret material")]
    InvalidProviderOperationFacts,
    #[error("Runner capacity could not produce a safe in-flight reservation")]
    InvalidInFlightCapacityReservation,
    #[error("provider operation identity does not match the creation request")]
    ProviderOperationIdentityMismatch,
    #[error("provider operation transition is out of order")]
    ProviderOperationTransitionConflict,
    #[error("provider operation boundary has not been reached")]
    ProviderOperationBoundaryNotReached,
    #[error("runtime spec does not match its persisted project, placement, runtime, or artifact")]
    RuntimeSpecMismatch,
    #[error("runtime capability advertisement changed during creation")]
    RuntimeCapabilitiesMismatch,
    #[error("runtime capability advertisement exceeds current placement policy")]
    RuntimeCapabilitiesNotAuthorized,
    #[error("no promoted runtime artifact is available for a new runtime")]
    RuntimeArtifactUnavailable,
    #[error("hosting tier is required before creating an agent")]
    MissingHostingTier,
    #[error("selected hosting tier is not authorized by this account or Launch Code")]
    HostingTierNotAuthorized,
    #[error("launch code is required")]
    MissingLaunchCode,
    #[error("launch code is invalid")]
    InvalidLaunchCode,
    #[error("launch code batch name is required")]
    MissingLaunchCodeBatchName,
    #[error("launch code batch name is invalid")]
    InvalidLaunchCodeBatchName,
    #[error("launch code batch size is invalid")]
    InvalidLaunchCodeBatchSize,
    #[error("launch code batch expiry must be between one hour and 30 days")]
    InvalidLaunchCodeBatchExpiry,
    #[error("launch code batch was not found")]
    LaunchCodeBatchNotFound,
    #[error("agent creation entitlement is exhausted")]
    AgentCreationEntitlementExhausted,
    #[error("billing is required before creating an agent")]
    BillingRequired,
    #[error("agent creation runner id is required")]
    MissingAgentCreationRunnerId,
    #[error("agent creation lease token is required")]
    MissingAgentCreationLeaseToken,
    #[error("agent creation lease duration is invalid")]
    InvalidAgentCreationLeaseDuration,
    #[error("agent creation request is not available")]
    AgentCreationRequestUnavailable,
    #[error("agent creation request was not found")]
    AgentCreationRequestNotFound,
    #[error("agent creation request lease does not match")]
    AgentCreationRequestLeaseConflict,
    #[error("agent creation request is not launching")]
    AgentCreationRequestNotLaunching,
    #[error("agent creation request cannot be cancelled")]
    AgentCreationRequestNotCancellable,
    #[error("source machine id is required")]
    MissingSourceMachineId,
    #[error("runtime artifact id is required")]
    MissingRuntimeArtifactId,
    #[error("runtime artifact reference is required")]
    MissingRuntimeArtifactReference,
    #[error("runtime artifact version label is required")]
    MissingRuntimeArtifactVersionLabel,
    #[error("runtime artifact state schema version is required")]
    MissingRuntimeArtifactStateSchemaVersion,
    #[error("runtime artifact was not found")]
    RuntimeArtifactNotFound,
    #[error("runtime artifact is not promoted")]
    RuntimeArtifactNotPromoted,
    #[error("runtime artifact is retired")]
    RuntimeArtifactRetired,
    #[error("a promoted or runtime-referenced artifact is immutable")]
    RuntimeArtifactImmutable,
    #[error("project was not found")]
    ProjectNotFound,
    #[error("project runtime was not found")]
    ProjectRuntimeNotFound,
    #[error("runtime restart is not supported for this runtime")]
    RuntimeRestartUnsupported,
    #[error("the requested runtime control is not supported for this runtime")]
    RuntimeControlUnsupported,
    #[error("runtime upgrade is supported only for Kata runtimes created by Core")]
    RuntimeUpgradeUnsupported,
    #[error("runtime upgrades are not enabled for this Core generation")]
    RuntimeUpgradeNotEnabled,
    #[error("runtime upgrade target is incompatible with the mounted state schema")]
    RuntimeUpgradeStateSchemaIncompatible,
    #[error("a different runtime upgrade is already in progress")]
    RuntimeUpgradeTargetConflict,
    #[error("another runtime control operation is already in progress")]
    RuntimeControlOperationConflict,
    #[error("runtime upgrade completion did not match the requested artifact")]
    RuntimeUpgradeCompletionMismatch,
    #[error("runtime retirement snapshot receipt did not match the leased runtime")]
    RuntimeRetirementSnapshotMismatch,
    #[error("runtime retirement snapshot receipt conflicts with the stored receipt")]
    RuntimeRetirementSnapshotConflict,
    #[error("runtime retirement is not enabled for this Core generation")]
    RuntimeRetirementNotEnabled,
    #[error("all unrecoverable runtime archive acknowledgements are required")]
    UnrecoverableRuntimeArchiveAcknowledgementRequired,
    #[error("unrecoverable runtime archive owner does not match")]
    UnrecoverableRuntimeArchiveOwnerMismatch,
    #[error("runtime has provider metadata and cannot use unrecoverable legacy archival")]
    UnrecoverableRuntimeArchiveProviderMetadataPresent,
    #[error("the compute-absent acknowledgement is required for retired runtime offboarding")]
    RetiredRuntimeOffboardAcknowledgementRequired,
    #[error("retired runtime offboard owner does not match")]
    RetiredRuntimeOffboardOwnerMismatch,
    #[error("a verified runtime retirement receipt is required for retired runtime offboarding")]
    RetiredRuntimeOffboardReceiptMissing,
    #[error("runtime offboarding phase cannot regress from {current} to {attempted}")]
    OffboardingPhaseRegression {
        current: OffboardingPhase,
        attempted: OffboardingPhase,
    },
    #[error(
        "runtime offboarding is already at {phase}; resume it with runtime-offboard-retired-exact instead of enqueueing a new destroy"
    )]
    RuntimeOffboardingResumeRequired { phase: OffboardingPhase },
    #[error("runtime control request was not found")]
    RuntimeControlRequestNotFound,
    #[error("runtime control request is not in the launching phase")]
    RuntimeControlRequestNotLaunching,
    #[error("runtime control request lease does not match")]
    RuntimeControlRequestLeaseConflict,
    #[error("runtime control request failure message is required")]
    MissingRuntimeControlFailureMessage,
    #[error("finite private api key is required")]
    MissingFinitePrivateApiKey,
    #[error("finite private api key is invalid")]
    InvalidFinitePrivateApiKey,
    #[error("finite private grant was not found")]
    FinitePrivateGrantNotFound,
    #[error("finite private grant is not active")]
    FinitePrivateGrantNotActive,
    #[error("finite private limit profile was not found")]
    FinitePrivateLimitProfileNotFound,
    #[error("finite private reservation was not found")]
    FinitePrivateReservationNotFound,
    #[error("finite private reservation is already settled")]
    FinitePrivateReservationAlreadySettled,
    #[error("Stripe customer id is required")]
    MissingStripeCustomerId,
    #[error("Stripe subscription id is required")]
    MissingStripeSubscriptionId,
    #[error("Stripe standard price id is required before granting billing entitlement")]
    MissingStripeStandardPriceId,
    #[error("Stripe subscription price is not eligible for hosted agents")]
    StripeSubscriptionPriceMismatch,
    #[error("Stripe customer is already linked to a different org")]
    StripeCustomerConflict,
    #[error("billing account was not found")]
    BillingAccountNotFound,
    #[error("billing subscription status is invalid")]
    InvalidBillingSubscriptionStatus,
    #[error("finite private usage estimate is invalid")]
    InvalidFinitePrivateUsageEstimate,
    #[error("agent creation failure message is required")]
    MissingAgentCreationFailureMessage,
    #[error("timestamp is invalid")]
    InvalidTimestamp,
    #[error("WorkOS user is already linked to a different email")]
    WorkosUserConflict,
    #[error(
        "account email change conflicts with current identity, destination, or operation state"
    )]
    AccountEmailChangeConflict,
    #[error("failed to format current time")]
    TimeFormat(#[from] time::error::Format),
    #[error("store error: {0}")]
    Store(String),
    /// A failed store operation with structured, log-only detail. The `Display`
    /// impl is intentionally generic ("database error") so the detail never
    /// leaks into a user-facing response; it is logged server-side in the
    /// `ApiError` conversion behind a correlation id.
    #[error("database error")]
    Database(Box<StoreErrorDetail>),
}

pub type CoreResult<T> = Result<T, CoreError>;
