//! The single owner of every Stripe billing concern in Core.
//!
//! # Where Stripe events enter the system
//!
//! Stripe webhooks are verified DASHBOARD-side today
//! (`apps/dashboard/src/app/api/stripe/webhook/route.ts`): the Next.js route
//! verifies the `stripe-signature` header with the Stripe SDK, re-fetches the
//! subscription, extracts the standard-price item, and POSTs the normalized
//! fields to Core's service-authed `/api/core/billing/stripe/subscription`
//! endpoint. Core never sees raw Stripe webhook JSON — the DTOs here are the
//! contract that endpoint accepts. Moving signature verification server-side
//! (so Core could consume typed `async-stripe` events directly) is a possible
//! follow-up; it changes the deployment trust boundary and is deliberately
//! out of scope for the consolidation that created this module.
//!
//! # What lives here (and nowhere else)
//!
//! - The Stripe wire vocabulary: [`BillingSubscriptionStatus`] (Stripe
//!   subscription status strings) and the camelCase JSON DTOs the dashboard
//!   posts ([`LinkStripeCustomerRequest`], [`SyncStripeSubscriptionRequest`]).
//! - The durable billing-account row model ([`CustomerBillingAccount`]) and
//!   its Postgres reads/writes, including customer linking.
//! - The two webhook safety guards, whose semantics must not change:
//!   - the event-ordering guard ([`stripe_event_is_stale`]): for the SAME
//!     subscription, a delivery whose Stripe `event.created` predates the last
//!     APPLIED one is dropped, so a stale webhook cannot resurrect a canceled
//!     subscription;
//!   - the subscription-replacement guard
//!     ([`should_replace_stripe_subscription`]): a new subscription id may
//!     only replace a terminal (canceled/incomplete_expired) or absent one,
//!     and only with an entitlement-granting status.
//!
//! `store.rs` opens the transaction and delegates here; `api.rs` maps HTTP to
//! the DTOs. No Stripe JSON field name appears outside this module.

use crate::store::{
    customer_org_exists, ensure_personal_org_row, ensure_standard_agent_creation_entitlement_row,
    optional_hosting_tier_column, store_error, upsert_linked_user,
};
use crate::{
    BillingClass, CoreError, CoreResult, HostingTier, current_time_iso, normalize_owner_email,
    trim_to_option, wire_enum,
};
use serde::{Deserialize, Serialize};
use tokio_postgres::{GenericClient, Row};

wire_enum! {
    BillingSubscriptionStatus {
    Incomplete => "incomplete",
    IncompleteExpired => "incomplete_expired",
    Trialing => "trialing",
    Active => "active",
    PastDue => "past_due",
    Canceled => "canceled",
    Unpaid => "unpaid",
    Paused => "paused",
    }
    parse: parse_billing_subscription_status
}

impl BillingSubscriptionStatus {
    pub fn can_create_agent(self) -> bool {
        matches!(self, Self::Active | Self::Trialing)
    }
}

/// Subscription-replacement guard: may an incoming webhook for a DIFFERENT
/// subscription id overwrite the account's current one? Only when there is no
/// current subscription, or the current one is terminal AND the incoming
/// status grants entitlement (active/trialing). Otherwise the incoming event
/// is ignored so an old or unrelated subscription cannot clobber a live one.
pub(crate) fn should_replace_stripe_subscription(
    current: Option<BillingSubscriptionStatus>,
    incoming: BillingSubscriptionStatus,
) -> bool {
    match current {
        None => true,
        Some(
            BillingSubscriptionStatus::Canceled | BillingSubscriptionStatus::IncompleteExpired,
        ) => incoming.can_create_agent(),
        Some(_) => false,
    }
}

/// Event-ordering guard: `true` when a delivery for the SAME subscription
/// carries a Stripe `event.created` strictly older than the last APPLIED
/// event's. Missing timestamps on either side never drop the event — legacy
/// rows (and dashboard deliveries without `event.created`) must still sync.
pub(crate) fn stripe_event_is_stale(last_applied: Option<i64>, incoming: Option<i64>) -> bool {
    matches!(
        (last_applied, incoming),
        (Some(last), Some(next)) if next < last
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomerBillingAccount {
    pub customer_org_id: String,
    #[serde(default)]
    pub hosting_tier: Option<HostingTier>,
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub stripe_price_id: Option<String>,
    pub subscription_status: Option<BillingSubscriptionStatus>,
    pub current_period_end: Option<String>,
    pub cancel_at_period_end: bool,
    pub last_stripe_event_id: Option<String>,
    /// Unix timestamp (`event.created`) of the most recently APPLIED Stripe
    /// webhook for this account. The event-ordering guard compares against it so
    /// a stale event delivered out of order can't resurrect a canceled sub.
    pub last_stripe_event_created: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinkStripeCustomerInput {
    pub verified_email: String,
    pub workos_user_id: String,
    pub stripe_customer_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncStripeSubscriptionInput {
    pub customer_org_id: Option<String>,
    pub stripe_customer_id: String,
    pub stripe_subscription_id: String,
    pub stripe_price_id: Option<String>,
    pub expected_stripe_price_id: Option<String>,
    pub subscription_status: BillingSubscriptionStatus,
    pub current_period_end: Option<String>,
    pub cancel_at_period_end: bool,
    pub stripe_event_id: Option<String>,
    /// Unix timestamp of the Stripe `event.created` for this delivery. Threaded
    /// from the dashboard webhook so Core can order webhooks monotonically and
    /// ignore stale ones (see `sync_stripe_subscription`).
    pub stripe_event_created: Option<i64>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinkStripeCustomerRequest {
    pub stripe_customer_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncStripeSubscriptionRequest {
    pub customer_org_id: Option<String>,
    pub stripe_customer_id: String,
    pub stripe_subscription_id: String,
    pub stripe_price_id: Option<String>,
    pub subscription_status: BillingSubscriptionStatus,
    pub current_period_end: Option<String>,
    pub cancel_at_period_end: bool,
    pub stripe_event_id: Option<String>,
    pub stripe_event_created: Option<i64>,
    pub now: Option<String>,
}

/// Does this org hold a subscription status that grants agent-creation
/// entitlement? Read-only gate used by self-serve agent creation.
pub(crate) async fn customer_org_has_active_billing<C>(
    client: &C,
    customer_org_id: &str,
) -> CoreResult<bool>
where
    C: GenericClient + Sync,
{
    let Some(row) = client
        .query_opt(
            "SELECT subscription_status FROM customer_billing_accounts
             WHERE customer_org_id = $1",
            &[&customer_org_id],
        )
        .await
        .map_err(store_error)?
    else {
        return Ok(false);
    };
    let Some(status) = row.get::<_, Option<String>>("subscription_status") else {
        return Ok(false);
    };
    let status = parse_billing_subscription_status(&status)
        .ok_or(CoreError::InvalidBillingSubscriptionStatus)?;
    Ok(BillingSubscriptionStatus::can_create_agent(status))
}

pub(crate) async fn select_customer_billing_account<C>(
    client: &C,
    customer_org_id: &str,
    for_update: bool,
) -> CoreResult<Option<CustomerBillingAccount>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT customer_org_id, hosting_tier, stripe_customer_id, stripe_subscription_id, stripe_price_id,
                subscription_status, core_rfc3339(current_period_end) AS current_period_end, cancel_at_period_end,
                last_stripe_event_id, last_stripe_event_created, core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
         FROM customer_billing_accounts WHERE customer_org_id = $1{}",
        if for_update { " FOR UPDATE" } else { "" }
    );
    client
        .query_opt(&sql, &[&customer_org_id])
        .await
        .map_err(store_error)?
        .map(|row| customer_billing_account_from_row(&row))
        .transpose()
}

async fn select_customer_billing_account_by_stripe_customer<C>(
    client: &C,
    stripe_customer_id: &str,
) -> CoreResult<Option<CustomerBillingAccount>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT customer_org_id, hosting_tier, stripe_customer_id, stripe_subscription_id, stripe_price_id,
                    subscription_status, core_rfc3339(current_period_end) AS current_period_end, cancel_at_period_end,
                    last_stripe_event_id, last_stripe_event_created, core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
             FROM customer_billing_accounts WHERE stripe_customer_id = $1",
            &[&stripe_customer_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| customer_billing_account_from_row(&row))
        .transpose()
}

fn customer_billing_account_from_row(row: &Row) -> CoreResult<CustomerBillingAccount> {
    let status: Option<String> = row.get("subscription_status");
    Ok(CustomerBillingAccount {
        customer_org_id: row.get("customer_org_id"),
        hosting_tier: optional_hosting_tier_column(row, "hosting_tier")?,
        stripe_customer_id: row.get("stripe_customer_id"),
        stripe_subscription_id: row.get("stripe_subscription_id"),
        stripe_price_id: row.get("stripe_price_id"),
        subscription_status: status
            .as_deref()
            .map(|value| {
                parse_billing_subscription_status(value).ok_or_else(|| {
                    CoreError::Store(format!("invalid billing subscription status {value}"))
                })
            })
            .transpose()?,
        current_period_end: row.get("current_period_end"),
        cancel_at_period_end: row.get("cancel_at_period_end"),
        last_stripe_event_id: row.get("last_stripe_event_id"),
        last_stripe_event_created: row.get("last_stripe_event_created"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// Guard `link_stripe_customer_to_org`'s conflict rules: a Stripe customer id may
/// belong to exactly one org, and an org's existing customer id cannot change.
async fn ensure_no_stripe_customer_conflict<C>(
    client: &C,
    customer_org_id: &str,
    stripe_customer_id: &str,
    existing: Option<&CustomerBillingAccount>,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    if let Some(existing_customer_id) =
        existing.and_then(|account| account.stripe_customer_id.as_deref())
        && existing_customer_id != stripe_customer_id
    {
        return Err(CoreError::StripeCustomerConflict);
    }
    if client
        .query_opt(
            "SELECT customer_org_id FROM customer_billing_accounts
             WHERE stripe_customer_id = $1 AND customer_org_id <> $2",
            &[&stripe_customer_id, &customer_org_id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::StripeCustomerConflict);
    }
    Ok(())
}

/// Row-scoped equivalent of `link_stripe_customer_to_org`: upsert only the
/// `customer_billing_accounts` row for this org, setting the Stripe customer id
/// while preserving any existing subscription fields.
async fn link_stripe_customer_to_org<C>(
    client: &C,
    customer_org_id: &str,
    stripe_customer_id: &str,
    now: &str,
) -> CoreResult<CustomerBillingAccount>
where
    C: GenericClient + Sync,
{
    let existing = select_customer_billing_account(client, customer_org_id, true).await?;
    ensure_no_stripe_customer_conflict(
        client,
        customer_org_id,
        stripe_customer_id,
        existing.as_ref(),
    )
    .await?;
    let row = client
        .query_one(
            "INSERT INTO customer_billing_accounts
               (customer_org_id, hosting_tier, stripe_customer_id, cancel_at_period_end, created_at, updated_at)
             VALUES ($1, 'standard', $2, FALSE, $3::text::timestamptz, $3::text::timestamptz)
             ON CONFLICT (customer_org_id) DO UPDATE SET
               stripe_customer_id = EXCLUDED.stripe_customer_id,
               updated_at = EXCLUDED.updated_at
             RETURNING customer_org_id, stripe_customer_id, stripe_subscription_id, stripe_price_id,
                       hosting_tier, subscription_status, core_rfc3339(current_period_end) AS current_period_end, cancel_at_period_end,
                       last_stripe_event_id, last_stripe_event_created,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[&customer_org_id, &stripe_customer_id, &now],
        )
        .await
        .map_err(store_error)?;
    customer_billing_account_from_row(&row)
}

/// Transaction body of `CoreStore::link_stripe_customer`: resolve the verified
/// user to their personal org, then link the Stripe customer to that org.
pub(crate) async fn link_stripe_customer<C>(
    client: &C,
    input: LinkStripeCustomerInput,
) -> CoreResult<CustomerBillingAccount>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let verified_email = normalize_owner_email(Some(&input.verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let workos_user_id = input.workos_user_id.trim().to_string();
    if workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    let stripe_customer_id = trim_to_option(Some(&input.stripe_customer_id))
        .ok_or(CoreError::MissingStripeCustomerId)?;

    // Same WorkOS-conflict guard the in-memory `ensure_linked_user_with_billing_class`
    // enforces: a workos id may not move to a different email.
    if client
        .query_opt(
            "SELECT id FROM users WHERE workos_user_id = $1 AND normalized_email <> $2",
            &[&workos_user_id, &verified_email],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::WorkosUserConflict);
    }

    let user = upsert_linked_user(client, &verified_email, &workos_user_id, &now).await?;
    let org = ensure_personal_org_row(client, &user, BillingClass::Standard, &now).await?;
    link_stripe_customer_to_org(client, &org.id, &stripe_customer_id, &now).await
}

/// Transaction body of `CoreStore::sync_stripe_subscription`: apply one
/// dashboard-normalized Stripe webhook delivery to the durable billing
/// account, under both webhook safety guards (see the module docs).
pub(crate) async fn sync_stripe_subscription<C>(
    client: &C,
    input: SyncStripeSubscriptionInput,
) -> CoreResult<CustomerBillingAccount>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let stripe_customer_id = trim_to_option(Some(&input.stripe_customer_id))
        .ok_or(CoreError::MissingStripeCustomerId)?;
    let stripe_subscription_id = trim_to_option(Some(&input.stripe_subscription_id))
        .ok_or(CoreError::MissingStripeSubscriptionId)?;
    let stripe_price_id = trim_to_option(input.stripe_price_id.as_deref());

    // Resolve the org: explicit id, else the account that already owns this
    // Stripe customer (natural key), else there is nothing to sync.
    let customer_org_id = match trim_to_option(input.customer_org_id.as_deref()) {
        Some(org_id) => org_id,
        None => select_customer_billing_account_by_stripe_customer(client, &stripe_customer_id)
            .await?
            .map(|account| account.customer_org_id)
            .ok_or(CoreError::BillingAccountNotFound)?,
    };
    if !customer_org_exists(client, &customer_org_id).await? {
        return Err(CoreError::BillingAccountNotFound);
    }

    // Lock this org's billing row for the transaction (row-scoped concurrency).
    let existing = select_customer_billing_account(client, &customer_org_id, true).await?;

    // Event-ordering guard: for the SAME subscription, drop a webhook whose
    // Stripe `event.created` predates the last applied one.
    if let Some(account) = existing.as_ref()
        && account.stripe_subscription_id.as_deref() == Some(stripe_subscription_id.as_str())
        && stripe_event_is_stale(
            account.last_stripe_event_created,
            input.stripe_event_created,
        )
    {
        return Ok(account.clone());
    }

    // Subscription-replacement guard: don't let a new subscription id clobber an
    // active one unless the status transition warrants it.
    if let Some(account) = existing.as_ref()
        && let Some(existing_subscription_id) = account.stripe_subscription_id.as_deref()
        && existing_subscription_id != stripe_subscription_id
        && !should_replace_stripe_subscription(
            account.subscription_status,
            input.subscription_status,
        )
    {
        return Ok(account.clone());
    }

    ensure_no_stripe_customer_conflict(
        client,
        &customer_org_id,
        &stripe_customer_id,
        existing.as_ref(),
    )
    .await?;

    if input.subscription_status.can_create_agent() {
        let expected_price_id = trim_to_option(input.expected_stripe_price_id.as_deref())
            .ok_or(CoreError::MissingStripeStandardPriceId)?;
        if stripe_price_id.as_deref() != Some(expected_price_id.as_str()) {
            return Err(CoreError::StripeSubscriptionPriceMismatch);
        }
    }

    let subscription_status = input.subscription_status.as_str();
    let last_stripe_event_id = trim_to_option(input.stripe_event_id.as_deref());
    let last_stripe_event_created = input
        .stripe_event_created
        .or_else(|| existing.as_ref().and_then(|a| a.last_stripe_event_created));
    let created_at = existing
        .as_ref()
        .map(|account| account.created_at.clone())
        .unwrap_or_else(|| now.clone());

    let row = client
        .query_one(
            "INSERT INTO customer_billing_accounts (
                customer_org_id, hosting_tier, stripe_customer_id, stripe_subscription_id, stripe_price_id,
                subscription_status, current_period_end, cancel_at_period_end,
                last_stripe_event_id, last_stripe_event_created, created_at, updated_at
             )
             VALUES ($1, 'standard', $2, $3, $4, $5, $6::text::timestamptz, $7, $8, $9, $10::text::timestamptz, $11::text::timestamptz)
             ON CONFLICT (customer_org_id) DO UPDATE SET
               stripe_customer_id = EXCLUDED.stripe_customer_id,
               stripe_subscription_id = EXCLUDED.stripe_subscription_id,
               stripe_price_id = EXCLUDED.stripe_price_id,
               subscription_status = EXCLUDED.subscription_status,
               current_period_end = EXCLUDED.current_period_end,
               cancel_at_period_end = EXCLUDED.cancel_at_period_end,
               last_stripe_event_id = EXCLUDED.last_stripe_event_id,
               last_stripe_event_created = EXCLUDED.last_stripe_event_created,
               updated_at = EXCLUDED.updated_at
             RETURNING customer_org_id, stripe_customer_id, stripe_subscription_id, stripe_price_id,
                       hosting_tier, subscription_status, core_rfc3339(current_period_end) AS current_period_end, cancel_at_period_end,
                       last_stripe_event_id, last_stripe_event_created,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[
                &customer_org_id,
                &stripe_customer_id,
                &stripe_subscription_id,
                &stripe_price_id,
                &subscription_status,
                &input.current_period_end,
                &input.cancel_at_period_end,
                &last_stripe_event_id,
                &last_stripe_event_created,
                &created_at,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    let account = customer_billing_account_from_row(&row)?;

    // Grant on active/trialing; zero the standard (non-launch-code) grant otherwise.
    if input.subscription_status.can_create_agent() {
        ensure_standard_agent_creation_entitlement_row(client, &customer_org_id, &now).await?;
        client
            .execute(
                "UPDATE customer_orgs
                 SET billing_class = 'standard', updated_at = $2::text::timestamptz
                 WHERE id = $1",
                &[&customer_org_id, &now],
            )
            .await
            .map_err(store_error)?;
    } else {
        client
            .execute(
                "UPDATE agent_creation_entitlements
                 SET allowed_new_agent_runtimes = 0, updated_at = $2::text::timestamptz
                 WHERE customer_org_id = $1 AND launch_code IS NULL",
                &[&customer_org_id, &now],
            )
            .await
            .map_err(store_error)?;
    }

    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_active_and_trialing_grant_agent_creation() {
        for status in [
            BillingSubscriptionStatus::Incomplete,
            BillingSubscriptionStatus::IncompleteExpired,
            BillingSubscriptionStatus::PastDue,
            BillingSubscriptionStatus::Canceled,
            BillingSubscriptionStatus::Unpaid,
            BillingSubscriptionStatus::Paused,
        ] {
            assert!(!status.can_create_agent(), "{status:?} must not entitle");
        }
        assert!(BillingSubscriptionStatus::Active.can_create_agent());
        assert!(BillingSubscriptionStatus::Trialing.can_create_agent());
    }

    #[test]
    fn status_wire_strings_are_stripe_subscription_statuses() {
        // These strings are Stripe's `subscription.status` vocabulary, pinned
        // by the dashboard webhook mapper. Round-trip every variant through
        // serde and the parser so the DB column, JSON API, and parser can
        // never drift apart.
        for status in [
            BillingSubscriptionStatus::Incomplete,
            BillingSubscriptionStatus::IncompleteExpired,
            BillingSubscriptionStatus::Trialing,
            BillingSubscriptionStatus::Active,
            BillingSubscriptionStatus::PastDue,
            BillingSubscriptionStatus::Canceled,
            BillingSubscriptionStatus::Unpaid,
            BillingSubscriptionStatus::Paused,
        ] {
            let wire = status.as_str();
            assert_eq!(parse_billing_subscription_status(wire), Some(status));
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, format!("\"{wire}\""));
            assert_eq!(
                serde_json::from_str::<BillingSubscriptionStatus>(&json).unwrap(),
                status
            );
        }
        assert_eq!(parse_billing_subscription_status("not_a_status"), None);
    }

    #[test]
    fn replacement_requires_terminal_current_and_entitling_incoming() {
        use BillingSubscriptionStatus::*;
        // No current subscription: any incoming status may establish one.
        for incoming in [
            Active, Trialing, PastDue, Canceled, Unpaid, Paused, Incomplete,
        ] {
            assert!(should_replace_stripe_subscription(None, incoming));
        }
        // Terminal current: only an entitling incoming status replaces it.
        for current in [Canceled, IncompleteExpired] {
            assert!(should_replace_stripe_subscription(Some(current), Active));
            assert!(should_replace_stripe_subscription(Some(current), Trialing));
            for incoming in [
                PastDue,
                Canceled,
                Unpaid,
                Paused,
                Incomplete,
                IncompleteExpired,
            ] {
                assert!(!should_replace_stripe_subscription(Some(current), incoming));
            }
        }
        // Anything else live (active, trialing, past_due, unpaid, paused,
        // incomplete): a different subscription id never replaces it.
        for current in [Active, Trialing, PastDue, Unpaid, Paused, Incomplete] {
            assert!(!should_replace_stripe_subscription(Some(current), Active));
            assert!(!should_replace_stripe_subscription(Some(current), Canceled));
        }
    }

    #[test]
    fn stale_event_guard_requires_both_timestamps_and_strictly_older_event() {
        // Strictly older delivery for the same subscription is stale.
        assert!(stripe_event_is_stale(Some(100), Some(99)));
        // Equal or newer deliveries are applied.
        assert!(!stripe_event_is_stale(Some(100), Some(100)));
        assert!(!stripe_event_is_stale(Some(100), Some(101)));
        // Missing timestamps never drop the event: legacy rows and deliveries
        // without `event.created` must still sync.
        assert!(!stripe_event_is_stale(Some(100), None));
        assert!(!stripe_event_is_stale(None, Some(99)));
        assert!(!stripe_event_is_stale(None, None));
    }

    #[test]
    fn sync_request_json_is_the_dashboard_contract() {
        // The dashboard webhook posts exactly these camelCase fields. Pin the
        // wire shape so a rename here cannot silently break the dashboard.
        let request: SyncStripeSubscriptionRequest = serde_json::from_str(
            r#"{
                "customerOrgId": "org_1",
                "stripeCustomerId": "cus_1",
                "stripeSubscriptionId": "sub_1",
                "stripePriceId": "price_standard",
                "subscriptionStatus": "active",
                "currentPeriodEnd": "2026-01-01T00:00:00Z",
                "cancelAtPeriodEnd": true,
                "stripeEventId": "evt_1",
                "stripeEventCreated": 1234,
                "now": null
            }"#,
        )
        .unwrap();
        assert_eq!(request.stripe_customer_id, "cus_1");
        assert_eq!(
            request.subscription_status,
            BillingSubscriptionStatus::Active
        );
        assert_eq!(request.stripe_event_created, Some(1234));
        let round_trip = serde_json::to_value(&request).unwrap();
        assert!(round_trip.get("stripeCustomerId").is_some());
        assert!(round_trip.get("stripeSubscriptionId").is_some());
        assert!(round_trip.get("stripeEventCreated").is_some());

        let link: LinkStripeCustomerRequest =
            serde_json::from_str(r#"{ "stripeCustomerId": "cus_2", "now": null }"#).unwrap();
        assert_eq!(link.stripe_customer_id, "cus_2");
    }
}
