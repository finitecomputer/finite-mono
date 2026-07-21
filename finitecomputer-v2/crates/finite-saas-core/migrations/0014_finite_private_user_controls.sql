-- User-facing Finite Private controls. Epochs prevent a late settlement from
-- charging a freshly reset burst window. Claim tables make the daily reset
-- and threshold notices atomic across dashboard and agent-runtime callers.
ALTER TABLE finite_private_grants
  ADD COLUMN IF NOT EXISTS burst_window_epoch BIGINT NOT NULL DEFAULT 0;

ALTER TABLE finite_private_reservations
  ADD COLUMN IF NOT EXISTS burst_window_epoch BIGINT NOT NULL DEFAULT 0;

-- Status is read after each successful turn. This prefix also avoids the old
-- full-table weekly grant scan while the epoch term makes the new burst-window
-- sum selective.
CREATE INDEX IF NOT EXISTS finite_private_reservations_grant_status_epoch_created_idx
  ON finite_private_reservations
  (grant_id, status, burst_window_epoch, created_at);

CREATE TABLE IF NOT EXISTS finite_private_daily_resets (
  grant_id TEXT NOT NULL REFERENCES finite_private_grants(id) ON DELETE CASCADE,
  reset_day DATE NOT NULL,
  claimed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (grant_id, reset_day)
);

CREATE TABLE IF NOT EXISTS finite_private_notice_claims (
  grant_id TEXT NOT NULL REFERENCES finite_private_grants(id) ON DELETE CASCADE,
  burst_window_epoch BIGINT NOT NULL,
  threshold_remaining_percent SMALLINT NOT NULL
    CHECK (threshold_remaining_percent IN (25, 10)),
  claimed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (grant_id, burst_window_epoch, threshold_remaining_percent)
);
