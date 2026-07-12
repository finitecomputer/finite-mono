-- Expand generation for explicit, provider-neutral Runtime control
-- capabilities. The column remains nullable so N-1 Core and Runner binaries
-- can continue reading and writing Agent Runtime rows during rollout.

ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS runtime_capabilities JSONB;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conrelid = 'agent_runtimes'::regclass
      AND conname = 'agent_runtimes_runtime_capabilities_object_check'
  ) THEN
    ALTER TABLE agent_runtimes
      ADD CONSTRAINT agent_runtimes_runtime_capabilities_object_check
      CHECK (
        runtime_capabilities IS NULL
        OR jsonb_typeof(runtime_capabilities) = 'object'
      );
  END IF;
END $$;

-- Backfill only the already-proven Core-created Kata population. This is an
-- explicit compatibility record, not provider/artifact inference in a
-- browser. Recover-known-good remains restart-only today, and Runtime
-- Retirement has no recovery-safe implementation, so both stay false.
UPDATE agent_runtimes AS runtime
  SET runtime_capabilities = jsonb_build_object(
    'schema', 'runtime_capabilities.v1',
    'capabilities', jsonb_build_object(
      'restart', true,
      'recover_known_good_chat', false,
      'runtime_upgrade', true,
      'stop', true,
      'runtime_retirement', false
    )
  )
  WHERE runtime.runtime_capabilities IS NULL
    AND EXISTS (
      SELECT 1
      FROM agent_creation_requests AS request
      WHERE request.agent_runtime_id = runtime.id
        AND request.status = 'running'
        AND COALESCE(request.placement_runner_class, request.runner_class) = 'kata'
    );
