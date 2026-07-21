-- One immutable, typed recovery snapshot receipt gates Runtime Retirement
-- offboarding. This table intentionally contains no repository credentials,
-- URLs, user names, or mutable lifecycle state.
CREATE TABLE IF NOT EXISTS runtime_retirement_snapshots (
  request_id text PRIMARY KEY REFERENCES runtime_control_requests(id),
  project_id text NOT NULL REFERENCES projects(id),
  agent_runtime_id text NOT NULL REFERENCES agent_runtimes(id),
  durable_state_id text NOT NULL,
  runtime_artifact_id text NOT NULL REFERENCES runtime_artifacts(id),
  schema_version text NOT NULL,
  backend text NOT NULL,
  locator text NOT NULL,
  zip_bytes bigint NOT NULL CHECK (zip_bytes > 0),
  zip_sha256 text NOT NULL CHECK (length(zip_sha256) = 64),
  manifest_sha256 text NOT NULL CHECK (length(manifest_sha256) = 64),
  created_at text NOT NULL,
  verified_at text NOT NULL,
  recovery_authority_id text NOT NULL,
  retention_policy text NOT NULL,
  stored_at timestamptz NOT NULL,
  UNIQUE (agent_runtime_id),
  CHECK (schema_version = 'runtime_retirement_snapshot.v1'),
  CHECK (backend = 'borg'),
  CHECK (retention_policy = 'indefinite_until_purge')
);

CREATE UNIQUE INDEX IF NOT EXISTS runtime_retirement_snapshots_locator_idx
  ON runtime_retirement_snapshots (backend, locator);
