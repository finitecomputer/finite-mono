//! Finite Private grants, credentials, reservations, and usage-window policy.

use crate::{CoreError, CoreResult, id_from_parts, parse_time, trim_to_option, wire_enum};
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::Duration;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub(crate) const DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE: &str = "finite-private-generous-v2";

pub const FINITE_PRIVATE_5X_LIMIT_PROFILE: &str = "finite-private-generous-5x-v1";

pub(crate) const DEFAULT_FINITE_PRIVATE_BURST_WINDOW_SECONDS: i64 = 5 * 60 * 60;

pub(crate) const DEFAULT_FINITE_PRIVATE_BURST_LIMIT_UNITS: i64 = 200_000_000;

pub(crate) const FINITE_PRIVATE_5X_BURST_LIMIT_UNITS: i64 = 500_000_000;

pub(crate) const DEFAULT_FINITE_PRIVATE_WEEKLY_LIMIT_UNITS: Option<i64> = None;

pub(crate) const FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS: i64 = 7 * 24 * 60 * 60;

wire_enum! {
    FinitePrivateGrantStatus {
    Active => "active",
    Revoked => "revoked",
    }
    parse: parse_finite_private_grant_status
}

wire_enum! {
    FinitePrivateApiKeyStatus {
    Active => "active",
    Revoked => "revoked",
    }
    parse: parse_finite_private_api_key_status
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinitePrivateReservationStatus {
    Reserved,
    Settled,
    Denied,
}

wire_enum! {
    FinitePrivateSettlementKind {
    Actual => "actual",
    Estimate => "estimate",
    }
    parse: parse_finite_private_settlement_kind
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateLimitProfile {
    pub id: String,
    pub burst_window_seconds: i64,
    pub burst_limit_units: i64,
    pub weekly_limit_units: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateGrant {
    pub id: String,
    pub user_id: String,
    pub limit_profile_id: String,
    pub status: FinitePrivateGrantStatus,
    pub current_window_started_at: Option<String>,
    pub current_window_used_units: i64,
    #[serde(default)]
    pub burst_window_epoch: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateApiKey {
    pub id: String,
    pub grant_id: String,
    pub project_id: Option<String>,
    pub agent_runtime_id: Option<String>,
    pub key_hash: String,
    pub status: FinitePrivateApiKeyStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateAdminAuditEvent {
    pub id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub grant_id: Option<String>,
    pub api_key_id: Option<String>,
    pub actor: String,
    pub metadata: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinitePrivateAdminProject {
    pub id: String,
    pub display_name: String,
    pub agent_runtime_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinitePrivateAdminAccount {
    pub user_id: String,
    pub email: String,
    pub grant: FinitePrivateGrant,
    pub api_keys: Vec<FinitePrivateApiKey>,
    pub projects: Vec<FinitePrivateAdminProject>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinitePrivateAdminState {
    /// Account-centric operator view. The legacy flat arrays remain during the
    /// additive dashboard/Core rollout so mixed versions fail gracefully.
    #[serde(default)]
    pub accounts: Vec<FinitePrivateAdminAccount>,
    #[serde(default)]
    pub profiles: Vec<FinitePrivateLimitProfile>,
    pub grants: Vec<FinitePrivateGrant>,
    pub api_keys: Vec<FinitePrivateApiKey>,
    pub admin_audit_events: Vec<FinitePrivateAdminAuditEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateReservation {
    pub id: String,
    pub request_id: String,
    pub api_key_id: String,
    pub grant_id: String,
    pub endpoint: String,
    pub model: String,
    pub estimated_usage_units: i64,
    pub reserved_usage_units: i64,
    pub settled_usage_units: Option<i64>,
    pub settlement_kind: Option<FinitePrivateSettlementKind>,
    pub status: FinitePrivateReservationStatus,
    #[serde(default)]
    pub burst_window_epoch: i64,
    pub usage_formula_version: String,
    pub upstream_status: Option<i32>,
    pub upstream_error_class: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordFinitePrivateRequestDiagnosticInput {
    pub reservation_id: String,
    pub request_id: String,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub first_output_ms: Option<i64>,
    pub first_answer_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub termination_reason: String,
    pub measurement_quality: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateUsageDecision {
    pub decision: String,
    pub reservation_id: Option<String>,
    pub limit_profile: Option<String>,
    pub burst_limit_units: Option<i64>,
    pub burst_remaining_units: Option<i64>,
    pub burst_reset_at: Option<String>,
    pub weekly_limit_units: Option<i64>,
    pub weekly_remaining_units: Option<i64>,
    pub weekly_reset_at: Option<String>,
    pub error: Option<FinitePrivateUsageError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateUsageError {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: String,
    pub retry_after: Option<i64>,
    pub reset_at: Option<String>,
    pub dashboard_url: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinitePrivateUsageNotice {
    pub threshold_remaining_percent: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinitePrivateUsageStatus {
    pub burst_limit_units: i64,
    pub burst_used_units: i64,
    pub burst_remaining_units: i64,
    pub burst_reset_at: String,
    pub free_daily_reset_available: bool,
    pub free_daily_reset_available_again_at: String,
    pub notice: Option<FinitePrivateUsageNotice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinitePrivateDailyResetResult {
    pub performed: bool,
    pub status: FinitePrivateUsageStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApproveFinitePrivateGrantInput {
    pub verified_email: String,
    pub workos_user_id: Option<String>,
    pub limit_profile_id: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IssueFinitePrivateApiKeyInput {
    pub grant_id: String,
    pub raw_key: String,
    pub project_id: Option<String>,
    pub agent_runtime_id: Option<String>,
    pub now: Option<String>,
}

/// Approve a grant and issue its first API key as one unit.
///
/// The two steps must share a transaction: a caller that approves and then
/// issues separately can leave a grant with no key behind when the second step
/// fails, and cannot be previewed by `--dry-run` at all because the rolled-back
/// grant is invisible to the key issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IssueFinitePrivateFriendKeyInput {
    pub verified_email: String,
    pub workos_user_id: Option<String>,
    pub limit_profile_id: Option<String>,
    /// Raw key material generated by the caller; only its hash is stored.
    pub raw_key: String,
    pub project_id: Option<String>,
    pub agent_runtime_id: Option<String>,
    pub now: Option<String>,
}

/// Grant and key created together by [`IssueFinitePrivateFriendKeyInput`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssuedFinitePrivateFriendKey {
    pub grant: FinitePrivateGrant,
    pub api_key: FinitePrivateApiKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionFinitePrivateRuntimeKeyInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub source_host_id: Option<String>,
    pub source_machine_id: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionFinitePrivateRuntimeKeyResult {
    pub grant: FinitePrivateGrant,
    pub api_key: FinitePrivateApiKey,
    pub raw_api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RevokeFinitePrivateGrantInput {
    pub grant_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RevokeFinitePrivateApiKeyInput {
    pub key_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RotateFinitePrivateApiKeyInput {
    pub key_id: String,
    pub raw_key: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResetFinitePrivateUsageWindowInput {
    pub grant_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReserveFinitePrivateUsageInput {
    pub request_id: String,
    pub presented_api_key: String,
    pub endpoint: String,
    pub model: String,
    pub estimated_prompt_tokens: i64,
    pub estimated_completion_tokens: i64,
    pub estimated_usage_units: i64,
    pub usage_formula_version: String,
    pub dashboard_url: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SettleFinitePrivateReservationInput {
    pub reservation_id: String,
    pub request_id: String,
    pub settlement: FinitePrivateSettlementKind,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub usage_units: Option<i64>,
    pub usage_formula_version: String,
    pub upstream_status: Option<i32>,
    pub upstream_error_class: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettleFinitePrivateReservationResult {
    pub settled: bool,
    pub reservation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminIssueFinitePrivateFriendKeyInput {
    pub admin_verified_email: String,
    pub friend_email: String,
    pub limit_profile_id: Option<String>,
    /// Raw key material generated by the caller; only its hash is stored.
    pub raw_key: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdminIssuedFinitePrivateKey {
    pub grant: FinitePrivateGrant,
    pub api_key: FinitePrivateApiKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminRotateFinitePrivateApiKeyInput {
    pub admin_verified_email: String,
    pub key_id: String,
    /// Replacement raw key material generated by the caller; only its hash is stored.
    pub raw_key: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminRevokeFinitePrivateApiKeyInput {
    pub admin_verified_email: String,
    pub key_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminResetFinitePrivateUsageWindowInput {
    pub admin_verified_email: String,
    pub grant_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminAssignFinitePrivateLimitProfileInput {
    pub admin_verified_email: String,
    pub grant_id: String,
    pub limit_profile_id: String,
    pub now: Option<String>,
}

impl FinitePrivateReservationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Settled => "settled",
            Self::Denied => "denied",
        }
    }
}

pub fn parse_finite_private_reservation_status(
    value: &str,
) -> Option<FinitePrivateReservationStatus> {
    match value {
        "reserved" => Some(FinitePrivateReservationStatus::Reserved),
        "settled" => Some(FinitePrivateReservationStatus::Settled),
        "denied" => Some(FinitePrivateReservationStatus::Denied),
        _ => None,
    }
}

pub(crate) fn hash_finite_private_api_key(value: &str) -> CoreResult<String> {
    let token = trim_to_option(Some(value)).ok_or(CoreError::MissingFinitePrivateApiKey)?;
    let digest = Sha256::digest(token.as_bytes());
    Ok(digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
}

pub fn generate_finite_private_api_key() -> CoreResult<String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| {
        CoreError::Store(format!(
            "failed to generate Finite Private API key: {error}"
        ))
    })?;
    let mut key = String::with_capacity("fpk_live_".len() + bytes.len() * 2);
    key.push_str("fpk_live_");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut key, "{byte:02x}")
            .map_err(|error| CoreError::Store(format!("failed to render API key: {error}")))?;
    }
    Ok(key)
}

pub(crate) fn finite_private_grant_id_for_user(user_id: &str) -> String {
    id_from_parts("fp_grant", &[user_id])
}

pub(crate) fn finite_private_api_key_id_for(grant_id: &str, key_hash: &str) -> String {
    id_from_parts("fp_key", &[grant_id, key_hash])
}

pub(crate) fn finite_private_reservation_id_for(api_key_id: &str, request_id: &str) -> String {
    id_from_parts("fp_reservation", &[api_key_id, request_id])
}

pub(crate) fn finite_private_active_window(
    grant: &FinitePrivateGrant,
    profile: &FinitePrivateLimitProfile,
    now_time: OffsetDateTime,
) -> CoreResult<(String, i64, String)> {
    let current_start = grant
        .current_window_started_at
        .as_deref()
        .map(parse_time)
        .transpose()?;
    let window_start = match current_start {
        Some(start) if now_time < start + Duration::seconds(profile.burst_window_seconds) => start,
        _ => now_time,
    };
    let used_units = if current_start == Some(window_start) {
        grant.current_window_used_units
    } else {
        0
    };
    let reset_at =
        (window_start + Duration::seconds(profile.burst_window_seconds)).format(&Rfc3339)?;
    Ok((window_start.format(&Rfc3339)?, used_units, reset_at))
}

pub(crate) fn finite_private_begins_new_epoch(
    grant: &FinitePrivateGrant,
    projected_window_started_at: &str,
) -> CoreResult<bool> {
    let Some(current_window_started_at) = grant.current_window_started_at.as_deref() else {
        return Ok(false);
    };
    Ok(parse_time(current_window_started_at)? != parse_time(projected_window_started_at)?)
}

pub(crate) fn finite_private_window_reset_at(
    grant: &FinitePrivateGrant,
    profile: &FinitePrivateLimitProfile,
    now_time: OffsetDateTime,
) -> CoreResult<String> {
    let (_, _, reset_at) = finite_private_active_window(grant, profile, now_time)?;
    Ok(reset_at)
}

pub(crate) fn finite_private_allow_decision(
    reservation_id: String,
    profile: &FinitePrivateLimitProfile,
    burst_remaining_units: i64,
    burst_reset_at: String,
    weekly_remaining_units: Option<i64>,
    weekly_reset_at: Option<String>,
) -> FinitePrivateUsageDecision {
    FinitePrivateUsageDecision {
        decision: "allow".to_string(),
        reservation_id: Some(reservation_id),
        limit_profile: Some(profile.id.clone()),
        burst_limit_units: Some(profile.burst_limit_units),
        burst_remaining_units: Some(burst_remaining_units.max(0)),
        burst_reset_at: Some(burst_reset_at),
        weekly_limit_units: profile.weekly_limit_units,
        weekly_remaining_units: weekly_remaining_units.map(|remaining| remaining.max(0)),
        weekly_reset_at,
        error: None,
    }
}

pub(crate) fn finite_private_denial(
    request_id: String,
    dashboard_url: String,
    message: &str,
    code: &str,
    retry_after: Option<i64>,
    reset_at: Option<String>,
) -> FinitePrivateUsageDecision {
    FinitePrivateUsageDecision {
        decision: "deny".to_string(),
        reservation_id: None,
        limit_profile: None,
        burst_limit_units: None,
        burst_remaining_units: None,
        burst_reset_at: reset_at.clone(),
        weekly_limit_units: None,
        weekly_remaining_units: None,
        weekly_reset_at: reset_at.clone(),
        error: Some(FinitePrivateUsageError {
            message: message.to_string(),
            error_type: "usage_limit".to_string(),
            code: code.to_string(),
            retry_after,
            reset_at,
            dashboard_url,
            request_id,
        }),
    }
}

pub(crate) fn finite_private_limit_reached_message(
    window_label: &str,
    reset_at: &str,
    retry_after_seconds: i64,
) -> String {
    format!(
        "Finite Private {window_label} limit reached. Your usage resets at {reset_at} ({}).",
        finite_private_retry_after_label(retry_after_seconds)
    )
}

pub(crate) fn finite_private_retry_after_label(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds == 0 {
        return "resetting now".to_string();
    }
    let total_minutes = (seconds + 59) / 60;
    let days = total_minutes / (24 * 60);
    let hours = (total_minutes % (24 * 60)) / 60;
    let minutes = total_minutes % 60;
    if days > 0 && hours > 0 {
        format!("in {days}d {hours}h")
    } else if days > 0 {
        format!("in {days}d")
    } else if hours > 0 && minutes > 0 {
        format!("in {hours}h {minutes}m")
    } else if hours > 0 {
        format!("in {hours}h")
    } else {
        format!("in {minutes}m")
    }
}

pub(crate) fn finite_private_next_daily_reset_at(now: OffsetDateTime) -> CoreResult<String> {
    let next_midnight = (now.unix_timestamp().div_euclid(86_400) + 1) * 86_400;
    OffsetDateTime::from_unix_timestamp(next_midnight)
        .map_err(|_| CoreError::InvalidTimestamp)?
        .format(&Rfc3339)
        .map_err(CoreError::from)
}
