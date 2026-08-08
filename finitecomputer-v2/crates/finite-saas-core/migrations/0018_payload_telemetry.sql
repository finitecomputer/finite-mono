-- Payload generations telemetry and the gen fence (payload-generations plan,
-- M4). Agents converge themselves, so Core records what each runtime last
-- reported running (shell /healthz -> runner -> Core) instead of commanding
-- it, and each channel head remembers its predecessor so "on the head or the
-- previous head" is checkable without history tables.

ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS payload_version_label TEXT;
ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS shell_version TEXT;
ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS release_channel TEXT;
ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS payload_reported_at TIMESTAMPTZ;

COMMENT ON COLUMN agent_runtimes.payload_version_label IS
  'The payload generation the runtime last reported running; NULL until a shell-image runtime reports.';
COMMENT ON COLUMN agent_runtimes.shell_version IS
  'The finite-shell version the runtime last reported; NULL for pre-shell images.';
COMMENT ON COLUMN agent_runtimes.release_channel IS
  'The release channel the runtime last reported being on; NULL for pre-shell images.';
COMMENT ON COLUMN agent_runtimes.payload_reported_at IS
  'When the runner last forwarded this runtime''s payload telemetry.';

ALTER TABLE release_channel_heads
  ADD COLUMN IF NOT EXISTS previous_artifact_id TEXT REFERENCES runtime_artifacts(id);

COMMENT ON COLUMN release_channel_heads.previous_artifact_id IS
  'The artifact this channel pointed at before the current head; agents still on it are converged (N-1), anything older is a repair signal.';
