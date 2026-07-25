-- A cold relocation is a specialized, operator-created launch request for an
-- existing stopped Runtime. Core keeps the source binding authoritative until
-- the target Runner verifies staged state and the same Agent Principal.

ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS relocation_spec JSONB;

-- The original table allowed one creation row per Project. Relocation keeps
-- that original creation as history and appends an operator transaction, so
-- retain the invariant only for ordinary creation rows.
ALTER TABLE agent_creation_requests
  DROP CONSTRAINT IF EXISTS agent_creation_requests_project_id_key;

CREATE UNIQUE INDEX IF NOT EXISTS agent_creation_requests_one_primary_creation_per_project
  ON agent_creation_requests(project_id)
  WHERE relocation_spec IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS agent_creation_requests_one_active_relocation_per_runtime
  ON agent_creation_requests(agent_runtime_id)
  WHERE relocation_spec IS NOT NULL
    AND status IN ('requested', 'launching');

COMMENT ON COLUMN agent_creation_requests.relocation_spec IS
  'Operator-only cold relocation contract; target launch verifies staged state and Agent Principal before replacing the Runtime source binding.';
