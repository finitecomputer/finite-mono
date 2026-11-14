-- Operator-created Agents: an admin creates an ordinary Agent owned by an
-- account named by email, including a pending account that has never signed
-- in. Each row is the durable audit record and the idempotency key of one
-- such creation. The Project, creation request, membership and entitlement
-- it points at are ordinary rows; nothing else reads this table to decide
-- how that Agent behaves.
--
-- Writer: CoreStore::admin_create_agent_for_owner, reached only from the
-- admin route POST /api/core/v1/admin/operator-agent-creations and the
-- root-only CLI `agent-create-for-owner-exact`. Readers: GET on the same
-- admin route and operators reading the table.
--
-- Additive and safe to reapply at every Core startup. Older Core never
-- reads or writes it, so rolling Core back leaves operator-created Agents
-- working as ordinary Agents and leaves this record in place. No existing
-- row is rewritten.
CREATE TABLE IF NOT EXISTS operator_agent_creations (
  id TEXT PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE
    CHECK (btrim(idempotency_key) <> '' AND length(idempotency_key) <= 128),
  owner_email TEXT NOT NULL CHECK (owner_email = lower(btrim(owner_email))),
  owner_user_id TEXT NOT NULL REFERENCES users(id),
  -- True when this creation minted the pending account for owner_email.
  owner_account_created BOOLEAN NOT NULL,
  customer_org_id TEXT NOT NULL REFERENCES customer_orgs(id),
  -- True when this creation minted the owner's personal org (as sponsored).
  customer_org_created BOOLEAN NOT NULL,
  -- Deferred: the record claims its idempotency key before Core writes the
  -- Project and request, in the same transaction.
  project_id TEXT NOT NULL UNIQUE REFERENCES projects(id) DEFERRABLE INITIALLY DEFERRED,
  agent_creation_request_id TEXT NOT NULL UNIQUE
    REFERENCES agent_creation_requests(id) DEFERRABLE INITIALLY DEFERRED,
  display_name TEXT NOT NULL,
  hosting_tier TEXT NOT NULL CHECK (hosting_tier IN ('standard', 'confidential')),
  placement_runner_class TEXT NOT NULL,
  runtime_resource_class TEXT NOT NULL,
  target_source_host_id TEXT,
  owner_chat_account_id TEXT CHECK (owner_chat_account_id IS NULL OR owner_chat_account_id ~ '^[0-9a-f]{64}$'),
  -- How the creation was admitted in place of a Launch Code or billing.
  admission TEXT NOT NULL CHECK (admission IN ('operator_sponsored')),
  reason TEXT NOT NULL CHECK (btrim(reason) <> '' AND length(reason) <= 2000),
  admin_user_id TEXT NOT NULL REFERENCES users(id),
  admin_email TEXT NOT NULL,
  admin_workos_user_id TEXT NOT NULL,
  surface TEXT NOT NULL CHECK (surface IN ('admin_api', 'cli')),
  created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS operator_agent_creations_owner
  ON operator_agent_creations(owner_user_id, created_at);
