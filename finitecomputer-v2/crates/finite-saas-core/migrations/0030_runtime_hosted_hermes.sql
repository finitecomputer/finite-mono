-- Opt-in new-creation bootstrap. Existing runtimes are deliberately not enrolled.
CREATE TABLE IF NOT EXISTS runtime_core_credentials (
  creation_request_id TEXT PRIMARY KEY REFERENCES agent_creation_requests(id),
  agent_runtime_id TEXT UNIQUE REFERENCES agent_runtimes(id) ON DELETE CASCADE,
  source_host_id TEXT NOT NULL,
  source_machine_id TEXT,
  owner_user_id TEXT NOT NULL REFERENCES users(id),
  bootstrap_secret TEXT NOT NULL CHECK (bootstrap_secret ~ '^[0-9a-f]{64}$'),
  token_sha256 TEXT NOT NULL UNIQUE CHECK (token_sha256 ~ '^[0-9a-f]{64}$'),
  lease_sha256 TEXT NOT NULL CHECK (lease_sha256 ~ '^[0-9a-f]{64}$'),
  activated BOOLEAN NOT NULL DEFAULT FALSE,
  revoked BOOLEAN NOT NULL DEFAULT FALSE,
  hosted_enabled BOOLEAN NOT NULL DEFAULT FALSE,
  hosted_generation BIGINT NOT NULL DEFAULT 1 CHECK (hosted_generation > 0),
  hosted_username TEXT,
  hosted_password TEXT,
  hosted_signing_secret TEXT,
  hosted_applied_generation BIGINT,
  hosted_apply_status TEXT NOT NULL DEFAULT 'pending' CHECK (hosted_apply_status IN ('pending','applied','error')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CHECK (NOT hosted_enabled OR (hosted_username IS NOT NULL AND hosted_password IS NOT NULL AND hosted_signing_secret IS NOT NULL)),
  CHECK (hosted_applied_generation IS NULL OR hosted_applied_generation BETWEEN 1 AND hosted_generation)
);
