use super::*;

pub(super) fn finite_private_limit_profile_from_row(row: &Row) -> FinitePrivateLimitProfile {
    FinitePrivateLimitProfile {
        id: row.get("id"),
        burst_window_seconds: row.get("burst_window_seconds"),
        burst_limit_units: row.get("burst_limit_units"),
        weekly_limit_units: row.get("weekly_limit_units"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn finite_private_reservation_from_row(row: &Row) -> CoreResult<FinitePrivateReservation> {
    let status: String = row.get("status");
    let settlement_kind: Option<String> = row.get("settlement_kind");
    let settlement_kind = match settlement_kind.as_deref() {
        Some(value) => Some(
            crate::parse_finite_private_settlement_kind(value).ok_or_else(|| {
                CoreError::Store(format!("invalid finite private settlement kind {value}"))
            })?,
        ),
        None => None,
    };
    Ok(FinitePrivateReservation {
        id: row.get("id"),
        request_id: row.get("request_id"),
        api_key_id: row.get("api_key_id"),
        grant_id: row.get("grant_id"),
        endpoint: row.get("endpoint"),
        model: row.get("model"),
        estimated_usage_units: row.get("estimated_usage_units"),
        reserved_usage_units: row.get("reserved_usage_units"),
        settled_usage_units: row.get("settled_usage_units"),
        settlement_kind,
        status: parse_finite_private_reservation_status(&status).ok_or_else(|| {
            CoreError::Store(format!(
                "invalid finite private reservation status {status}"
            ))
        })?,
        burst_window_epoch: row.get("burst_window_epoch"),
        usage_formula_version: row.get("usage_formula_version"),
        upstream_status: row.get("upstream_status"),
        upstream_error_class: row.get("upstream_error_class"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub(super) async fn select_finite_private_grant<C>(
    client: &C,
    grant_id: &str,
    for_update: bool,
) -> CoreResult<Option<FinitePrivateGrant>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT id, user_id, limit_profile_id, status,
                CASE WHEN current_window_started_at IS NULL THEN NULL
                     ELSE {started} END AS current_window_started_at,
                current_window_used_units, burst_window_epoch,
                {created} AS created_at, {updated} AS updated_at
         FROM finite_private_grants WHERE id = $1{lock}",
        started = rfc3339_col("current_window_started_at"),
        created = rfc3339_col("created_at"),
        updated = rfc3339_col("updated_at"),
        lock = if for_update { " FOR UPDATE" } else { "" },
    );
    client
        .query_opt(&sql, &[&grant_id])
        .await
        .map_err(store_error)?
        .map(|row| finite_private_grant_from_row(&row))
        .transpose()
}

pub(super) async fn select_finite_private_limit_profile<C>(
    client: &C,
    id: &str,
) -> CoreResult<Option<FinitePrivateLimitProfile>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT id, burst_window_seconds, burst_limit_units, weekly_limit_units,
                {created} AS created_at, {updated} AS updated_at
         FROM finite_private_limit_profiles WHERE id = $1",
        created = rfc3339_col("created_at"),
        updated = rfc3339_col("updated_at"),
    );
    Ok(client
        .query_opt(&sql, &[&id])
        .await
        .map_err(store_error)?
        .as_ref()
        .map(finite_private_limit_profile_from_row))
}

pub(super) async fn select_finite_private_reservation<C>(
    client: &C,
    reservation_id: &str,
    for_update: bool,
) -> CoreResult<Option<FinitePrivateReservation>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT id, request_id, api_key_id, grant_id, endpoint, model,
                estimated_usage_units, reserved_usage_units, settled_usage_units,
                settlement_kind, status, usage_formula_version, upstream_status,
                upstream_error_class, burst_window_epoch,
                {created} AS created_at, {updated} AS updated_at
         FROM finite_private_reservations WHERE id = $1{lock}",
        created = rfc3339_col("created_at"),
        updated = rfc3339_col("updated_at"),
        lock = if for_update { " FOR UPDATE" } else { "" },
    );
    client
        .query_opt(&sql, &[&reservation_id])
        .await
        .map_err(store_error)?
        .map(|row| finite_private_reservation_from_row(&row))
        .transpose()
}

/// Resolve the (active api key, active grant) pair for a presented raw key by
/// its hash. An empty/invalid/revoked key or grant yields `None` (a denial),
/// never an error — mirroring `finite_private_key_and_grant`.
pub(super) async fn postgres_finite_private_key_and_grant<C>(
    client: &C,
    presented_api_key: &str,
) -> CoreResult<Option<(FinitePrivateApiKey, FinitePrivateGrant)>>
where
    C: GenericClient + Sync,
{
    let key_hash = match hash_finite_private_api_key(presented_api_key) {
        Ok(hash) => hash,
        Err(CoreError::MissingFinitePrivateApiKey) => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(row) = client
        .query_opt(
            "SELECT id, grant_id, project_id, agent_runtime_id, key_hash, status,
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
             FROM finite_private_api_keys WHERE key_hash = $1",
            &[&key_hash],
        )
        .await
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    let api_key = finite_private_api_key_from_row(&row)?;
    if api_key.status != FinitePrivateApiKeyStatus::Active {
        return Ok(None);
    }
    let Some(grant) = select_finite_private_grant(client, &api_key.grant_id, false).await? else {
        return Ok(None);
    };
    if grant.status != FinitePrivateGrantStatus::Active {
        return Ok(None);
    }
    Ok(Some((api_key, grant)))
}

/// Weekly usage for a grant across the rolling window, summed over its own
/// reservations only (row-scoped by grant_id). Returns the used units and the
/// reset instant (earliest in-window reservation + one week).
pub(super) async fn postgres_finite_private_weekly_usage<C>(
    client: &C,
    grant_id: &str,
    window_start: &str,
    now: &str,
) -> CoreResult<(i64, Option<String>)>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT
           COALESCE(SUM(COALESCE(settled_usage_units, reserved_usage_units)), 0)::BIGINT AS used,
           CASE WHEN MIN(created_at) IS NULL THEN NULL ELSE {earliest} END AS earliest
         FROM finite_private_reservations
         WHERE grant_id = $1
           AND status <> 'denied'
           AND created_at >= $2::text::timestamptz
           AND created_at <= $3::text::timestamptz",
        earliest = rfc3339_col("MIN(created_at)"),
    );
    let row = client
        .query_one(&sql, &[&grant_id, &window_start, &now])
        .await
        .map_err(store_error)?;
    let used_units: i64 = row.get("used");
    let earliest: Option<String> = row.get("earliest");
    let reset_at = earliest
        .map(|earliest| {
            let parsed = parse_time(&earliest)?;
            (parsed + Duration::seconds(crate::FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
                .format(&Rfc3339)
                .map_err(CoreError::from)
        })
        .transpose()?;
    Ok((used_units, reset_at))
}
