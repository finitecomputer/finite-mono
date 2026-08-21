-- Live, idempotent migration for the canonical runtime-control lifecycle
-- state machine (2026-08 audit item H1). Production already has
-- 0001..0019, so this file must remain safe to reapply at every Core
-- startup just like the rest of the schema concat.
--
-- The legacy flat statuses map onto the machine exactly once:
--   requested                                   -> requested
--   running                                     -> launching
--   succeeded (kind restart/recover/upgrade)    -> succeeded
--   succeeded (kind stop/destroy)               -> stopped
--   failed                                      -> failed + failure_stage 'unknown'
-- The mapping is total over the legacy CHECK vocabulary; a value outside it
-- aborts the new CHECK below and fails Core startup closed, which is the
-- intended behavior if the pre-deploy production census (see the H1 PR)
-- missed a status.

ALTER TABLE runtime_control_requests
  ADD COLUMN IF NOT EXISTS failure_stage TEXT;

-- Drop the legacy status CHECK only when it is still the legacy definition,
-- so the remap below is never blocked by it and reapplies stay no-ops.
DO $$
DECLARE
  current_definition TEXT;
BEGIN
  SELECT pg_get_constraintdef(constraint_row.oid)
    INTO current_definition
    FROM pg_constraint AS constraint_row
    WHERE constraint_row.conrelid = 'runtime_control_requests'::regclass
      AND constraint_row.conname = 'runtime_control_requests_status_check';

  IF current_definition IS NOT NULL AND position('''launching''' IN current_definition) = 0 THEN
    ALTER TABLE runtime_control_requests
      DROP CONSTRAINT runtime_control_requests_status_check;
  END IF;
END $$;

-- The remap itself. Value-targeted, so reapplying changes nothing.
UPDATE runtime_control_requests
  SET status = 'stopped'
  WHERE status = 'succeeded' AND kind IN ('stop', 'destroy');
UPDATE runtime_control_requests
  SET status = 'launching'
  WHERE status = 'running';

-- Every row carries a stage value; 'unknown' is the legacy/N-1 marker.
-- New writers name the real stage when they write 'failed'.
UPDATE runtime_control_requests
  SET failure_stage = 'unknown'
  WHERE failure_stage IS NULL;
ALTER TABLE runtime_control_requests
  ALTER COLUMN failure_stage SET DEFAULT 'unknown';
ALTER TABLE runtime_control_requests
  ALTER COLUMN failure_stage SET NOT NULL;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_constraint AS constraint_row
    WHERE constraint_row.conrelid = 'runtime_control_requests'::regclass
      AND constraint_row.conname = 'runtime_control_requests_status_check'
  ) THEN
    ALTER TABLE runtime_control_requests
      ADD CONSTRAINT runtime_control_requests_status_check
      CHECK (status IN ('requested', 'launching', 'compute_up', 'ready', 'succeeded', 'stopped', 'failed'));
  END IF;
END $$;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_constraint AS constraint_row
    WHERE constraint_row.conrelid = 'runtime_control_requests'::regclass
      AND constraint_row.conname = 'runtime_control_requests_failure_stage_check'
  ) THEN
    ALTER TABLE runtime_control_requests
      ADD CONSTRAINT runtime_control_requests_failure_stage_check
      CHECK (failure_stage IN ('launch', 'compute', 'readiness', 'retirement', 'unknown'));
  END IF;
END $$;

-- "Active" now spans every non-terminal state. The predicate changed, so
-- the old index must be replaced, not merely kept; guard the drop so
-- reapplies do not rebuild it.
DO $$
DECLARE
  index_definition TEXT;
BEGIN
  SELECT pg_get_indexdef(index_row.indexrelid)
    INTO index_definition
    FROM pg_index AS index_row
    JOIN pg_class AS index_class ON index_class.oid = index_row.indexrelid
    WHERE index_class.relname = 'runtime_control_requests_one_active_per_runtime';

  IF index_definition IS NOT NULL AND position('''launching''' IN index_definition) = 0 THEN
    DROP INDEX runtime_control_requests_one_active_per_runtime;
  END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS runtime_control_requests_one_active_per_runtime
  ON runtime_control_requests(agent_runtime_id)
  WHERE status IN ('requested', 'launching', 'compute_up', 'ready');
