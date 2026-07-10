-- OPERATOR-INITIATED RESCUE ONLY. Do not include this file in CORE_SCHEMA_SQL.
-- Run immediately before rolling Core back to a generation that cannot parse
-- RuntimeControlKind::Upgrade. The audit row preserves the original operation
-- and target before the compatibility rewrite.
BEGIN;

LOCK TABLE runtime_control_requests IN SHARE ROW EXCLUSIVE MODE;

DO $$
BEGIN
  IF EXISTS (
    SELECT 1
    FROM runtime_control_requests
    WHERE kind = 'upgrade'
      AND status IN ('requested', 'running')
  ) THEN
    RAISE EXCEPTION
      'runtime upgrade rollback rescue refused: active upgrade requests still exist'
      USING HINT = 'Stop the runner, reconcile provider topology with the compatible generation, and make every upgrade request terminal before retrying.';
  END IF;
END $$;

INSERT INTO finite_private_admin_audit_events (
  id, action, target_type, target_id, grant_id, api_key_id, actor, metadata, created_at
)
SELECT
  'runtime_upgrade_rescue_' || md5(request.id),
  'runtime.upgrade.rollback_rescue',
  'runtime_control_request',
  request.id,
  NULL,
  NULL,
  'runtime-upgrade-rollback-rescue',
  jsonb_build_object(
    'originalKind', request.kind,
    'originalStatus', request.status,
    'targetRuntimeArtifactId', request.target_runtime_artifact_id
  ),
  CURRENT_TIMESTAMP
FROM runtime_control_requests AS request
WHERE request.kind = 'upgrade'
ON CONFLICT (id) DO NOTHING;

UPDATE runtime_control_requests
SET kind = 'restart',
    updated_at = CURRENT_TIMESTAMP
WHERE kind = 'upgrade'
  AND status IN ('succeeded', 'failed');

ALTER TABLE runtime_control_requests
  DROP CONSTRAINT IF EXISTS runtime_control_requests_kind_check;
ALTER TABLE runtime_control_requests
  ADD CONSTRAINT runtime_control_requests_kind_check
  CHECK (kind IN ('restart', 'recover_known_good_chat_runtime', 'stop', 'destroy'));

COMMIT;
