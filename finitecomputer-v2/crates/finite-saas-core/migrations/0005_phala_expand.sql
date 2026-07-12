-- Expand generation for Core-owned Hosting Tier, placement, RuntimeSpec, and
-- provider identity. Every new field remains nullable so the previous Core and
-- Runner revision can continue reading and controlling rows during rollout.

ALTER TABLE customer_billing_accounts
  ADD COLUMN IF NOT EXISTS hosting_tier TEXT;

ALTER TABLE agent_creation_entitlements
  ADD COLUMN IF NOT EXISTS hosting_tier TEXT;

ALTER TABLE launch_code_batches
  ADD COLUMN IF NOT EXISTS hosting_tier TEXT;

ALTER TABLE projects
  ADD COLUMN IF NOT EXISTS hosting_tier TEXT;

ALTER TABLE projects
  ADD COLUMN IF NOT EXISTS placement_runner_class TEXT;

ALTER TABLE projects
  ADD COLUMN IF NOT EXISTS runtime_resource_class TEXT;

ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS hosting_tier TEXT;

-- Keep the legacy runner_class column for the N-1 worker. The new placement
-- column is deliberately distinct so contraction can happen in a later
-- generation without changing the meaning of old rows under an old binary.
ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS placement_runner_class TEXT;

ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS runtime_resource_class TEXT;

ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS desired_runtime_artifact_id TEXT REFERENCES runtime_artifacts(id);

ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS runtime_spec JSONB;

ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS placement_runner_class TEXT;

ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS runtime_resource_class TEXT;

ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS provider_runtime_handle JSONB;

ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS provider_runtime_handle_history JSONB;

ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS contact_endpoint TEXT;

-- The old schema silently selected Phala when an old writer omitted the
-- legacy field. Keep the column for compatibility, but remove that default so
-- every new writer must make an explicit Core-owned decision.
ALTER TABLE agent_creation_requests
  ALTER COLUMN runner_class DROP DEFAULT;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conrelid = 'customer_billing_accounts'::regclass
      AND conname = 'customer_billing_accounts_hosting_tier_check'
  ) THEN
    ALTER TABLE customer_billing_accounts
      ADD CONSTRAINT customer_billing_accounts_hosting_tier_check
      CHECK (hosting_tier IS NULL OR hosting_tier IN ('standard', 'confidential'));
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conrelid = 'agent_creation_entitlements'::regclass
      AND conname = 'agent_creation_entitlements_hosting_tier_check'
  ) THEN
    ALTER TABLE agent_creation_entitlements
      ADD CONSTRAINT agent_creation_entitlements_hosting_tier_check
      CHECK (hosting_tier IS NULL OR hosting_tier IN ('standard', 'confidential'));
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conrelid = 'launch_code_batches'::regclass
      AND conname = 'launch_code_batches_hosting_tier_check'
  ) THEN
    ALTER TABLE launch_code_batches
      ADD CONSTRAINT launch_code_batches_hosting_tier_check
      CHECK (hosting_tier IS NULL OR hosting_tier IN ('standard', 'confidential'));
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conrelid = 'projects'::regclass
      AND conname = 'projects_hosting_tier_check'
  ) THEN
    ALTER TABLE projects
      ADD CONSTRAINT projects_hosting_tier_check
      CHECK (hosting_tier IS NULL OR hosting_tier IN ('standard', 'confidential'));
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conrelid = 'agent_creation_requests'::regclass
      AND conname = 'agent_creation_requests_hosting_tier_check'
  ) THEN
    ALTER TABLE agent_creation_requests
      ADD CONSTRAINT agent_creation_requests_hosting_tier_check
      CHECK (hosting_tier IS NULL OR hosting_tier IN ('standard', 'confidential'));
  END IF;
END $$;

DO $$
DECLARE
  table_name TEXT;
BEGIN
  FOREACH table_name IN ARRAY ARRAY['projects', 'agent_creation_requests', 'agent_runtimes']
  LOOP
    IF NOT EXISTS (
      SELECT 1 FROM pg_constraint
      WHERE conrelid = table_name::regclass
        AND conname = table_name || '_placement_runner_class_check'
    ) THEN
      EXECUTE format(
        'ALTER TABLE %I ADD CONSTRAINT %I CHECK (placement_runner_class IS NULL OR placement_runner_class IN (''local_docker'', ''apple_container'', ''kata'', ''phala'', ''enclavia''))',
        table_name,
        table_name || '_placement_runner_class_check'
      );
    END IF;

    IF NOT EXISTS (
      SELECT 1 FROM pg_constraint
      WHERE conrelid = table_name::regclass
        AND conname = table_name || '_runtime_resource_class_check'
    ) THEN
      EXECUTE format(
        'ALTER TABLE %I ADD CONSTRAINT %I CHECK (runtime_resource_class IS NULL OR runtime_resource_class IN (''vcpu4_memory8_gib'', ''vcpu2_memory4_gib''))',
        table_name,
        table_name || '_runtime_resource_class_check'
      );
    END IF;

    IF NOT EXISTS (
      SELECT 1 FROM pg_constraint
      WHERE conrelid = table_name::regclass
        AND conname = table_name || '_placement_complete_check'
    ) THEN
      EXECUTE format(
        'ALTER TABLE %I ADD CONSTRAINT %I CHECK ((placement_runner_class IS NULL) = (runtime_resource_class IS NULL))',
        table_name,
        table_name || '_placement_complete_check'
      );
    END IF;
  END LOOP;
END $$;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conrelid = 'agent_creation_requests'::regclass
      AND conname = 'agent_creation_requests_runtime_spec_object_check'
  ) THEN
    ALTER TABLE agent_creation_requests
      ADD CONSTRAINT agent_creation_requests_runtime_spec_object_check
      CHECK (runtime_spec IS NULL OR jsonb_typeof(runtime_spec) = 'object');
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conrelid = 'agent_runtimes'::regclass
      AND conname = 'agent_runtimes_provider_handle_object_check'
  ) THEN
    ALTER TABLE agent_runtimes
      ADD CONSTRAINT agent_runtimes_provider_handle_object_check
      CHECK (
        provider_runtime_handle IS NULL
        OR jsonb_typeof(provider_runtime_handle) = 'object'
      );
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conrelid = 'agent_runtimes'::regclass
      AND conname = 'agent_runtimes_provider_handle_history_array_check'
  ) THEN
    ALTER TABLE agent_runtimes
      ADD CONSTRAINT agent_runtimes_provider_handle_history_array_check
      CHECK (
        provider_runtime_handle_history IS NULL
        OR jsonb_typeof(provider_runtime_handle_history) = 'array'
      );
  END IF;
END $$;

-- Every pre-existing commercial/sponsored admission fact is Standard. This is
-- an access/product backfill only; it does not move a running Runtime.
UPDATE customer_billing_accounts
  SET hosting_tier = 'standard'
  WHERE hosting_tier IS NULL;

UPDATE agent_creation_entitlements
  SET hosting_tier = 'standard'
  WHERE hosting_tier IS NULL;

UPDATE launch_code_batches
  SET hosting_tier = 'standard'
  WHERE hosting_tier IS NULL;

UPDATE agent_creation_requests
  SET hosting_tier = 'standard'
  WHERE hosting_tier IS NULL;

UPDATE projects
  SET hosting_tier = 'standard'
  WHERE hosting_tier IS NULL;

-- An unlaunched legacy row cannot prove provider placement. The retired SQL
-- default made many such rows say Phala without a provider resource ever
-- existing, so deterministically route them to the Standard/Kata policy.
UPDATE agent_creation_requests
  SET runner_class = 'kata'
  WHERE agent_runtime_id IS NULL;

-- Preserve proven existing placement from the running request. Kata and the
-- experimental Phala path have known retained resource shapes. Other legacy
-- adapters keep using their legacy fields until a later explicit migration;
-- nullable expand fields avoid inventing a resource promise for them.
UPDATE agent_creation_requests
  SET placement_runner_class = runner_class,
      runtime_resource_class = CASE runner_class
        WHEN 'kata' THEN 'vcpu4_memory8_gib'
        WHEN 'phala' THEN 'vcpu2_memory4_gib'
      END
  WHERE placement_runner_class IS NULL
    AND runtime_resource_class IS NULL
    AND runner_class IN ('kata', 'phala');

UPDATE projects AS project
  SET placement_runner_class = request.placement_runner_class,
      runtime_resource_class = request.runtime_resource_class
  FROM agent_creation_requests AS request
  WHERE request.project_id = project.id
    AND project.placement_runner_class IS NULL
    AND project.runtime_resource_class IS NULL
    AND request.placement_runner_class IS NOT NULL
    AND request.runtime_resource_class IS NOT NULL;

UPDATE agent_runtimes AS runtime
  SET placement_runner_class = request.placement_runner_class,
      runtime_resource_class = request.runtime_resource_class
  FROM agent_creation_requests AS request
  WHERE request.agent_runtime_id = runtime.id
    AND runtime.placement_runner_class IS NULL
    AND runtime.runtime_resource_class IS NULL
    AND request.placement_runner_class IS NOT NULL
    AND request.runtime_resource_class IS NOT NULL;
