use super::*;
use crate::trials::TrialAccess;

/// Apply this new policy only to accounts that redeemed an event trial.
/// Sponsored and grandfathered access keeps its existing contract.
pub(super) async fn trial_access<C: GenericClient + Sync>(
    tx: &C,
    org: &str,
    now: &str,
) -> CoreResult<Option<TrialAccess>> {
    let row = tx.query_opt("SELECT c.name,b.subscription_status,core_rfc3339(b.current_period_end),core_trial_access_blocked(o.id, $2::text::timestamptz) FROM customer_orgs o
        JOIN trial_redemptions t ON t.customer_org_id=o.id AND t.state='redeemed' JOIN trial_campaigns c ON c.id=t.campaign_id
        LEFT JOIN customer_billing_accounts b ON b.customer_org_id=o.id WHERE o.id=$1", &[&org, &now]).await.map_err(store_error)?;
    Ok(row.map(|row| {
        let status: Option<String> = row.get(1);
        TrialAccess {
            blocked: row.get(3),
            event_name: row.get(0),
            subscription_status: status,
            period_end: row.get(2),
        }
    }))
}
