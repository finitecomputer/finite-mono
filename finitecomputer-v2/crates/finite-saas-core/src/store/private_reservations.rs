use super::*;

impl CoreStore {
    pub async fn reserve_finite_private_usage(
        &self,
        input: ReserveFinitePrivateUsageInput,
    ) -> CoreResult<FinitePrivateUsageDecision> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let decision = postgres_reserve_finite_private_usage(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(decision)
    }

    pub async fn settle_finite_private_reservation(
        &self,
        input: SettleFinitePrivateReservationInput,
    ) -> CoreResult<SettleFinitePrivateReservationResult> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_settle_finite_private_reservation(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }
}

async fn postgres_reserve_finite_private_usage<C>(
    client: &C,
    input: ReserveFinitePrivateUsageInput,
) -> CoreResult<FinitePrivateUsageDecision>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let now_time = parse_time(&now)?;
    let request_id = trim_to_option(Some(&input.request_id)).unwrap_or_else(|| {
        crate::id_from_parts("fp_request", &[&now, &input.endpoint, &input.model])
    });
    let dashboard_url = trim_to_option(Some(&input.dashboard_url))
        .unwrap_or_else(|| "https://finite.computer/dashboard".to_string());
    if input.estimated_usage_units <= 0
        || input.estimated_prompt_tokens < 0
        || input.estimated_completion_tokens < 0
    {
        return Err(CoreError::InvalidFinitePrivateUsageEstimate);
    }
    let Some((api_key, _)) =
        postgres_finite_private_key_and_grant(client, &input.presented_api_key).await?
    else {
        return Ok(crate::finite_private_denial(
            request_id,
            dashboard_url,
            "Finite Private API key is invalid or revoked.",
            "invalid_api_key",
            None,
            None,
        ));
    };
    // Re-read the grant FOR UPDATE to serialize concurrent reservations.
    let grant = select_finite_private_grant(client, &api_key.grant_id, true)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let profile = select_finite_private_limit_profile(client, &grant.limit_profile_id)
        .await?
        .ok_or(CoreError::FinitePrivateLimitProfileNotFound)?;

    let reservation_id = crate::finite_private_reservation_id_for(&api_key.id, &request_id);
    let (weekly_used_units, weekly_reset_at) = if profile.weekly_limit_units.is_some() {
        let window_start = (now_time
            - Duration::seconds(crate::FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
        .format(&Rfc3339)?;
        postgres_finite_private_weekly_usage(client, &grant.id, &window_start, &now).await?
    } else {
        // The shipped profiles have no rolling weekly limit. Avoid scanning
        // the reservation ledger when its result cannot affect admission or
        // the public response.
        (0, None)
    };

    if let Some(existing) =
        select_finite_private_reservation(client, &reservation_id, false).await?
    {
        return Ok(crate::finite_private_allow_decision(
            existing.id,
            &profile,
            profile.burst_limit_units - grant.current_window_used_units,
            crate::finite_private_window_reset_at(&grant, &profile, now_time)?,
            profile
                .weekly_limit_units
                .map(|limit| limit - weekly_used_units),
            weekly_reset_at,
        ));
    }

    let (window_started_at, current_used_units, reset_at) =
        crate::finite_private_active_window(&grant, &profile, now_time)?;
    let begins_new_epoch = crate::finite_private_begins_new_epoch(&grant, &window_started_at)?;
    let reservation_epoch = grant.burst_window_epoch + i64::from(begins_new_epoch);
    let remaining_before = profile.burst_limit_units - current_used_units;
    if input.estimated_usage_units > remaining_before {
        let retry_after = (parse_time(&reset_at)? - now_time).whole_seconds().max(0);
        let message =
            crate::finite_private_limit_reached_message("burst window", &reset_at, retry_after);
        return Ok(crate::finite_private_denial(
            request_id,
            dashboard_url,
            &message,
            "burst_window_limit_exceeded",
            Some(retry_after),
            Some(reset_at),
        ));
    }
    if let Some(weekly_limit_units) = profile.weekly_limit_units {
        let weekly_remaining_before = weekly_limit_units - weekly_used_units;
        if input.estimated_usage_units > weekly_remaining_before {
            let reset_at = weekly_reset_at.clone().unwrap_or_else(|| {
                (now_time + Duration::seconds(crate::FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| now.clone())
            });
            let retry_after = (parse_time(&reset_at)? - now_time).whole_seconds().max(0);
            let message =
                crate::finite_private_limit_reached_message("weekly", &reset_at, retry_after);
            return Ok(crate::finite_private_denial(
                request_id,
                dashboard_url,
                &message,
                "weekly_limit_exceeded",
                Some(retry_after),
                Some(reset_at),
            ));
        }
    }

    let new_used_units = current_used_units + input.estimated_usage_units;
    client
        .execute(
            "UPDATE finite_private_grants
             SET current_window_started_at = $2::text::timestamptz,
                 current_window_used_units = $3,
                 burst_window_epoch = $4,
                 updated_at = $5::text::timestamptz
             WHERE id = $1",
            &[
                &grant.id,
                &window_started_at,
                &new_used_units,
                &reservation_epoch,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    let endpoint = crate::trim_or_fallback(&input.endpoint, "/v1/chat/completions");
    let model = crate::trim_or_fallback(&input.model, "kimi-k2-6");
    let usage_formula_version =
        crate::trim_or_fallback(&input.usage_formula_version, "2026-05-26.v1");
    client
        .execute(
            "INSERT INTO finite_private_reservations (
               id, request_id, api_key_id, grant_id, endpoint, model,
               estimated_usage_units, reserved_usage_units, settled_usage_units,
               settlement_kind, status, usage_formula_version, upstream_status,
               upstream_error_class, burst_window_epoch, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7, NULL, NULL, 'reserved', $8, NULL, NULL,
                     $9, $10::text::timestamptz, $10::text::timestamptz)",
            &[
                &reservation_id,
                &request_id,
                &api_key.id,
                &grant.id,
                &endpoint,
                &model,
                &input.estimated_usage_units,
                &usage_formula_version,
                &reservation_epoch,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(crate::finite_private_allow_decision(
        reservation_id,
        &profile,
        profile.burst_limit_units - new_used_units,
        reset_at,
        profile
            .weekly_limit_units
            .map(|limit| limit - (weekly_used_units + input.estimated_usage_units)),
        weekly_reset_at.or_else(|| {
            profile.weekly_limit_units.map(|_| {
                (now_time + Duration::seconds(crate::FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| now.clone())
            })
        }),
    ))
}

async fn postgres_settle_finite_private_reservation<C>(
    client: &C,
    input: SettleFinitePrivateReservationInput,
) -> CoreResult<SettleFinitePrivateReservationResult>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let reservation_id = trim_to_option(Some(&input.reservation_id))
        .ok_or(CoreError::FinitePrivateReservationNotFound)?;
    let request_id = trim_to_option(Some(&input.request_id))
        .ok_or(CoreError::FinitePrivateReservationNotFound)?;
    let existing = select_finite_private_reservation(client, &reservation_id, true)
        .await?
        .ok_or(CoreError::FinitePrivateReservationNotFound)?;
    if existing.request_id != request_id {
        return Err(CoreError::FinitePrivateReservationNotFound);
    }
    let settled_units = input
        .usage_units
        .unwrap_or(existing.reserved_usage_units)
        .max(0);
    if existing.status == FinitePrivateReservationStatus::Settled {
        let formula = crate::trim_or_fallback(
            &input.usage_formula_version,
            &existing.usage_formula_version,
        );
        if existing.settled_usage_units == Some(settled_units)
            && existing.settlement_kind == Some(input.settlement)
            && existing.usage_formula_version == formula
            && existing.upstream_status == input.upstream_status
            && existing.upstream_error_class
                == trim_to_option(input.upstream_error_class.as_deref())
        {
            return Ok(SettleFinitePrivateReservationResult {
                settled: true,
                reservation_id,
            });
        }
        return Err(CoreError::FinitePrivateReservationAlreadySettled);
    }
    let delta = settled_units - existing.reserved_usage_units;
    // Adjust the grant's burst usage by the settle delta (clamped at 0).
    client
        .execute(
            "UPDATE finite_private_grants
             SET current_window_used_units = GREATEST(current_window_used_units + $2, 0),
                 updated_at = $3::text::timestamptz
             WHERE id = $1 AND burst_window_epoch = $4",
            &[
                &existing.grant_id,
                &delta,
                &now,
                &existing.burst_window_epoch,
            ],
        )
        .await
        .map_err(store_error)?;
    let usage_formula_version = crate::trim_or_fallback(
        &input.usage_formula_version,
        &existing.usage_formula_version,
    );
    let upstream_error_class = trim_to_option(input.upstream_error_class.as_deref());
    client
        .execute(
            "UPDATE finite_private_reservations
             SET status = 'settled',
                 settled_usage_units = $2,
                 settlement_kind = $3,
                 usage_formula_version = $4,
                 upstream_status = $5,
                 upstream_error_class = $6,
                 updated_at = $7::text::timestamptz
             WHERE id = $1",
            &[
                &reservation_id,
                &settled_units,
                &input.settlement.as_str(),
                &usage_formula_version,
                &input.upstream_status,
                &upstream_error_class,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(SettleFinitePrivateReservationResult {
        settled: true,
        reservation_id,
    })
}
