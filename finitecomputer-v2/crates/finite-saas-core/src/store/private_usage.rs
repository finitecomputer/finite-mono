use super::*;

impl CoreStore {
    pub async fn finite_private_usage_status_for_api_key(
        &self,
        presented_api_key: &str,
        claim_notice: bool,
        now: Option<String>,
    ) -> CoreResult<Option<FinitePrivateUsageStatus>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_finite_private_usage_status_for_api_key(
            &*tx,
            presented_api_key,
            claim_notice,
            now,
        )
        .await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn finite_private_usage_status_for_workos_user(
        &self,
        workos_user_id: &str,
        now: Option<String>,
    ) -> CoreResult<Option<FinitePrivateUsageStatus>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_finite_private_usage_status_for_workos_user(&*tx, workos_user_id, now).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn claim_finite_private_daily_reset_for_api_key(
        &self,
        presented_api_key: &str,
        now: Option<String>,
    ) -> CoreResult<FinitePrivateDailyResetResult> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_claim_finite_private_daily_reset_for_api_key(&*tx, presented_api_key, now)
                .await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn claim_finite_private_daily_reset_for_workos_user(
        &self,
        workos_user_id: &str,
        now: Option<String>,
    ) -> CoreResult<Option<FinitePrivateDailyResetResult>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_claim_finite_private_daily_reset_for_workos_user(&*tx, workos_user_id, now)
                .await?;
        self.finish(tx).await?;
        Ok(result)
    }
}

async fn postgres_finite_private_grant_id_for_workos_user<C>(
    client: &C,
    workos_user_id: &str,
) -> CoreResult<Option<String>>
where
    C: GenericClient + Sync,
{
    Ok(client
        .query_opt(
            "SELECT fpg.id
             FROM finite_private_grants fpg
             JOIN users usr ON usr.id = fpg.user_id
             WHERE usr.workos_user_id = $1 AND fpg.status = 'active'",
            &[&workos_user_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| row.get("id")))
}

async fn postgres_finite_private_usage_status_for_api_key<C>(
    client: &C,
    presented_api_key: &str,
    claim_notice: bool,
    now: Option<String>,
) -> CoreResult<Option<FinitePrivateUsageStatus>>
where
    C: GenericClient + Sync,
{
    let Some((_, grant)) = postgres_finite_private_key_and_grant(client, presented_api_key).await?
    else {
        return Ok(None);
    };
    postgres_finite_private_usage_status_for_grant(client, &grant.id, claim_notice, now)
        .await
        .map(Some)
}

async fn postgres_finite_private_usage_status_for_workos_user<C>(
    client: &C,
    workos_user_id: &str,
    now: Option<String>,
) -> CoreResult<Option<FinitePrivateUsageStatus>>
where
    C: GenericClient + Sync,
{
    let Some(grant_id) =
        postgres_finite_private_grant_id_for_workos_user(client, workos_user_id).await?
    else {
        return Ok(None);
    };
    postgres_finite_private_usage_status_for_grant(client, &grant_id, false, now)
        .await
        .map(Some)
}

async fn postgres_finite_private_usage_status_for_grant<C>(
    client: &C,
    grant_id: &str,
    claim_notice: bool,
    now: Option<String>,
) -> CoreResult<FinitePrivateUsageStatus>
where
    C: GenericClient + Sync,
{
    let now = now.unwrap_or(current_time_iso()?);
    let now_time = parse_time(&now)?;
    let grant = select_finite_private_grant(client, grant_id, false)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    if grant.status != FinitePrivateGrantStatus::Active {
        return Err(CoreError::FinitePrivateGrantNotActive);
    }
    let profile = select_finite_private_limit_profile(client, &grant.limit_profile_id)
        .await?
        .ok_or(CoreError::FinitePrivateLimitProfileNotFound)?;
    let (window_started_at, current_used_units, reset_at) =
        crate::finite_private_active_window(&grant, &profile, now_time)?;
    let begins_new_epoch = crate::finite_private_begins_new_epoch(&grant, &window_started_at)?;
    let epoch = grant.burst_window_epoch + i64::from(begins_new_epoch);
    let settled_used_units: i64 = client
        .query_one(
            "SELECT COALESCE(SUM(settled_usage_units), 0)::bigint AS used_units
             FROM finite_private_reservations
             WHERE grant_id = $1
               AND burst_window_epoch = $2
               AND status = 'settled'
               AND created_at >= $3::text::timestamptz",
            &[&grant.id, &epoch, &window_started_at],
        )
        .await
        .map_err(store_error)?
        .get("used_units");
    let notice = if claim_notice {
        postgres_claim_finite_private_usage_notice(
            client,
            &grant.id,
            epoch,
            settled_used_units,
            &profile,
            &reset_at,
            &now,
        )
        .await?
    } else {
        None
    };
    let daily_reset_used = client
        .query_opt(
            "SELECT 1
             FROM finite_private_daily_resets
             WHERE grant_id = $1
               AND reset_day = ($2::text::timestamptz AT TIME ZONE 'UTC')::date",
            &[&grant.id, &now],
        )
        .await
        .map_err(store_error)?
        .is_some();
    Ok(FinitePrivateUsageStatus {
        burst_limit_units: profile.burst_limit_units,
        burst_used_units: current_used_units.max(0),
        burst_remaining_units: (profile.burst_limit_units - current_used_units).max(0),
        burst_reset_at: reset_at,
        free_daily_reset_available: !daily_reset_used,
        free_daily_reset_available_again_at: crate::finite_private_next_daily_reset_at(now_time)?,
        notice,
    })
}

async fn postgres_claim_finite_private_usage_notice<C>(
    client: &C,
    grant_id: &str,
    epoch: i64,
    settled_used_units: i64,
    profile: &FinitePrivateLimitProfile,
    reset_at: &str,
    now: &str,
) -> CoreResult<Option<FinitePrivateUsageNotice>>
where
    C: GenericClient + Sync,
{
    let remaining = (profile.burst_limit_units - settled_used_units).max(0);
    let threshold: Option<i16> =
        if i128::from(remaining) * 100 <= i128::from(profile.burst_limit_units) * 10 {
            Some(10)
        } else if i128::from(remaining) * 100 <= i128::from(profile.burst_limit_units) * 25 {
            Some(25)
        } else {
            None
        };
    let Some(threshold) = threshold else {
        return Ok(None);
    };
    if threshold == 10 {
        client
            .execute(
                "INSERT INTO finite_private_notice_claims (
                   grant_id, burst_window_epoch, threshold_remaining_percent, claimed_at
                 ) VALUES ($1, $2, 25, $3::text::timestamptz)
                 ON CONFLICT DO NOTHING",
                &[&grant_id, &epoch, &now],
            )
            .await
            .map_err(store_error)?;
    }
    let claimed = client
        .query_opt(
            "INSERT INTO finite_private_notice_claims (
               grant_id, burst_window_epoch, threshold_remaining_percent, claimed_at
             ) VALUES ($1, $2, $3, $4::text::timestamptz)
             ON CONFLICT DO NOTHING
             RETURNING threshold_remaining_percent",
            &[&grant_id, &epoch, &threshold, &now],
        )
        .await
        .map_err(store_error)?
        .is_some();
    if !claimed {
        return Ok(None);
    }
    let retry_after = (parse_time(reset_at)? - parse_time(now)?)
        .whole_seconds()
        .max(0);
    Ok(Some(FinitePrivateUsageNotice {
        threshold_remaining_percent: i64::from(threshold),
        message: format!(
            "You have {threshold}% of your Finite Private burst limit remaining. Your usage resets at {reset_at} ({}).",
            crate::finite_private_retry_after_label(retry_after)
        ),
    }))
}

async fn postgres_claim_finite_private_daily_reset_for_api_key<C>(
    client: &C,
    presented_api_key: &str,
    now: Option<String>,
) -> CoreResult<FinitePrivateDailyResetResult>
where
    C: GenericClient + Sync,
{
    let Some((_, grant)) = postgres_finite_private_key_and_grant(client, presented_api_key).await?
    else {
        return Err(CoreError::InvalidFinitePrivateApiKey);
    };
    postgres_claim_finite_private_daily_reset_for_grant(client, &grant.id, now).await
}

async fn postgres_claim_finite_private_daily_reset_for_workos_user<C>(
    client: &C,
    workos_user_id: &str,
    now: Option<String>,
) -> CoreResult<Option<FinitePrivateDailyResetResult>>
where
    C: GenericClient + Sync,
{
    let Some(grant_id) =
        postgres_finite_private_grant_id_for_workos_user(client, workos_user_id).await?
    else {
        return Ok(None);
    };
    postgres_claim_finite_private_daily_reset_for_grant(client, &grant_id, now)
        .await
        .map(Some)
}

async fn postgres_claim_finite_private_daily_reset_for_grant<C>(
    client: &C,
    grant_id: &str,
    now: Option<String>,
) -> CoreResult<FinitePrivateDailyResetResult>
where
    C: GenericClient + Sync,
{
    let now = now.unwrap_or(current_time_iso()?);
    let grant = select_finite_private_grant(client, grant_id, true)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    if grant.status != FinitePrivateGrantStatus::Active {
        return Err(CoreError::FinitePrivateGrantNotActive);
    }
    let performed = client
        .query_opt(
            "INSERT INTO finite_private_daily_resets (grant_id, reset_day, claimed_at)
             VALUES (
               $1,
               ($2::text::timestamptz AT TIME ZONE 'UTC')::date,
               $2::text::timestamptz
             )
             ON CONFLICT DO NOTHING
             RETURNING grant_id",
            &[&grant_id, &now],
        )
        .await
        .map_err(store_error)?
        .is_some();
    if performed {
        client
            .execute(
                "UPDATE finite_private_grants
                 SET current_window_started_at = $2::text::timestamptz,
                     current_window_used_units = 0,
                     burst_window_epoch = burst_window_epoch + 1,
                     updated_at = $2::text::timestamptz
                 WHERE id = $1",
                &[&grant_id, &now],
            )
            .await
            .map_err(store_error)?;
    }
    let status =
        postgres_finite_private_usage_status_for_grant(client, grant_id, false, Some(now)).await?;
    Ok(FinitePrivateDailyResetResult { performed, status })
}
