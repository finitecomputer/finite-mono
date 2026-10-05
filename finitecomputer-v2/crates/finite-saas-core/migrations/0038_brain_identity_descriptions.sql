-- Brain identity descriptions v1 (FIN-122): which exact Nostr key belongs to
-- which Core account, and which Brain audiences an account has agreed may see
-- its contact. Purely additive and safe to reapply at every Core startup like
-- the rest of the schema concat. No existing row is rewritten.
--
-- Writer: POST /api/core/internal/v1/brain-account-observations (trusted
-- dashboard backend + verified WorkOS session), one transaction per operation.
-- Reader: POST /api/core/internal/v1/brain-identity-descriptions (Brain server).
-- Neither table grants Brain access; Brain authority stays in Brain.

-- An existing hosted human key observed for a stable Core account. At most one
-- active account per key; a conflicting claim is refused, never replaced.
CREATE TABLE IF NOT EXISTS account_brain_principals (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  public_key_hex TEXT NOT NULL CHECK (public_key_hex ~ '^[0-9a-f]{64}$'),
  source TEXT NOT NULL CHECK (source IN ('hosted_device_observation')),
  issuer TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('active', 'retired')),
  first_observed_at TIMESTAMPTZ NOT NULL,
  last_observed_at TIMESTAMPTZ NOT NULL,
  revision BIGINT NOT NULL CHECK (revision > 0)
);

CREATE UNIQUE INDEX IF NOT EXISTS account_brain_principals_one_active_key
  ON account_brain_principals(public_key_hex)
  WHERE status = 'active';

-- Bounded per-account key reads (ORDER BY key LIMIT n) use this index.
CREATE INDEX IF NOT EXISTS account_brain_principals_active_by_user
  ON account_brain_principals(user_id, public_key_hex)
  WHERE status = 'active';

-- One disclosure scope per account, exact Brain server origin and Brain id.
-- Revocation sets revoked_at; a later qualifying action opens a new row.
CREATE TABLE IF NOT EXISTS account_brain_sharing_scopes (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  brain_server TEXT NOT NULL,
  brain_id TEXT NOT NULL,
  issuer TEXT NOT NULL,
  established_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  revision BIGINT NOT NULL CHECK (revision > 0)
);

CREATE UNIQUE INDEX IF NOT EXISTS account_brain_sharing_scopes_one_active
  ON account_brain_sharing_scopes(user_id, brain_server, brain_id)
  WHERE revoked_at IS NULL;

-- Idempotency receipts, scoped to the account so one account can neither
-- replay nor probe another's operation id. No credential is stored.
CREATE TABLE IF NOT EXISTS brain_account_observation_receipts (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  operation_id TEXT NOT NULL,
  issuer TEXT NOT NULL,
  payload_sha256 TEXT NOT NULL CHECK (payload_sha256 ~ '^[0-9a-f]{64}$'),
  outcome TEXT NOT NULL CHECK (outcome IN ('recorded', 'unchanged')),
  recorded_at TIMESTAMPTZ NOT NULL,
  UNIQUE (user_id, operation_id)
);

-- Exact-key agent lookups match the canonical npub text by equality.
CREATE INDEX IF NOT EXISTS agent_runtimes_health_reporting_npub
  ON agent_runtimes(health_reporting_npub)
  WHERE health_reporting_npub IS NOT NULL;
