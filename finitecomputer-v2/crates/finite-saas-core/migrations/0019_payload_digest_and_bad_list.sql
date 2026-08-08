-- Payload telemetry enrichment (payload-generations hardening, batch B):
-- the shell's /healthz now serves the current generation's verified tree
-- digest and its bad-listed version labels; the runner forwards both. The
-- digest is immutable artifact evidence next to the mutable version label;
-- the bad list makes "N agents refused vX" readable from the fleet view.
-- Neither feeds the gen fence — labels stay the comparison key, and there
-- is deliberately no auto-demotion.

ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS payload_digest TEXT;
ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS payload_bad_versions JSONB;

COMMENT ON COLUMN agent_runtimes.payload_digest IS
  'The verified tree digest of the payload generation the runtime last reported running; NULL when the shell has no verified digest on record (pre-shell images, post-rollback).';
COMMENT ON COLUMN agent_runtimes.payload_bad_versions IS
  'JSON array of version labels this runtime''s shell has bad-listed (failed flip health gates); NULL until a shell-image runtime reports. Evidence for the fleet view, not an auto-demotion input.';
