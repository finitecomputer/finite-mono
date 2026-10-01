-- Snapshot the responsible grant user at admission, not the human who initiated
-- a turn. Historical and previous-binary writes remain explicitly unattributed.
-- No ownership lookup or backfill may rewrite this historical observation.
ALTER TABLE finite_private_reservations
  ADD COLUMN IF NOT EXISTS usage_user_id TEXT;

ALTER TABLE finite_private_request_diagnostics
  ADD COLUMN IF NOT EXISTS usage_user_id TEXT;
