use super::*;

impl CoreStore {
    pub async fn billing_overview(
        &self,
        input: LinkVerifiedUserInput,
    ) -> CoreResult<BillingOverview> {
        // Read-only: no global lock, no full-state rewrite, no writes at all.
        // A read that wrote the whole DB was anti-pattern #3; this is targeted
        // SELECTs inside a READ ONLY transaction.
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.execute("SET TRANSACTION READ ONLY", &[])
            .await
            .map_err(store_error)?;
        let overview = postgres_billing_overview(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(overview)
    }

    pub async fn link_stripe_customer(
        &self,
        input: LinkStripeCustomerInput,
    ) -> CoreResult<CustomerBillingAccount> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let account = billing::link_stripe_customer(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(account)
    }

    pub async fn sync_stripe_subscription(
        &self,
        input: SyncStripeSubscriptionInput,
    ) -> CoreResult<CustomerBillingAccount> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let account = billing::sync_stripe_subscription(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(account)
    }
}

pub(super) async fn select_agent_creation_entitlement_by_org<C>(
    client: &C,
    customer_org_id: &str,
) -> CoreResult<Option<AgentCreationEntitlement>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, customer_org_id, hosting_tier, allowed_new_agent_runtimes, launch_code,
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
             FROM agent_creation_entitlements WHERE customer_org_id = $1",
            &[&customer_org_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| agent_creation_entitlement_from_row(&row))
        .transpose()
}

/// Mirrors `active_agent_creation_entitlement_count`: active self-serve runtime
/// links plus pending (`requested`/`launching`) self-serve requests for the org,
/// all row-scoped. Imported legacy runtimes remain visible but do not consume a
/// hosted/self-serve launch entitlement.
pub(super) async fn postgres_active_agent_creation_entitlement_count<C>(
    client: &C,
    customer_org_id: &str,
) -> CoreResult<i64>
where
    C: GenericClient + Sync,
{
    Ok(client
        .query_one(
            "SELECT (
                (SELECT COUNT(*) FROM project_runtime_links links
                 JOIN projects projects ON projects.id = links.project_id
                 WHERE projects.customer_org_id = $1
                   AND projects.import_candidate_id IS NULL
                   AND links.active = TRUE)
                +
                (SELECT COUNT(*) FROM agent_creation_requests
                 WHERE customer_org_id = $1 AND status IN ('requested', 'launching'))
             )::BIGINT",
            &[&customer_org_id],
        )
        .await
        .map_err(store_error)?
        .get(0))
}

/// Read-only billing overview via targeted SELECTs. NEVER writes: an org that
/// does not exist yet (a user who has not reached checkout) yields a synthesized
/// Standard view (`requires_billing`, `!can_create_agent`) rather than creating
/// rows — the persisted org is minted on the write paths (checkout/link), not here.
async fn postgres_billing_overview<C>(
    client: &C,
    input: LinkVerifiedUserInput,
) -> CoreResult<BillingOverview>
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

    let org = match select_user_by_email(client, &verified_email).await? {
        Some(user) => select_personal_org_by_owner(client, &user.id).await?,
        None => None,
    };
    let org = match org {
        Some(org) => org,
        // No persisted org yet: synthesize the Standard default the write paths
        // would create, WITHOUT inserting anything.
        None => CustomerOrganization {
            id: new_customer_org_id()?,
            owner_user_id: String::new(),
            name: verified_email,
            billing_class: BillingClass::Standard,
            created_at: now.clone(),
            updated_at: now,
        },
    };

    let billing_account = billing::select_customer_billing_account(client, &org.id, false).await?;
    let agent_creation_entitlement =
        select_agent_creation_entitlement_by_org(client, &org.id).await?;
    let has_active_billing = billing::customer_org_has_active_billing(client, &org.id).await?;
    let active_count = postgres_active_agent_creation_entitlement_count(client, &org.id).await?;

    let can_create_agent = agent_creation_entitlement
        .as_ref()
        .is_some_and(|entitlement| {
            active_count < i64::from(entitlement.allowed_new_agent_runtimes)
        })
        && (has_active_billing
            || org.billing_class == BillingClass::Grandfathered
            || org.billing_class == BillingClass::Sponsored);
    let requires_billing = !has_active_billing && org.billing_class == BillingClass::Standard;

    Ok(BillingOverview {
        customer_org: org,
        billing_account,
        agent_creation_entitlement,
        can_create_agent,
        requires_billing,
    })
}

pub(crate) async fn customer_org_exists<C>(client: &C, org_id: &str) -> CoreResult<bool>
where
    C: GenericClient + Sync,
{
    Ok(client
        .query_opt("SELECT id FROM customer_orgs WHERE id = $1", &[&org_id])
        .await
        .map_err(store_error)?
        .is_some())
}

pub(super) async fn grant_launch_code_agent_creation_entitlement_row<C>(
    client: &C,
    customer_org_id: &str,
    launch_code_id: &str,
    hosting_tier: HostingTier,
    now: &str,
) -> CoreResult<AgentCreationEntitlement>
where
    C: GenericClient + Sync,
{
    let id = agent_creation_entitlement_id_for(customer_org_id);
    let row = client
        .query_one(
            "INSERT INTO agent_creation_entitlements
               (id, customer_org_id, hosting_tier, allowed_new_agent_runtimes, launch_code, created_at, updated_at)
             VALUES ($1, $2, $3, 1, $4, $5::text::timestamptz, $5::text::timestamptz)
             ON CONFLICT (customer_org_id) DO UPDATE SET
               allowed_new_agent_runtimes = agent_creation_entitlements.allowed_new_agent_runtimes + 1,
               hosting_tier = EXCLUDED.hosting_tier,
               launch_code = EXCLUDED.launch_code,
               updated_at = EXCLUDED.updated_at
             RETURNING id, customer_org_id, hosting_tier, allowed_new_agent_runtimes, launch_code,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[
                &id,
                &customer_org_id,
                &hosting_tier.as_str(),
                &launch_code_id,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    agent_creation_entitlement_from_row(&row)
}

pub(crate) async fn ensure_standard_agent_creation_entitlement_row<C>(
    client: &C,
    customer_org_id: &str,
    now: &str,
) -> CoreResult<AgentCreationEntitlement>
where
    C: GenericClient + Sync,
{
    let id = agent_creation_entitlement_id_for(customer_org_id);
    let row = client
        .query_one(
            "INSERT INTO agent_creation_entitlements
               (id, customer_org_id, hosting_tier, allowed_new_agent_runtimes, launch_code, created_at, updated_at)
             VALUES ($1, $2, 'standard', 1, NULL, $3::text::timestamptz, $3::text::timestamptz)
             ON CONFLICT (customer_org_id) DO UPDATE SET
               allowed_new_agent_runtimes = GREATEST(
                 agent_creation_entitlements.allowed_new_agent_runtimes,
                 EXCLUDED.allowed_new_agent_runtimes
               ),
               launch_code = agent_creation_entitlements.launch_code,
               updated_at = EXCLUDED.updated_at
             RETURNING id, customer_org_id, hosting_tier, allowed_new_agent_runtimes, launch_code,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[&id, &customer_org_id, &now],
        )
        .await
        .map_err(store_error)?;
    agent_creation_entitlement_from_row(&row)
}
