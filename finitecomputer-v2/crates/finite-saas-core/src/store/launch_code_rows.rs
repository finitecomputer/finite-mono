use super::*;

pub(super) async fn postgres_list_launch_code_batches<C>(
    client: &C,
) -> CoreResult<Vec<LaunchCodeBatchDetails>>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query(
            "SELECT id, name, hosting_tier, code_count, core_rfc3339(expires_at) AS expires_at, core_rfc3339(revoked_at) AS revoked_at,
                    revoked_by_workos_user_id, created_by_workos_user_id,
                    core_rfc3339(created_at) AS created_at
               FROM launch_code_batches
              ORDER BY launch_code_batches.created_at DESC, id DESC",
            &[],
        )
        .await
        .map_err(store_error)?;
    let mut details = Vec::with_capacity(rows.len());
    for row in rows {
        details.push(
            postgres_launch_code_batch_details(client, launch_code_batch_from_row(&row)?).await?,
        );
    }
    Ok(details)
}

pub(super) async fn postgres_launch_code_batch_details<C>(
    client: &C,
    batch: LaunchCodeBatch,
) -> CoreResult<LaunchCodeBatchDetails>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query(
            "SELECT id, redeemed_customer_org_id, core_rfc3339(redeemed_at) AS redeemed_at
               FROM launch_codes
              WHERE batch_id = $1
              ORDER BY id",
            &[&batch.id],
        )
        .await
        .map_err(store_error)?;
    let codes = rows
        .into_iter()
        .map(|row| LaunchCodeStatus {
            id: row.get("id"),
            redeemed_customer_org_id: row.get("redeemed_customer_org_id"),
            redeemed_at: row.get("redeemed_at"),
        })
        .collect();
    Ok(LaunchCodeBatchDetails { batch, codes })
}

pub(super) fn launch_code_batch_from_row(row: &Row) -> CoreResult<LaunchCodeBatch> {
    let count: i32 = row.get("code_count");
    Ok(LaunchCodeBatch {
        id: row.get("id"),
        name: row.get("name"),
        hosting_tier: optional_hosting_tier_column(row, "hosting_tier")?,
        code_count: u32::try_from(count).map_err(|_| CoreError::InvalidLaunchCodeBatchSize)?,
        expires_at: row.get("expires_at"),
        revoked_at: row.get("revoked_at"),
        revoked_by_workos_user_id: row.get("revoked_by_workos_user_id"),
        created_by_workos_user_id: row.get("created_by_workos_user_id"),
        created_at: row.get("created_at"),
    })
}

pub(super) async fn lock_unused_standard_launch_code<C: GenericClient + Sync>(
    client: &C,
    code_id: &str,
    expected_batch_id: &str,
    operator_workos_user_id: &str,
) -> CoreResult<()> {
    let row = client
        .query_opt(
            "SELECT code.id FROM launch_codes code
             JOIN launch_code_batches batch ON batch.id = code.batch_id
             WHERE code.id = $1 AND code.batch_id = $2
               AND batch.created_by_workos_user_id = $3
               AND COALESCE(batch.hosting_tier, 'standard') = 'standard'
               AND batch.code_count = 1
               AND batch.revoked_at IS NULL AND batch.expires_at > CURRENT_TIMESTAMP
               AND code.redeemed_customer_org_id IS NULL
               AND code.redemption_idempotency_key IS NULL AND code.redeemed_at IS NULL
             FOR UPDATE OF code, batch",
            &[&code_id, &expected_batch_id, &operator_workos_user_id],
        )
        .await
        .map_err(store_error)?;
    if row.is_none() {
        return Err(CoreError::InvalidLaunchCode);
    }
    Ok(())
}

pub(super) async fn lock_postgres_launch_code<C>(
    client: &C,
    launch_code: &str,
    now: &str,
) -> CoreResult<LockedLaunchCode>
where
    C: GenericClient + Sync,
{
    let code_hash = hash_launch_code(launch_code)?;
    parse_time(now)?;
    let row = client
        .query_opt(
            "SELECT code.id, code.batch_id, code.code_hash,
                    code.redeemed_customer_org_id,
                    code.redemption_idempotency_key, core_rfc3339(code.redeemed_at) AS redeemed_at,
                    core_rfc3339(code.created_at) AS created_at,
                    batch.hosting_tier,
                    batch.revoked_at IS NOT NULL AS batch_revoked,
                    batch.expires_at <= $2::text::timestamptz AS batch_expired
              FROM launch_codes AS code
               JOIN launch_code_batches AS batch ON batch.id = code.batch_id
              WHERE code.code_hash = $1
              FOR UPDATE OF code, batch",
            &[&code_hash, &now],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::InvalidLaunchCode)?;
    let batch_revoked: bool = row.get("batch_revoked");
    let batch_expired: bool = row.get("batch_expired");
    let redeemed_customer_org_id: Option<String> = row.get("redeemed_customer_org_id");
    if redeemed_customer_org_id.is_none() && (batch_revoked || batch_expired) {
        return Err(CoreError::InvalidLaunchCode);
    }
    // Read the binding AFTER taking the code lock. Redemption and operator
    // targeting serialize on that same row; an outer-join snapshot could miss
    // a binding committed while the lock was being acquired.
    let target_source_host_id = client
        .query_opt(
            "SELECT source_host_id FROM launch_code_host_targets WHERE launch_code_id = $1",
            &[&row.get::<_, String>("id")],
        )
        .await
        .map_err(store_error)?
        .map(|target| target.get("source_host_id"));
    Ok(LockedLaunchCode {
        target_source_host_id,
        hosting_tier: optional_hosting_tier_column(&row, "hosting_tier")?,
        record: LaunchCodeRecord {
            id: row.get("id"),
            batch_id: row.get("batch_id"),
            code_hash: row.get("code_hash"),
            redeemed_customer_org_id,
            redemption_idempotency_key: row.get("redemption_idempotency_key"),
            redeemed_at: row.get("redeemed_at"),
            created_at: row.get("created_at"),
        },
    })
}

pub(super) async fn redeem_postgres_launch_code<C>(
    client: &C,
    launch_code_id: &str,
    customer_org_id: &str,
    idempotency_key: &str,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let updated = client
        .execute(
            "UPDATE launch_codes
                SET redeemed_customer_org_id = $2,
                    redemption_idempotency_key = $3,
                    redeemed_at = $4::text::timestamptz
              WHERE id = $1
                AND redeemed_customer_org_id IS NULL
                AND redemption_idempotency_key IS NULL
                AND redeemed_at IS NULL",
            &[&launch_code_id, &customer_org_id, &idempotency_key, &now],
        )
        .await
        .map_err(store_error)?;
    if updated != 1 {
        return Err(CoreError::InvalidLaunchCode);
    }
    Ok(())
}
