//! Event trial contracts. Stripe owns the trial clock and payment state;
//! Core owns campaign capacity and durable seat attribution.
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTrialCampaign {
    pub name: String,
    pub seat_limit: i32,
    #[serde(default = "default_trial_days")]
    pub trial_days: i32,
}
fn default_trial_days() -> i32 {
    7
}

/// Absolute capacity, guarded by the operator's last observed limit.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IncreaseTrialCapacity {
    pub seat_limit: i32,
    pub expected_seat_limit: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialCampaign {
    pub id: String,
    pub name: String,
    pub seat_limit: i32,
    pub trial_days: i32,
    pub active: bool,
    pub reserved_seats: i64,
    pub redeemed_seats: i64,
    pub seats_remaining: i64,
    pub redemptions: Vec<TrialRedemption>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialRedemption {
    pub customer_org_id: String,
    pub owner_workos_user_id: Option<String>,
    pub state: String,
    pub redeemed_at: Option<String>,
    pub trial_access: Option<TrialAccess>,
}

// No Debug: the code is returned only at issuance.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuedTrialCampaign {
    pub id: String,
    pub code: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrialCodeRequest {
    pub code: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialOffer {
    pub campaign_id: String,
    pub trial_days: i32,
}

/// Session details are supplied only by the trusted Stripe adapter, after
/// creation and before exposing its URL to the customer.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReserveTrial {
    pub code: String,
    pub customer_org_id: String,
    pub stripe_customer_id: String,
    pub stripe_session_id: String,
    pub attempt_id: String,
    pub checkout_expires_at: i64,
    pub trial_days: i32,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialReservation {
    pub stripe_session_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpireTrial {
    pub stripe_session_id: String,
    pub stripe_customer_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrialAccess {
    pub blocked: bool,
    pub event_name: String,
    pub subscription_status: Option<String>,
    pub period_end: Option<String>,
}

// 16 independent base32 symbols preserve the previous code's 80 random bits.
// O/I/0/1 are absent; groups can be read aloud. Plaintext is returned only once.
const TRIAL_CODE_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

pub(crate) fn generate_trial_code() -> crate::CoreResult<String> {
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|error| {
        crate::CoreError::Store(format!("failed to generate trial code: {error}"))
    })?;
    let compact: String = random
        .iter()
        .map(|byte| TRIAL_CODE_ALPHABET[(byte & 31) as usize] as char)
        .collect();
    Ok(group_trial_code(&compact))
}

pub(crate) fn hash_trial_code(value: &str) -> crate::CoreResult<String> {
    let compact: String = value
        .chars()
        .filter(|c| *c != '-' && !c.is_ascii_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if compact.len() == 16
        && compact
            .bytes()
            .all(|byte| TRIAL_CODE_ALPHABET.contains(&byte))
    {
        crate::launch_codes::hash_launch_code(&group_trial_code(&compact))
    } else {
        // Existing trial_<hex> codes retain their exact, case-sensitive hashes.
        crate::launch_codes::hash_launch_code(value)
    }
}

fn group_trial_code(compact: &str) -> String {
    compact
        .as_bytes()
        .chunks(4)
        .map(|group| std::str::from_utf8(group).expect("ASCII trial code"))
        .collect::<Vec<_>>()
        .join("-")
}
