-- Runner-ferried standing readiness (2026-08 audit synthesis, H1 slice 3).
-- The runner polls each live runtime's /contact on a bounded cadence and posts
-- one health report per runtime to Core; Core keeps only the latest report on
-- the runtime row and projects readiness at read time (no sweeper, no history
-- table). Purely additive columns, safe to reapply at every Core startup just
-- like the rest of the schema concat.

ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS health_reported_at TIMESTAMPTZ;
ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS health_observed_at TIMESTAMPTZ;
ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS health_ready BOOLEAN;
ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS health_reason TEXT;
ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS health_report_interval_seconds INTEGER;
ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS health_reporting_npub TEXT;

COMMENT ON COLUMN agent_runtimes.health_reported_at IS
  'When Core recorded the latest runner-ferried health report (Core clock; freshness is measured from this).';
COMMENT ON COLUMN agent_runtimes.health_observed_at IS
  'When the runner last read the runtime''s /contact (runner clock; evidence only, never a freshness input).';
COMMENT ON COLUMN agent_runtimes.health_ready IS
  'The latest report''s ready flag; NULL until the runner''s standing poller first reports.';
COMMENT ON COLUMN agent_runtimes.health_reason IS
  'The latest report''s bounded not-ready reason (guest-reported error or the runner''s unreachable marker).';
COMMENT ON COLUMN agent_runtimes.health_report_interval_seconds IS
  'The reporting runner''s poll cadence; the read-time projection declares staleness after 3x this interval.';
COMMENT ON COLUMN agent_runtimes.health_reporting_npub IS
  'The Agent Principal npub the runner pinned and observed for the latest report (anti-port-squat evidence).';
