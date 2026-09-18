SELECT COALESCE(json_agg(record), '[]'::json) FROM (
  SELECT (EXTRACT(EPOCH FROM d.observed_at) * 1000000000)::numeric(30,0)::text AS "timestampNs",
    json_build_object(
      'version', 1, 'observedAtUnixSeconds', EXTRACT(EPOCH FROM d.observed_at)::bigint,
      'reservationId', d.reservation_id, 'requestId', d.request_id,
      'apiKeyId', d.api_key_id, 'grantId', d.grant_id,
      'projectId', COALESCE(d.project_id, 'shared-unattributed'),
      'agentRuntimeId', COALESCE(d.agent_runtime_id, 'shared-unattributed'),
      'endpoint', d.endpoint, 'model', d.model,
      'promptTokens', d.prompt_tokens, 'completionTokens', d.completion_tokens,
      'firstOutputMs', d.first_output_ms, 'firstAnswerMs', d.first_answer_ms,
      'durationMs', d.duration_ms, 'terminationReason', d.termination_reason,
      'measurementQuality', d.measurement_quality, 'observedAt', d.observed_at,
      'accountingStatus', d.accounting_status, 'settlementKind', d.settlement_kind,
      'upstreamStatus', d.upstream_status, 'upstreamErrorClass', d.upstream_error_class,
      'settledUsageUnits', d.settled_usage_units
    ) AS event
  FROM finite_private_request_diagnostics d
  WHERE d.exported_at IS NULL AND d.observed_at >= CURRENT_TIMESTAMP - INTERVAL '7 days'
  ORDER BY d.observed_at, d.reservation_id LIMIT 500
) record;
