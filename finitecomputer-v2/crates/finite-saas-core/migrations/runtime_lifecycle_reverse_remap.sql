-- OPERATOR-INITIATED RESCUE ONLY. Do not include this file in CORE_SCHEMA_SQL.
-- Run immediately before rolling Core back across 0021_runtime_lifecycle.sql
-- to a generation whose runtime_control_requests still speaks the flat status
-- vocabulary. It inverts exactly the forward remap 0021 performs:
--   requested                                   -> requested   (unchanged)
--   launching / compute_up / ready              -> running
--   succeeded (kind restart/recover/upgrade)    -> succeeded   (unchanged)
--   stopped (kind stop/destroy)                 -> succeeded
--   failed (+ failure_stage)                    -> failed      (unchanged)
-- `stopped` predates neither vocabulary, but Postgres only ever admitted it
-- through 0021's own `succeeded AND kind IN ('stop','destroy')` rewrite or
-- post-H1 writers (which reach it solely from Stop/Destroy), so targeting it
-- by kind is exact. Any value outside both vocabularies fails the restored
-- legacy CHECK below inside this same transaction: ambiguous state aborts the
-- rescue closed instead of shipping it to the rolled-back generation.
--
-- H1 (0021_runtime_lifecycle.sql): the active statuses below are the post-H1
-- lifecycle vocabulary. Rolling Core back across H1 additionally requires
-- THIS runbook; the companion runtime_upgrade_rollback_rescue.sql (which see)
-- only rewrites the upgrade KIND and must also run before rolling past 0002.
--
-- Like every store statement, the previous generation of Core names its
-- columns on INSERT and SELECT, so the extra NOT NULL DEFAULT 'unknown'
-- failure_stage column stays in place: N-1 readers never project it and N-1
-- writers leave it at its default.

BEGIN;

LOCK TABLE runtime_control_requests IN SHARE ROW EXCLUSIVE MODE;

DO $$
BEGIN
  IF EXISTS (
    SELECT 1
    FROM runtime_control_requests
    WHERE kind = 'upgrade'
      AND status IN ('requested', 'launching', 'compute_up', 'ready')
  ) THEN
    RAISE EXCEPTION
      'runtime lifecycle reverse remap refused: active upgrade requests still exist'
      USING HINT = 'Stop the runner, reconcile provider topology with the compatible generation, and make every upgrade request terminal before retrying.';
  END IF;
END $$;

INSERT INTO finite_private_admin_audit_events (
  id, action, target_type, target_id, grant_id, api_key_id, actor, metadata, created_at
)
SELECT
  'runtime_lifecycle_reverse_remap_' || md5(request.id),
  'runtime.lifecycle.reverse_remap',
  'runtime_control_request',
  request.id,
  NULL,
  NULL,
  'runtime-lifecycle-reverse-remap',
  jsonb_build_object(
    'originalStatus', request.status,
    'originalKind', request.kind,
    'originalFailureStage', request.failure_stage
  ),
  CURRENT_TIMESTAMP
FROM runtime_control_requests AS request
WHERE request.status IN ('launching', 'compute_up', 'ready')
   OR (
        request.status = 'stopped'
        AND request.kind IN ('stop', 'destroy')
      )
ON CONFLICT (id) DO NOTHING;

-- Drop the post-H1 status CHECK before rewriting to 'running', which only the
-- legacy definition admits; guard so reruns of an already-reversed schema are
-- no-ops.
DO $$
DECLARE
  current_definition TEXT;
BEGIN
  SELECT pg_get_constraintdef(constraint_row.oid)
    INTO current_definition
    FROM pg_constraint AS constraint_row
    WHERE constraint_row.conrelid = 'runtime_control_requests'::regclass
      AND constraint_row.conname = 'runtime_control_requests_status_check';

  IF current_definition IS NOT NULL AND position('''launching''' IN current_definition) > 0 THEN
    ALTER TABLE runtime_control_requests
      DROP CONSTRAINT runtime_control_requests_status_check;
  END IF;
END $$;

-- The reverse remap itself. Value-targeted, so rerunning changes nothing.
UPDATE runtime_control_requests
  SET status = 'running',
      updated_at = CURRENT_TIMESTAMP
  WHERE status IN ('launching', 'compute_up', 'ready');
UPDATE runtime_control_requests
  SET status = 'succeeded',
      updated_at = CURRENT_TIMESTAMP
  WHERE status = 'stopped' AND kind IN ('stop', 'destroy');

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
      CHECK (status IN ('requested', 'running', 'succeeded', 'failed'));
  END IF;
END $$;

-- "Active" collapses back to the legacy pair; replace, don't keep, whenever
-- the predicate still carries the post-H1 vocabulary.
DO $$
DECLARE
  index_definition TEXT;
BEGIN
  SELECT pg_get_indexdef(index_row.indexrelid)
    INTO index_definition
    FROM pg_index AS index_row
    JOIN pg_class AS index_class ON index_class.oid = index_row.indexrelid
    WHERE index_class.relname = 'runtime_control_requests_one_active_per_runtime';

  IF index_definition IS NOT NULL AND position('''launching''' IN index_definition) > 0 THEN
    DROP INDEX runtime_control_requests_one_active_per_runtime;
  END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS runtime_control_requests_one_active_per_runtime
  ON runtime_control_requests(agent_runtime_id)
  WHERE status IN ('requested', 'running');

COMMIT;
