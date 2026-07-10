-- Live, idempotent migration for explicit Runtime Upgrade operations. Production
-- already has 0001_core.sql, so this file must remain safe to reapply at every
-- Core startup just like the original schema bootstrap.
ALTER TABLE runtime_control_requests
  ADD COLUMN IF NOT EXISTS target_runtime_artifact_id TEXT REFERENCES runtime_artifacts(id);

DO $$
DECLARE
  current_definition TEXT;
BEGIN
  SELECT pg_get_constraintdef(constraint_row.oid)
    INTO current_definition
    FROM pg_constraint AS constraint_row
    WHERE constraint_row.conrelid = 'runtime_control_requests'::regclass
      AND constraint_row.conname = 'runtime_control_requests_kind_check';

  IF current_definition IS NULL THEN
    ALTER TABLE runtime_control_requests
      ADD CONSTRAINT runtime_control_requests_kind_check
      CHECK (kind IN ('restart', 'recover_known_good_chat_runtime', 'upgrade', 'stop', 'destroy'));
  ELSIF position('''upgrade''' IN current_definition) = 0 THEN
    ALTER TABLE runtime_control_requests
      DROP CONSTRAINT runtime_control_requests_kind_check;

    ALTER TABLE runtime_control_requests
      ADD CONSTRAINT runtime_control_requests_kind_check
      CHECK (kind IN ('restart', 'recover_known_good_chat_runtime', 'upgrade', 'stop', 'destroy'));
  END IF;
END $$;

-- Serialize all lifecycle mutations for one Runtime, not merely duplicates of
-- the same kind. Core also locks the Runtime row before enqueueing; this index
-- is the durable invariant and catches any future writer that bypasses Core.
CREATE UNIQUE INDEX IF NOT EXISTS runtime_control_requests_one_active_per_runtime
  ON runtime_control_requests(agent_runtime_id)
  WHERE status IN ('requested', 'running');
