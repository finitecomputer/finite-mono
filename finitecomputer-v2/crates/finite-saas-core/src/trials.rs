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
