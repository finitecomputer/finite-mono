use super::*;

impl CoreStore {
    async fn diagnostic_connection(&self) -> CoreResult<Object> {
        tokio::time::timeout(std::time::Duration::from_millis(250), self.connection())
            .await
            .map_err(|_| CoreError::Store("diagnostic connection budget exceeded".into()))?
    }

    #[tracing::instrument(skip(self, input), fields(operation = "record_finite_private_request_diagnostic", reservation_id = %input.reservation_id))]
    pub async fn record_finite_private_request_diagnostic(
        &self,
        input: RecordFinitePrivateRequestDiagnosticInput,
    ) -> CoreResult<FinitePrivateRequestDiagnostic> {
        let mut client = self.diagnostic_connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.batch_execute("SET LOCAL statement_timeout = '2s'; SET LOCAL lock_timeout = '250ms';")
            .await
            .map_err(store_error)?;
        let diagnostic = postgres_record_finite_private_request_diagnostic(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(diagnostic)
    }

    pub async fn finite_private_request_diagnostics(
        &self,
        limit: i64,
    ) -> CoreResult<Vec<FinitePrivateRequestDiagnostic>> {
        Ok(self
            .finite_private_request_diagnostics_page(FinitePrivateRequestDiagnosticQuery {
                limit: Some(limit),
                ..Default::default()
            })
            .await?
            .items)
    }

    pub async fn finite_private_request_diagnostics_page(
        &self,
        query: FinitePrivateRequestDiagnosticQuery,
    ) -> CoreResult<FinitePrivateRequestDiagnosticPage> {
        let mut client = self.diagnostic_connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.batch_execute("SET TRANSACTION READ ONLY; SET LOCAL statement_timeout = '2s'; SET LOCAL lock_timeout = '250ms';")
            .await.map_err(store_error)?;
        let FinitePrivateRequestDiagnosticQuery {
            limit,
            before_observed_at,
            before_reservation_id,
            model,
            api_key_id,
            project_id,
            agent_runtime_id,
        } = query;
        let limit = limit.unwrap_or(100).clamp(1, 1000);
        let rows = tx
            .query(
                &format!(
                    "SELECT d.reservation_id, d.request_id, d.api_key_id, d.grant_id,
                            d.project_id, d.agent_runtime_id, d.endpoint, d.model,
                            d.prompt_tokens, d.completion_tokens, d.first_output_ms,
                            d.first_answer_ms, d.duration_ms, d.termination_reason,
                            d.measurement_quality, {observed} AS observed_at,
                            d.accounting_status,
                            d.settlement_kind, d.upstream_status, d.upstream_error_class
                     FROM finite_private_request_diagnostics d
                     WHERE d.observed_at >= CURRENT_TIMESTAMP - INTERVAL '7 days'
                       AND ($2::text IS NULL OR d.model = $2)
                       AND ($3::text IS NULL OR d.api_key_id = $3)
                       AND ($4::text IS NULL OR d.project_id = $4)
                       AND ($5::text IS NULL OR d.agent_runtime_id = $5)
                       AND ($6::text IS NULL OR d.observed_at < $6::text::timestamptz
                            OR (d.observed_at = $6::text::timestamptz
                                AND d.reservation_id < COALESCE($7::text, '')))
                     ORDER BY d.observed_at DESC, d.reservation_id DESC
                     LIMIT $1",
                    observed = rfc3339_col("d.observed_at"),
                ),
                &[
                    &(limit + 1),
                    &model,
                    &api_key_id,
                    &project_id,
                    &agent_runtime_id,
                    &before_observed_at,
                    &before_reservation_id,
                ],
            )
            .await
            .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        let truncated = rows.len() > limit as usize;
        let mut items = Vec::with_capacity(rows.len().min(limit as usize));
        for row in rows.into_iter().take(limit as usize) {
            items.push(finite_private_request_diagnostic_from_row(&row)?);
        }
        let (next_before_observed_at, next_before_reservation_id) = if truncated {
            items
                .last()
                .map(|item| {
                    (
                        Some(item.observed_at.clone()),
                        Some(item.reservation_id.clone()),
                    )
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
        };
        Ok(FinitePrivateRequestDiagnosticPage {
            items,
            next_before_observed_at,
            next_before_reservation_id,
            truncated,
            retention_days: 7,
            coverage: "core-reserved-only",
        })
    }
}

async fn postgres_record_finite_private_request_diagnostic<C>(
    client: &C,
    input: RecordFinitePrivateRequestDiagnosticInput,
) -> CoreResult<FinitePrivateRequestDiagnostic>
where
    C: GenericClient + Sync,
{
    let reservation_id = trim_to_option(Some(&input.reservation_id))
        .ok_or(CoreError::InvalidFinitePrivateUsageEstimate)?;
    let request_id = trim_to_option(Some(&input.request_id))
        .ok_or(CoreError::InvalidFinitePrivateUsageEstimate)?;
    if !matches!(
        input.termination_reason.as_str(),
        "complete"
            | "upstream_error"
            | "upstream_stream_timeout"
            | "upstream_stream_error"
            | "client_disconnected_or_stream_cancelled"
    ) || !matches!(
        input.measurement_quality.as_str(),
        "observed_usage" | "estimated_usage"
    ) || input.termination_reason.trim().is_empty()
        || input.measurement_quality.trim().is_empty()
        || [
            input.prompt_tokens,
            input.completion_tokens,
            input.first_output_ms,
            input.first_answer_ms,
            input.duration_ms,
        ]
        .into_iter()
        .flatten()
        .any(|value| !(0..=86_400_000).contains(&value))
    {
        return Err(CoreError::InvalidFinitePrivateUsageEstimate);
    }
    // The Core clock is authoritative. Ignore caller timestamps so retries,
    // clock skew, or a compromised limiter cannot extend diagnostic retention.
    let observed_at = current_time_iso()?;
    // Issue audit metadata preserves reservation-time attribution even when
    // the same raw key is later reissued to another Project or revoked. Tied
    // issue timestamps are ambiguous and deliberately remain unattributed.
    let row = client
        .query_opt(
            &format!(
                "SELECT r.id AS reservation_id, r.request_id, r.api_key_id, r.grant_id,
                        CASE WHEN issue.event_count = 1 THEN issue.project_id
                             WHEN issue.event_count = 0 AND k.updated_at <= r.created_at THEN k.project_id
                             ELSE NULL END AS project_id,
                        CASE WHEN issue.event_count = 1 THEN issue.agent_runtime_id
                             WHEN issue.event_count = 0 AND k.updated_at <= r.created_at THEN k.agent_runtime_id
                             ELSE NULL END AS agent_runtime_id,
                        r.endpoint, r.model,
                        $3::bigint AS prompt_tokens, $4::bigint AS completion_tokens,
                        $5::bigint AS first_output_ms, $6::bigint AS first_answer_ms,
                        $7::bigint AS duration_ms, $8::text AS termination_reason,
                        $9::text AS measurement_quality, {observed} AS observed_at,
                        r.status AS accounting_status, r.settled_usage_units, r.settlement_kind,
                        r.upstream_status, r.upstream_error_class
                 FROM finite_private_reservations r
                 JOIN finite_private_api_keys k ON k.id = r.api_key_id
                 LEFT JOIN LATERAL (
                   SELECT COUNT(*) AS event_count,
                          MIN(a.metadata->>'projectId') AS project_id,
                          MIN(a.metadata->>'agentRuntimeId') AS agent_runtime_id
                   FROM finite_private_admin_audit_events a
                   WHERE a.api_key_id = r.api_key_id AND a.action = 'finite_private.api_key.issue'
                     AND a.created_at = (
                       SELECT MAX(latest.created_at) FROM finite_private_admin_audit_events latest
                       WHERE latest.api_key_id = r.api_key_id
                         AND latest.action = 'finite_private.api_key.issue'
                         AND latest.created_at <= r.created_at
                     )
                 ) issue ON true
                 WHERE r.id = $1 AND r.request_id = $2
                   AND r.created_at >= CURRENT_TIMESTAMP - INTERVAL '7 days'",
                observed = rfc3339_col("$10::text::timestamptz"),
            ),
            &[
                &reservation_id,
                &request_id,
                &input.prompt_tokens,
                &input.completion_tokens,
                &input.first_output_ms,
                &input.first_answer_ms,
                &input.duration_ms,
                &input.termination_reason,
                &input.measurement_quality,
                &observed_at,
            ],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::FinitePrivateReservationNotFound)?;
    let reservation_id = row.get::<_, String>("reservation_id");
    let request_id = row.get::<_, String>("request_id");
    let api_key_id = row.get::<_, String>("api_key_id");
    let grant_id = row.get::<_, String>("grant_id");
    let project_id = row.get::<_, Option<String>>("project_id");
    let agent_runtime_id = row.get::<_, Option<String>>("agent_runtime_id");
    let endpoint = row.get::<_, String>("endpoint");
    let model = row.get::<_, String>("model");
    let settled_usage_units = row.get::<_, Option<i64>>("settled_usage_units");
    let accounting_status = row.get::<_, String>("accounting_status");
    let settlement_kind = row.get::<_, Option<String>>("settlement_kind");
    let upstream_status = row.get::<_, Option<i32>>("upstream_status");
    let upstream_error_class = row.get::<_, Option<String>>("upstream_error_class");
    client
        .execute(
            "INSERT INTO finite_private_request_diagnostics (
               reservation_id, request_id, api_key_id, grant_id, project_id,
               agent_runtime_id, endpoint, model, prompt_tokens, completion_tokens,
               first_output_ms, first_answer_ms, duration_ms, termination_reason,
               measurement_quality, observed_at, accounting_status, settlement_kind,
               upstream_status, upstream_error_class, settled_usage_units
             )
             VALUES ($1::text, $2::text, $3::text, $4::text, $5::text, $6::text,
                     $7::text, $8::text, $9::bigint, $10::bigint, $11::bigint,
                     $12::bigint, $13::bigint, $14::text, $15::text,
                     $16::text::timestamptz, $17::text, $18::text, $19::integer, $20::text, $21::bigint)
             ON CONFLICT (reservation_id) DO NOTHING",
            &[
                &reservation_id,
                &request_id,
                &api_key_id,
                &grant_id,
                &project_id,
                &agent_runtime_id,
                &endpoint,
                &model,
                &input.prompt_tokens,
                &input.completion_tokens,
                &input.first_output_ms,
                &input.first_answer_ms,
                &input.duration_ms,
                &input.termination_reason,
                &input.measurement_quality,
                &observed_at,
                &accounting_status,
                &settlement_kind,
                &upstream_status,
                &upstream_error_class,
                &settled_usage_units,
            ],
        )
        .await
        .map_err(store_error)?;
    // Read the persisted row after the upsert so an idempotent retry returns
    // its original Core-clocked observation time rather than a fresh value.
    let persisted = client
        .query_one(
            &format!(
                "SELECT d.reservation_id, d.request_id, d.api_key_id, d.grant_id,
                        d.project_id, d.agent_runtime_id, d.endpoint, d.model,
                        d.prompt_tokens, d.completion_tokens, d.first_output_ms,
                        d.first_answer_ms, d.duration_ms, d.termination_reason,
                        d.measurement_quality, {observed} AS observed_at,
                        d.accounting_status, d.settlement_kind,
                        d.upstream_status, d.upstream_error_class
                 FROM finite_private_request_diagnostics d
                 WHERE d.reservation_id = $1",
                observed = rfc3339_col("d.observed_at"),
            ),
            &[&reservation_id],
        )
        .await
        .map_err(store_error)?;
    finite_private_request_diagnostic_from_row(&persisted)
}

fn finite_private_request_diagnostic_from_row(
    row: &Row,
) -> CoreResult<FinitePrivateRequestDiagnostic> {
    Ok(FinitePrivateRequestDiagnostic {
        reservation_id: row.get("reservation_id"),
        request_id: row.get("request_id"),
        api_key_id: row.get("api_key_id"),
        grant_id: row.get("grant_id"),
        project_id: row.get("project_id"),
        agent_runtime_id: row.get("agent_runtime_id"),
        endpoint: row.get("endpoint"),
        model: row.get("model"),
        prompt_tokens: row.get("prompt_tokens"),
        completion_tokens: row.get("completion_tokens"),
        first_output_ms: row.get("first_output_ms"),
        first_answer_ms: row.get("first_answer_ms"),
        duration_ms: row.get("duration_ms"),
        termination_reason: row.get("termination_reason"),
        measurement_quality: row.get("measurement_quality"),
        observed_at: row.get("observed_at"),
        accounting_status: parse_finite_private_reservation_status(
            &row.get::<_, String>("accounting_status"),
        )
        .ok_or_else(|| CoreError::Store("invalid finite private accounting status".into()))?,
        settlement_kind: row
            .get::<_, Option<String>>("settlement_kind")
            .as_deref()
            .map(|value| {
                parse_finite_private_settlement_kind(value).ok_or_else(|| {
                    CoreError::Store("invalid finite private settlement kind".into())
                })
            })
            .transpose()?,
        upstream_status: row.get("upstream_status"),
        upstream_error_class: row.get("upstream_error_class"),
    })
}
