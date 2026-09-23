-- Expand provider vocabulary only. Existing placement is immutable: admission
-- policy changes never migrate an existing request, Project, or Runtime.
DO $$
DECLARE
  target TEXT;
BEGIN
  FOREACH target IN ARRAY ARRAY['projects', 'agent_creation_requests', 'agent_runtimes', 'agent_creation_provider_operations']
  LOOP
    IF EXISTS (
      SELECT 1 FROM pg_constraint
      WHERE conrelid = target::regclass
        AND conname = left(target || '_placement_runner_class_check', 63)
        AND pg_get_constraintdef(oid) LIKE '%substrate%'
    ) THEN CONTINUE; END IF;
    EXECUTE format('ALTER TABLE %I DROP CONSTRAINT IF EXISTS %I',
      target, target || '_placement_runner_class_check');
    EXECUTE format(
      'ALTER TABLE %I ADD CONSTRAINT %I CHECK (placement_runner_class IS NULL OR placement_runner_class IN (''local_docker'', ''apple_container'', ''kata'', ''phala'', ''enclavia'', ''substrate''))',
      target, target || '_placement_runner_class_check');
  END LOOP;
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conrelid = 'agent_creation_requests'::regclass
      AND conname = 'agent_creation_requests_runner_class_check'
      AND pg_get_constraintdef(oid) LIKE '%substrate%'
  ) THEN
  ALTER TABLE agent_creation_requests DROP CONSTRAINT agent_creation_requests_runner_class_check;
  ALTER TABLE agent_creation_requests ADD CONSTRAINT agent_creation_requests_runner_class_check
    CHECK (runner_class IN ('local_docker', 'apple_container', 'kata', 'phala', 'enclavia', 'substrate'));
  END IF;
END $$;
