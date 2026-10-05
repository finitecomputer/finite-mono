use super::*;
use crate::trials::*;

impl CoreStore {
    #[tracing::instrument(skip_all, fields(operator = actor, campaign = id))]
    pub async fn increase_trial_capacity(
        &self,
        id: &str,
        input: IncreaseTrialCapacity,
        actor: &str,
    ) -> CoreResult<()> {
        if actor.trim().is_empty() || !(1..=10000).contains(&input.seat_limit) {
            return Err(CoreError::TrialUnavailable(
                "Total signup limit must be 1–10000.",
            ));
        }
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        // Share reserve_trial's campaign lock: capacity and reservations cannot race.
        let row = tx
            .query_opt(
                "SELECT seat_limit,active FROM trial_campaigns WHERE id=$1 FOR UPDATE",
                &[&id],
            )
            .await
            .map_err(store_error)?
            .ok_or(CoreError::TrialUnavailable("Trial campaign not found."))?;
        let current: i32 = row.get(0);
        if !row.get::<_, bool>(1) {
            return Err(CoreError::TrialUnavailable("This campaign is inactive."));
        }
        if current != input.expected_seat_limit {
            return Err(CoreError::TrialUnavailable(
                "The signup limit changed. Refresh before increasing it.",
            ));
        }
        if input.seat_limit <= current {
            return Err(CoreError::TrialUnavailable(
                "New total signup limit must exceed the current limit.",
            ));
        }
        tx.execute(
            "UPDATE trial_campaigns SET seat_limit=$2 WHERE id=$1",
            &[&id, &input.seat_limit],
        )
        .await
        .map_err(store_error)?;
        self.finish(tx).await
    }

    #[tracing::instrument(skip_all, fields(operator = actor))]
    pub async fn create_trial_campaign(
        &self,
        input: CreateTrialCampaign,
        actor: &str,
    ) -> CoreResult<IssuedTrialCampaign> {
        if input.name.trim().is_empty()
            || input.name.chars().count() > 120
            || input.name.chars().any(char::is_control)
            || !(1..=10000).contains(&input.seat_limit)
            || !(1..=30).contains(&input.trial_days)
            || actor.trim().is_empty()
        {
            return Err(CoreError::TrialUnavailable(
                "Enter an event name, 1–10000 seats, and 1–30 trial days.",
            ));
        }
        let id = crate::generate_surrogate_id("trial_campaign")?;
        let code = generate_trial_code()?;
        let hash = hash_trial_code(&code)?;
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.execute("INSERT INTO trial_campaigns(id,name,code_hash,seat_limit,trial_days,created_by_workos_user_id,code) VALUES($1,$2,$3,$4,$5,$6,$7)",
            &[&id,&input.name.trim(),&hash,&input.seat_limit,&input.trial_days,&actor,&code]).await.map_err(store_error)?;
        self.finish(tx).await?;
        Ok(IssuedTrialCampaign { id, code })
    }

    pub async fn trial_offer(&self, code: &str) -> CoreResult<TrialOffer> {
        let client = self.connection().await?;
        let hash = hash_trial_code(code)?;
        let row = client
            .query_opt(
                "SELECT id,trial_days FROM trial_campaigns WHERE code_hash=$1 AND active",
                &[&hash],
            )
            .await
            .map_err(store_error)?
            .ok_or(CoreError::TrialUnavailable(
                "This trial code is invalid or inactive.",
            ))?;
        Ok(TrialOffer {
            campaign_id: row.get(0),
            trial_days: row.get(1),
        })
    }

    #[tracing::instrument(skip_all, fields(user = user, org = input.customer_org_id))]
    pub async fn reserve_trial(
        &self,
        user: &str,
        input: ReserveTrial,
    ) -> CoreResult<TrialReservation> {
        if !input.stripe_session_id.starts_with("cs_")
            || input.attempt_id.trim().is_empty()
            || input.attempt_id.len() > 128
        {
            return Err(CoreError::TrialUnavailable("Invalid trial checkout."));
        }
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.execute("INSERT INTO trial_checkout_sessions(stripe_session_id,stripe_customer_id) VALUES($1,$2) ON CONFLICT(stripe_session_id) DO NOTHING", &[&input.stripe_session_id,&input.stripe_customer_id]).await.map_err(store_error)?;
        let session = tx.query_one("SELECT expired,stripe_customer_id FROM trial_checkout_sessions WHERE stripe_session_id=$1 FOR UPDATE", &[&input.stripe_session_id]).await.map_err(store_error)?;
        if session.get::<_, bool>(0) || session.get::<_, String>(1) != input.stripe_customer_id {
            return Err(CoreError::TrialUnavailable(
                "This checkout has expired. Please try again.",
            ));
        }
        let hash = hash_trial_code(&input.code)?;
        let campaign = tx.query_opt("SELECT id,seat_limit,trial_days FROM trial_campaigns WHERE code_hash=$1 AND active FOR UPDATE", &[&hash]).await.map_err(store_error)?
            .ok_or(CoreError::TrialUnavailable("This trial code is invalid or inactive."))?;
        let id: String = campaign.get(0);
        if campaign.get::<_, i32>(2) != input.trial_days {
            return Err(CoreError::TrialUnavailable(
                "The trial offer changed. Please try again.",
            ));
        }
        let org = tx.query_opt("SELECT o.id FROM customer_orgs o JOIN users u ON u.id=o.owner_user_id JOIN customer_billing_accounts b ON b.customer_org_id=o.id
            WHERE o.id=$1 AND u.workos_user_id=$2 AND b.stripe_customer_id=$3 AND o.billing_class='standard'
              AND b.stripe_subscription_id IS NULL FOR UPDATE OF o,b", &[&input.customer_org_id,&user,&input.stripe_customer_id]).await.map_err(store_error)?;
        if org.is_none() {
            return Err(CoreError::TrialUnavailable(
                "Trials are available to new subscription accounts only.",
            ));
        }
        if let Some(existing) = tx.query_opt("SELECT campaign_id,stripe_session_id,state FROM trial_redemptions WHERE customer_org_id=$1 AND state <> 'expired'", &[&input.customer_org_id]).await.map_err(store_error)? {
            if existing.get::<_, String>(0) != id || existing.get::<_, String>(2) != "reserved" {
                return Err(CoreError::TrialUnavailable("This account has already used a trial."));
            }
            return Ok(TrialReservation { stripe_session_id: existing.get(1) });
        }
        let count: i64 = tx.query_one("SELECT count(*) FROM trial_redemptions WHERE campaign_id=$1 AND state <> 'expired'", &[&id]).await.map_err(store_error)?.get(0);
        if count >= i64::from(campaign.get::<_, i32>(1)) {
            return Err(CoreError::TrialUnavailable(
                "All seats for this trial code have been claimed.",
            ));
        }
        let valid: bool = tx
            .query_one(
                "SELECT to_timestamp($1::bigint::double precision) > CURRENT_TIMESTAMP",
                &[&input.checkout_expires_at],
            )
            .await
            .map_err(store_error)?
            .get(0);
        if !valid {
            return Err(CoreError::TrialUnavailable(
                "This checkout has expired. Please try again.",
            ));
        }
        let redemption_id = crate::generate_surrogate_id("trial_seat")?;
        tx.execute("INSERT INTO trial_redemptions(id,campaign_id,customer_org_id,attempt_id,stripe_session_id,state,checkout_expires_at)
            VALUES($1,$2,$3,$4,$5,'reserved',to_timestamp($6::bigint::double precision))", &[&redemption_id,&id,&input.customer_org_id,&input.attempt_id,&input.stripe_session_id,&input.checkout_expires_at]).await.map_err(store_error)?;
        self.finish(tx).await?;
        Ok(TrialReservation {
            stripe_session_id: input.stripe_session_id,
        })
    }

    #[tracing::instrument(skip_all, fields(session = input.stripe_session_id))]
    pub async fn expire_trial(&self, input: ExpireTrial) -> CoreResult<()> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.execute("INSERT INTO trial_checkout_sessions(stripe_session_id,stripe_customer_id,expired) VALUES($1,$2,TRUE)
            ON CONFLICT(stripe_session_id) DO UPDATE SET expired=TRUE WHERE trial_checkout_sessions.stripe_customer_id=EXCLUDED.stripe_customer_id", &[&input.stripe_session_id,&input.stripe_customer_id]).await.map_err(store_error)?;
        // Only verified Stripe session.expired may release a reservation. A
        // completed seat remains consumed even after cancellation/nonpayment.
        tx.execute("UPDATE trial_redemptions r SET state='expired' FROM customer_billing_accounts b
            WHERE r.stripe_session_id=$1 AND r.state='reserved' AND b.customer_org_id=r.customer_org_id AND b.stripe_customer_id=$2",
            &[&input.stripe_session_id,&input.stripe_customer_id]).await.map_err(store_error)?;
        self.finish(tx).await
    }

    pub async fn list_trial_campaigns(&self) -> CoreResult<Vec<TrialCampaign>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.batch_execute("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .await
            .map_err(store_error)?;
        let mut result = Vec::new();
        for row in tx.query("SELECT id,name,seat_limit,trial_days,active,code,code_revision FROM trial_campaigns ORDER BY created_at DESC", &[]).await.map_err(store_error)? {
            let id: String = row.get(0);
            let rows = tx.query("SELECT r.customer_org_id,u.workos_user_id,r.state,core_rfc3339(r.redeemed_at),
                b.subscription_status,core_rfc3339(b.current_period_end),core_trial_access_blocked(o.id,CURRENT_TIMESTAMP),
                u.normalized_email,ARRAY(SELECT p.display_name FROM projects p WHERE p.customer_org_id=o.id ORDER BY p.created_at,p.id)
                FROM trial_redemptions r JOIN customer_orgs o ON o.id=r.customer_org_id JOIN users u ON u.id=o.owner_user_id
                LEFT JOIN customer_billing_accounts b ON b.customer_org_id=o.id WHERE r.campaign_id=$1 ORDER BY r.created_at", &[&id]).await.map_err(store_error)?;
            let redemptions: Vec<_> = rows.iter().map(|r| {
                let state: String = r.get(2);
                let trial_access = (state == "redeemed").then(|| TrialAccess {
                    blocked: r.get(6), event_name: row.get(1), subscription_status: r.get(4), period_end: r.get(5),
                });
                TrialRedemption { customer_org_id:r.get(0),owner_workos_user_id:r.get(1),owner_email:r.get(7),agent_names:r.get(8),state,redeemed_at:r.get(3),trial_access }
            }).collect();
            let reserved_seats = redemptions.iter().filter(|r| r.state == "reserved").count() as i64;
            let redeemed_seats = redemptions.iter().filter(|r| r.state == "redeemed").count() as i64;
            let seat_limit: i32 = row.get(2);
            result.push(TrialCampaign { id,name:row.get(1),code:row.get(5),code_revision:row.get(6),seat_limit,trial_days:row.get(3),active:row.get(4),reserved_seats,redeemed_seats,seats_remaining:i64::from(seat_limit)-reserved_seats-redeemed_seats,redemptions });
        }
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }
}

pub(super) async fn redeem<C: GenericClient + Sync>(
    tx: &C,
    attempt: &str,
    account: &CustomerBillingAccount,
) -> CoreResult<()> {
    let row = tx.query_opt("SELECT id,stripe_subscription_id,state FROM trial_redemptions WHERE attempt_id=$1 AND customer_org_id=$2 FOR UPDATE", &[&attempt,&account.customer_org_id]).await.map_err(store_error)?
        .ok_or(CoreError::TrialUnavailable("Trial checkout has no reserved seat."))?;
    let previous: Option<String> = row.get(1);
    if row.get::<_, String>(2) == "expired"
        || previous
            .as_ref()
            .is_some_and(|s| Some(s) != account.stripe_subscription_id.as_ref())
    {
        return Err(CoreError::TrialUnavailable(
            "Trial checkout does not match its reserved seat.",
        ));
    }
    tx.execute("UPDATE trial_redemptions SET state='redeemed',stripe_subscription_id=$2,redeemed_at=COALESCE(redeemed_at,CURRENT_TIMESTAMP) WHERE id=$1",
        &[&row.get::<_, String>(0),&account.stripe_subscription_id]).await.map_err(store_error)?;
    Ok(())
}
