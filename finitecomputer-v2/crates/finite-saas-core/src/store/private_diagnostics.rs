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
    ) -> CoreResult<()> {
        let mut client = self.diagnostic_connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.batch_execute("SET LOCAL statement_timeout = '2s'; SET LOCAL lock_timeout = '250ms';")
            .await
            .map_err(store_error)?;
        let valid_reservation =
            postgres_record_finite_private_request_diagnostic(&*tx, input).await?;
        if !valid_reservation {
            return Err(CoreError::FinitePrivateReservationNotFound);
        }
        self.finish(tx).await?;
        Ok(())
    }
}

async fn postgres_record_finite_private_request_diagnostic<C>(
    client: &C,
    input: RecordFinitePrivateRequestDiagnosticInput,
) -> CoreResult<bool>
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
    ) || [
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

    // Reservation-time audit metadata preserves attribution across key reissue.
    // A tie is ambiguous, so it deliberately remains unattributed.
    let result = client
        .query_one(
            "WITH source AS MATERIALIZED (
               SELECT r.id AS reservation_id, r.request_id, r.api_key_id,
                      CASE WHEN issue.event_count = 1 THEN issue.project_id
                           WHEN issue.event_count = 0 AND k.updated_at <= r.created_at THEN k.project_id
                           ELSE NULL END AS project_id,
                      CASE WHEN issue.event_count = 1 THEN issue.agent_runtime_id
                           WHEN issue.event_count = 0 AND k.updated_at <= r.created_at THEN k.agent_runtime_id
                           ELSE NULL END AS agent_runtime_id,
                      r.endpoint, r.model, $3::bigint AS prompt_tokens,
                      $4::bigint AS completion_tokens, $5::bigint AS first_output_ms,
                      $6::bigint AS first_answer_ms, $7::bigint AS duration_ms,
                      $8::text AS termination_reason, $9::text AS measurement_quality,
                      r.upstream_status, r.upstream_error_class, CURRENT_TIMESTAMP AS observed_at
               FROM finite_private_reservations r
               JOIN finite_private_api_keys k ON k.id = r.api_key_id
               LEFT JOIN LATERAL (
                 SELECT COUNT(*) AS event_count,
                        MIN(a.metadata->>'projectId') AS project_id,
                        MIN(a.metadata->>'agentRuntimeId') AS agent_runtime_id
                 FROM finite_private_admin_audit_events a
                 WHERE a.api_key_id = r.api_key_id
                   AND a.action = 'finite_private.api_key.issue'
                   AND a.created_at = (
                     SELECT MAX(latest.created_at)
                     FROM finite_private_admin_audit_events latest
                     WHERE latest.api_key_id = r.api_key_id
                       AND latest.action = 'finite_private.api_key.issue'
                       AND latest.created_at <= r.created_at
                   )
               ) issue ON true
               WHERE r.id = $1 AND r.request_id = $2
                 AND r.created_at >= CURRENT_TIMESTAMP - INTERVAL '7 days'
             ), inserted AS (
               INSERT INTO finite_private_request_diagnostics (
                 reservation_id, request_id, api_key_id, project_id, agent_runtime_id,
                 endpoint, model, prompt_tokens, completion_tokens, first_output_ms,
                 first_answer_ms, duration_ms, termination_reason, measurement_quality,
                 upstream_status, upstream_error_class, observed_at
               )
               SELECT reservation_id, request_id, api_key_id, project_id, agent_runtime_id,
                      endpoint, model, prompt_tokens, completion_tokens, first_output_ms,
                      first_answer_ms, duration_ms, termination_reason, measurement_quality,
                      upstream_status, upstream_error_class, observed_at
               FROM source
               ON CONFLICT (reservation_id) DO NOTHING
               RETURNING reservation_id
             )
             SELECT EXISTS (SELECT 1 FROM source) AS valid_reservation",
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
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(result.get("valid_reservation"))
}
