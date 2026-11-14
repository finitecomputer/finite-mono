-- Operator-created Agents: an admin creates an ordinary Agent owned by an
-- account named by email, including a pending account that has never signed
-- in. Each row is the durable audit record and the idempotency key of one
-- such creation. The Project, creation request and membership it points at
-- are ordinary rows.
--
-- Writer: CoreStore::admin_create_agent_for_owner, reached only from the
-- admin route POST /api/core/v1/admin/operator-agent-creations and the
-- root-only CLI `agent-create-for-owner-exact`. Readers: GET on the same
-- admin route; the self-serve allowance count, which leaves these Agents
-- out; event-trial reservation, redemption and suspension, which never
-- apply to an org holding one; and Finite Private runtime-key provisioning,
-- which never changes an existing grant for one.
--
-- Additive and safe to reapply at every Core startup. No existing row is
-- rewritten. Older Core never reads or writes it: after a rollback these
-- Agents keep working as ordinary Agents, count against the org's
-- self-serve allowance again (fewer self-serve slots, never more), and the
-- trial bar and grant protection are not enforced until Core is upgraded.
CREATE TABLE IF NOT EXISTS operator_agent_creations (
  id TEXT PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE
    CHECK (btrim(idempotency_key) <> '' AND length(idempotency_key) <= 128),
  owner_email TEXT NOT NULL CHECK (owner_email = lower(btrim(owner_email))),
  owner_user_id TEXT NOT NULL REFERENCES users(id),
  -- True only when this creation itself wrote the new pending account row.
  owner_account_created BOOLEAN NOT NULL,
  customer_org_id TEXT NOT NULL REFERENCES customer_orgs(id),
  -- True only when this creation itself wrote the new org row (sponsored).
  customer_org_created BOOLEAN NOT NULL,
  -- The WorkOS user the owner signs in as, found or created by the operator
  -- path, and that user's hosted-chat account. The Agent admits only it.
  owner_workos_user_id TEXT NOT NULL CHECK (btrim(owner_workos_user_id) <> ''),
  owner_chat_account_id TEXT NOT NULL CHECK (owner_chat_account_id ~ '^[0-9a-f]{64}$'),
  -- The operator confirmed adding an Agent to an already signed-in account.
  existing_account_confirmed BOOLEAN NOT NULL,
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
  -- How the creation was admitted in place of a Launch Code or billing.
  admission TEXT NOT NULL CHECK (admission IN ('operator_sponsored')),
  reason TEXT NOT NULL CHECK (btrim(reason) <> '' AND length(reason) <= 2000),
  admin_user_id TEXT NOT NULL REFERENCES users(id),
  admin_email TEXT NOT NULL,
  admin_workos_user_id TEXT NOT NULL,
  surface TEXT NOT NULL CHECK (surface IN ('admin_api', 'cli')),
  -- The OS account that ran the CLI; set exactly for CLI creations.
  cli_os_user TEXT CHECK ((surface = 'cli') = (cli_os_user IS NOT NULL)),
  created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS operator_agent_creations_owner
  ON operator_agent_creations(owner_user_id, created_at);
CREATE INDEX IF NOT EXISTS operator_agent_creations_org
  ON operator_agent_creations(customer_org_id);
