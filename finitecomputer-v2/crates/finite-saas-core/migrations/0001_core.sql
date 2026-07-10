CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  normalized_email TEXT NOT NULL UNIQUE,
  link_status TEXT NOT NULL CHECK (link_status IN ('pending', 'linked')),
  workos_user_id TEXT UNIQUE,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL,
  CHECK (
    (link_status = 'pending' AND workos_user_id IS NULL)
    OR (link_status = 'linked' AND workos_user_id IS NOT NULL)
  )
);

CREATE TABLE IF NOT EXISTS customer_orgs (
  id TEXT PRIMARY KEY,
  owner_user_id TEXT NOT NULL REFERENCES users(id),
  name TEXT NOT NULL,
  billing_class TEXT NOT NULL CHECK (billing_class IN ('grandfathered', 'off2026', 'standard')),
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS customer_orgs_one_personal_org_per_owner
  ON customer_orgs(owner_user_id);

CREATE TABLE IF NOT EXISTS customer_billing_accounts (
  customer_org_id TEXT PRIMARY KEY REFERENCES customer_orgs(id),
  stripe_customer_id TEXT UNIQUE,
  stripe_subscription_id TEXT UNIQUE,
  stripe_price_id TEXT,
  subscription_status TEXT CHECK (
    subscription_status IN (
      'incomplete',
      'incomplete_expired',
      'trialing',
      'active',
      'past_due',
      'canceled',
      'unpaid',
      'paused'
    )
  ),
  current_period_end TIMESTAMPTZ,
  cancel_at_period_end BOOLEAN NOT NULL DEFAULT FALSE,
  last_stripe_event_id TEXT,
  last_stripe_event_created BIGINT,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL
);

ALTER TABLE customer_billing_accounts
  ADD COLUMN IF NOT EXISTS stripe_customer_id TEXT;

ALTER TABLE customer_billing_accounts
  ADD COLUMN IF NOT EXISTS stripe_subscription_id TEXT;

ALTER TABLE customer_billing_accounts
  ADD COLUMN IF NOT EXISTS stripe_price_id TEXT;

ALTER TABLE customer_billing_accounts
  ADD COLUMN IF NOT EXISTS subscription_status TEXT;

ALTER TABLE customer_billing_accounts
  ADD COLUMN IF NOT EXISTS current_period_end TIMESTAMPTZ;

ALTER TABLE customer_billing_accounts
  ADD COLUMN IF NOT EXISTS cancel_at_period_end BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE customer_billing_accounts
  ADD COLUMN IF NOT EXISTS last_stripe_event_id TEXT;

-- Monotonic ordering signal (Stripe `event.created`, unix seconds) used by the
-- event-ordering guard in `sync_stripe_subscription` to drop stale webhooks.
ALTER TABLE customer_billing_accounts
  ADD COLUMN IF NOT EXISTS last_stripe_event_created BIGINT;

DO $$
BEGIN
  IF EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'customer_billing_accounts_subscription_status_check'
  ) THEN
    ALTER TABLE customer_billing_accounts
      DROP CONSTRAINT customer_billing_accounts_subscription_status_check;
  END IF;

  ALTER TABLE customer_billing_accounts
    ADD CONSTRAINT customer_billing_accounts_subscription_status_check
    CHECK (
      subscription_status IN (
        'incomplete',
        'incomplete_expired',
        'trialing',
        'active',
        'past_due',
        'canceled',
        'unpaid',
        'paused'
      )
    );
END $$;

CREATE TABLE IF NOT EXISTS project_import_candidates (
  id TEXT PRIMARY KEY,
  source_host_id TEXT NOT NULL,
  source_machine_id TEXT NOT NULL,
  source_import_key TEXT NOT NULL UNIQUE,
  owner_email TEXT NOT NULL,
  latest_host_owner_email TEXT,
  pending_user_id TEXT NOT NULL REFERENCES users(id),
  customer_org_id TEXT NOT NULL REFERENCES customer_orgs(id),
  status TEXT NOT NULL CHECK (status IN ('pending', 'claimed', 'admin_review')),
  project_id TEXT,
  agent_runtime_id TEXT,
  claimed_by_user_id TEXT REFERENCES users(id),
  host_facts JSONB NOT NULL,
  known_external_channel_participants JSONB NOT NULL DEFAULT '[]'::jsonb,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL,
  UNIQUE (source_host_id, source_machine_id),
  CHECK (
    (status = 'pending' AND project_id IS NULL AND agent_runtime_id IS NULL AND claimed_by_user_id IS NULL)
    OR (status = 'claimed' AND project_id IS NOT NULL AND agent_runtime_id IS NOT NULL AND claimed_by_user_id IS NOT NULL)
    OR status = 'admin_review'
  )
);

CREATE TABLE IF NOT EXISTS projects (
  id TEXT PRIMARY KEY,
  customer_org_id TEXT NOT NULL REFERENCES customer_orgs(id),
  owner_user_id TEXT NOT NULL REFERENCES users(id),
  display_name TEXT NOT NULL,
  import_candidate_id TEXT UNIQUE REFERENCES project_import_candidates(id),
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS runtime_artifacts (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('oci_image')),
  reference TEXT NOT NULL,
  version_label TEXT NOT NULL,
  source_git_sha TEXT,
  finitec_version TEXT,
  hermes_source_ref TEXT,
  finite_platform_plugin_ref TEXT,
  state_schema_version TEXT NOT NULL,
  base_image TEXT,
  created_at TIMESTAMPTZ NOT NULL,
  promoted_at TIMESTAMPTZ,
  retired_at TIMESTAMPTZ,
  CHECK (promoted_at IS NULL OR retired_at IS NULL OR retired_at >= promoted_at)
);

DO $$
BEGIN
  IF to_regclass('agent_runtimes') IS NOT NULL THEN
    UPDATE agent_runtimes
      SET runtime_artifact_id = NULL
      WHERE runtime_artifact_id IN (
        SELECT id
        FROM runtime_artifacts
        WHERE kind NOT IN ('oci_image')
      );
  END IF;

  DELETE FROM runtime_artifacts
    WHERE kind NOT IN ('oci_image');
END $$;

DO $$
BEGIN
  IF EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'runtime_artifacts_kind_check'
  ) THEN
    ALTER TABLE runtime_artifacts
      DROP CONSTRAINT runtime_artifacts_kind_check;
  END IF;

  ALTER TABLE runtime_artifacts
    ADD CONSTRAINT runtime_artifacts_kind_check
    CHECK (kind IN ('oci_image'));
END $$;

CREATE TABLE IF NOT EXISTS agent_runtimes (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  source_host_id TEXT NOT NULL,
  source_machine_id TEXT NOT NULL,
  source_import_key TEXT NOT NULL UNIQUE,
  runtime_artifact_id TEXT REFERENCES runtime_artifacts(id),
  state_schema_version TEXT,
  host_facts JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL
);

ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS runtime_artifact_id TEXT REFERENCES runtime_artifacts(id);

ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS state_schema_version TEXT;

CREATE TABLE IF NOT EXISTS runtime_relay_credentials (
  agent_runtime_id TEXT PRIMARY KEY REFERENCES agent_runtimes(id) ON DELETE CASCADE,
  token_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS project_runtime_links (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  agent_runtime_id TEXT NOT NULL REFERENCES agent_runtimes(id),
  active BOOLEAN NOT NULL,
  created_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS project_runtime_links_one_active_runtime
  ON project_runtime_links(project_id)
  WHERE active;

CREATE TABLE IF NOT EXISTS chat_identities (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  kind TEXT NOT NULL CHECK (kind IN ('hosted_web')),
  device_id TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  UNIQUE (user_id, kind, device_id)
);

CREATE TABLE IF NOT EXISTS project_room_memberships (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  chat_identity_id TEXT NOT NULL REFERENCES chat_identities(id),
  role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
  created_at TIMESTAMPTZ NOT NULL,
  UNIQUE (project_id, chat_identity_id)
);

CREATE TABLE IF NOT EXISTS runtime_status_snapshots (
  agent_runtime_id TEXT PRIMARY KEY REFERENCES agent_runtimes(id),
  status TEXT NOT NULL CHECK (status IN ('online', 'offline', 'stale', 'unknown')),
  last_heartbeat_at TIMESTAMPTZ,
  runtime_host TEXT NOT NULL,
  active_inference_profile TEXT,
  hermes_available BOOLEAN,
  updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS inference_profiles (
  id TEXT PRIMARY KEY,
  customer_org_id TEXT REFERENCES customer_orgs(id),
  project_id TEXT REFERENCES projects(id),
  profile_key TEXT NOT NULL,
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  is_default BOOLEAN NOT NULL DEFAULT FALSE,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL,
  CHECK (
    (customer_org_id IS NOT NULL AND project_id IS NULL)
    OR (customer_org_id IS NULL AND project_id IS NOT NULL)
  )
);

CREATE TABLE IF NOT EXISTS agent_creation_entitlements (
  id TEXT PRIMARY KEY,
  customer_org_id TEXT NOT NULL REFERENCES customer_orgs(id),
  allowed_new_agent_runtimes INTEGER NOT NULL CHECK (allowed_new_agent_runtimes >= 0),
  launch_code TEXT,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL
);

-- One entitlement row per org: the standard-billing path does
-- `INSERT ... ON CONFLICT (customer_org_id)`, which requires a unique constraint
-- on that column. The table shipped with only a PK on `id`, so every
-- standard-billing agent creation failed with "there is no unique or exclusion
-- constraint matching the ON CONFLICT specification". Both entitlement helpers
-- already derive the row id from the org and treat one-per-org as invariant, so
-- this matches intent. Added via the same idempotent DO-block pattern used for
-- the constraint changes above so it applies cleanly to the deployed prod DB.
DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'agent_creation_entitlements_customer_org_id_key'
  ) THEN
    ALTER TABLE agent_creation_entitlements
      ADD CONSTRAINT agent_creation_entitlements_customer_org_id_key
      UNIQUE (customer_org_id);
  END IF;
END $$;

CREATE TABLE IF NOT EXISTS agent_creation_requests (
  id TEXT PRIMARY KEY,
  customer_org_id TEXT NOT NULL REFERENCES customer_orgs(id),
  owner_user_id TEXT NOT NULL REFERENCES users(id),
  project_id TEXT NOT NULL UNIQUE REFERENCES projects(id),
  idempotency_key TEXT NOT NULL,
  display_name TEXT NOT NULL,
  runner_class TEXT NOT NULL DEFAULT 'phala' CHECK (runner_class IN ('local_docker', 'apple_container', 'kata', 'phala', 'enclavia')),
  profile_picture_url TEXT,
  status TEXT NOT NULL CHECK (status IN ('requested', 'launching', 'running', 'failed', 'cancelled')),
  requested_launch_code TEXT,
  agent_runtime_id TEXT REFERENCES agent_runtimes(id),
  runner_id TEXT,
  lease_token TEXT,
  lease_expires_at TIMESTAMPTZ,
  failure_message TEXT,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL,
  UNIQUE (owner_user_id, idempotency_key)
);

ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS runner_id TEXT;

ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS lease_token TEXT;

ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ;

ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS failure_message TEXT;

ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS runner_class TEXT NOT NULL DEFAULT 'phala';

ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS profile_picture_url TEXT;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conname = 'agent_creation_requests_runner_class_check'
  ) THEN
    ALTER TABLE agent_creation_requests
      ADD CONSTRAINT agent_creation_requests_runner_class_check
      CHECK (runner_class IN ('local_docker', 'apple_container', 'kata', 'phala', 'enclavia'));
  END IF;
END $$;

-- Partition key for the agent-creation lease queue. Agent creation is a shared
-- new-sandbox pool with no per-request host today, so this is NULL by default
-- (any runner may claim it). When a request is routed to a specific source
-- host, a runner declaring that host claims it; a runner leases only rows whose
-- target is NULL or equal to its own host. This replaces the banned global
-- claim across all rows with a per-source-host partition, still using
-- FOR UPDATE SKIP LOCKED. Nullable + IF NOT EXISTS so it re-applies to prod.
ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS target_source_host_id TEXT;

CREATE INDEX IF NOT EXISTS agent_creation_requests_lease_partition_idx
  ON agent_creation_requests(status, target_source_host_id, created_at, id);

CREATE TABLE IF NOT EXISTS runtime_control_requests (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  agent_runtime_id TEXT NOT NULL REFERENCES agent_runtimes(id),
  source_host_id TEXT NOT NULL,
  source_machine_id TEXT NOT NULL,
  requested_by_user_id TEXT NOT NULL REFERENCES users(id),
  kind TEXT NOT NULL CHECK (kind IN ('restart', 'recover_known_good_chat_runtime', 'stop', 'destroy')),
  status TEXT NOT NULL CHECK (status IN ('requested', 'running', 'succeeded', 'failed')),
  runner_id TEXT,
  lease_token TEXT,
  lease_expires_at TIMESTAMPTZ,
  failure_message TEXT,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL,
  completed_at TIMESTAMPTZ
);

DO $$
BEGIN
  IF EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'runtime_control_requests_kind_check'
  ) THEN
    ALTER TABLE runtime_control_requests
      DROP CONSTRAINT runtime_control_requests_kind_check;
  END IF;

  ALTER TABLE runtime_control_requests
    ADD CONSTRAINT runtime_control_requests_kind_check
    CHECK (kind IN ('restart', 'recover_known_good_chat_runtime', 'stop', 'destroy'));
END $$;

CREATE INDEX IF NOT EXISTS runtime_control_requests_pending_idx
  ON runtime_control_requests(status, source_host_id, created_at, id);

CREATE TABLE IF NOT EXISTS source_host_relays (
  source_host_id TEXT PRIMARY KEY,
  url TEXT NOT NULL,
  admin_token TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS finite_private_limit_profiles (
  id TEXT PRIMARY KEY,
  burst_window_seconds BIGINT NOT NULL CHECK (burst_window_seconds > 0),
  burst_limit_units BIGINT NOT NULL CHECK (burst_limit_units > 0),
  weekly_limit_units BIGINT CHECK (weekly_limit_units IS NULL OR weekly_limit_units > 0),
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS finite_private_grants (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  limit_profile_id TEXT NOT NULL REFERENCES finite_private_limit_profiles(id),
  status TEXT NOT NULL CHECK (status IN ('active', 'revoked')),
  current_window_started_at TIMESTAMPTZ,
  current_window_used_units BIGINT NOT NULL DEFAULT 0 CHECK (current_window_used_units >= 0),
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL,
  UNIQUE (user_id)
);

CREATE TABLE IF NOT EXISTS finite_private_api_keys (
  id TEXT PRIMARY KEY,
  grant_id TEXT NOT NULL REFERENCES finite_private_grants(id),
  project_id TEXT REFERENCES projects(id),
  agent_runtime_id TEXT REFERENCES agent_runtimes(id),
  key_hash TEXT NOT NULL UNIQUE,
  status TEXT NOT NULL CHECK (status IN ('active', 'revoked')),
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS finite_private_admin_audit_events (
  id TEXT PRIMARY KEY,
  action TEXT NOT NULL,
  target_type TEXT NOT NULL,
  target_id TEXT NOT NULL,
  grant_id TEXT REFERENCES finite_private_grants(id),
  api_key_id TEXT REFERENCES finite_private_api_keys(id),
  actor TEXT NOT NULL,
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS finite_private_admin_audit_events_created_at
  ON finite_private_admin_audit_events(created_at, id);

CREATE TABLE IF NOT EXISTS finite_private_reservations (
  id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL,
  api_key_id TEXT NOT NULL REFERENCES finite_private_api_keys(id),
  grant_id TEXT NOT NULL REFERENCES finite_private_grants(id),
  endpoint TEXT NOT NULL,
  model TEXT NOT NULL,
  estimated_usage_units BIGINT NOT NULL CHECK (estimated_usage_units > 0),
  reserved_usage_units BIGINT NOT NULL CHECK (reserved_usage_units >= 0),
  settled_usage_units BIGINT CHECK (settled_usage_units IS NULL OR settled_usage_units >= 0),
  settlement_kind TEXT CHECK (settlement_kind IN ('actual', 'estimate')),
  status TEXT NOT NULL CHECK (status IN ('reserved', 'settled', 'denied')),
  usage_formula_version TEXT NOT NULL,
  upstream_status INTEGER,
  upstream_error_class TEXT,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL,
  UNIQUE (api_key_id, request_id)
);
