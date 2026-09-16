-- Core originates and retains bootstrap material for retryable launch delivery.
-- Initially empty: older runtimes are not enrolled by this migration.
CREATE TABLE IF NOT EXISTS runtime_core_credentials (
  creation_request_id TEXT PRIMARY KEY REFERENCES agent_creation_requests(id),
  agent_runtime_id TEXT UNIQUE REFERENCES agent_runtimes(id) ON DELETE CASCADE,
  source_host_id TEXT NOT NULL,
  source_machine_id TEXT,
  bootstrap_secret TEXT NOT NULL CHECK (bootstrap_secret ~ '^[0-9a-f]{64}$'),
  token_sha256 TEXT NOT NULL UNIQUE CHECK (token_sha256 ~ '^[0-9a-f]{64}$'),
  lease_sha256 TEXT NOT NULL CHECK (lease_sha256 ~ '^[0-9a-f]{64}$'),
  activated BOOLEAN NOT NULL DEFAULT FALSE,
  revoked BOOLEAN NOT NULL DEFAULT FALSE,
  endpoint_generation BIGINT NOT NULL DEFAULT 0 CHECK (endpoint_generation >= 0),
  endpoint_id TEXT CHECK (endpoint_id ~ '^[0-9a-f]{64}$'),
  relay_url TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

ALTER TABLE runtime_core_credentials ADD COLUMN IF NOT EXISTS hosted_access_enabled BOOLEAN NOT NULL DEFAULT FALSE;
CREATE TABLE IF NOT EXISTS runtime_peer_admissions (
  creation_request_id TEXT NOT NULL REFERENCES runtime_core_credentials(creation_request_id) ON DELETE CASCADE,
  endpoint_generation BIGINT NOT NULL,
  peer_id TEXT NOT NULL CHECK (peer_id ~ '^[0-9a-f]{64}$'),
  workos_user_id TEXT NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (creation_request_id, endpoint_generation, peer_id)
);
