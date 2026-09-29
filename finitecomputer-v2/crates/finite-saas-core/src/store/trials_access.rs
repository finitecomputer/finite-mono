use super::*;
use crate::trials::TrialAccess;

/// Apply this new policy only to accounts that redeemed an event trial.
/// Sponsored and grandfathered access keeps its existing contract.
pub(super) async fn trial_access<C: GenericClient + Sync>(
    tx: &C,
    org: &str,
) -> CoreResult<Option<TrialAccess>> {
    let row = tx.query_opt("SELECT c.name,b.subscription_status,core_rfc3339(b.current_period_end),o.billing_class FROM customer_orgs o
        JOIN trial_redemptions t ON t.customer_org_id=o.id AND t.state='redeemed' JOIN trial_campaigns c ON c.id=t.campaign_id
        JOIN customer_billing_accounts b ON b.customer_org_id=o.id WHERE o.id=$1", &[&org]).await.map_err(store_error)?;
    Ok(row.map(|row| {
        let status: Option<String> = row.get(1);
        TrialAccess {
            blocked: row.get::<_, String>(3) == "standard"
                && !matches!(status.as_deref(), Some("active" | "trialing")),
            event_name: row.get(0),
            subscription_status: status,
            period_end: row.get(2),
        }
    }))
}
