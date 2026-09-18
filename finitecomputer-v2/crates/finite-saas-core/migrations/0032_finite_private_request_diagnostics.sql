-- Bounded operator diagnostics are separate from the accounting ledger. The
-- ledger remains authoritative for the Runaway Guard and is never pruned by
-- this relation.
CREATE TABLE IF NOT EXISTS finite_private_request_diagnostics (
  reservation_id TEXT PRIMARY KEY REFERENCES finite_private_reservations(id) ON DELETE CASCADE,
  request_id TEXT NOT NULL,
  api_key_id TEXT NOT NULL,
  project_id TEXT,
  agent_runtime_id TEXT,
  endpoint TEXT NOT NULL,
  model TEXT NOT NULL,
  prompt_tokens BIGINT CHECK (prompt_tokens IS NULL OR prompt_tokens >= 0),
  completion_tokens BIGINT CHECK (completion_tokens IS NULL OR completion_tokens >= 0),
  first_output_ms BIGINT CHECK (first_output_ms IS NULL OR first_output_ms >= 0),
  first_answer_ms BIGINT CHECK (first_answer_ms IS NULL OR first_answer_ms >= 0),
  duration_ms BIGINT CHECK (duration_ms IS NULL OR duration_ms >= 0),
  termination_reason TEXT NOT NULL,
  measurement_quality TEXT NOT NULL,
  upstream_status INTEGER,
  upstream_error_class TEXT,
  observed_at TIMESTAMPTZ NOT NULL,
  exported_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS finite_private_request_diagnostics_observed_at
  ON finite_private_request_diagnostics(observed_at DESC, reservation_id DESC);

CREATE INDEX IF NOT EXISTS finite_private_request_diagnostics_pending
  ON finite_private_request_diagnostics(observed_at, reservation_id) WHERE exported_at IS NULL;

-- The existing key-issue audit is the historical association authority.
CREATE INDEX IF NOT EXISTS finite_private_key_issue_attribution
  ON finite_private_admin_audit_events(api_key_id, created_at DESC)
  WHERE action = 'finite_private.api_key.issue';
