-- ADR-0046 SaaS Core enrichment: Permanent Departure Facts and the per-account
-- agent roster revision. Brain consumes departure facts with a
-- last-applied-revision cursor; routine Brain authorization never calls Core.
--
-- `account_id` is the owner's WorkOS user id — the external account handle
-- Brain already resolves through `/api/core/v1/brain/agent-account`.
-- `principal_ref` is the principal's stable external reference: the Managed
-- Agent Email for agents, the verified human mailbox for humans. Rows are
-- append-only facts; `revision` is the global monotonic replay cursor.
CREATE TABLE IF NOT EXISTS brain_agent_departure_facts (
  revision BIGSERIAL PRIMARY KEY,
  account_id TEXT NOT NULL,
  principal_kind TEXT NOT NULL CHECK (principal_kind IN ('human', 'agent')),
  principal_ref TEXT NOT NULL,
  departed_at TIMESTAMPTZ NOT NULL,
  reason TEXT NOT NULL CHECK (reason IN ('retired', 'deleted', 'unlinked'))
);

CREATE INDEX IF NOT EXISTS brain_agent_departure_facts_account_idx
  ON brain_agent_departure_facts(account_id, revision);

-- Per-account monotonic roster revision, bumped in the same transaction as
-- every roster membership change (agent creation success, retirement,
-- deletion, unlink). Missing row means revision 0 (no recorded changes).
CREATE TABLE IF NOT EXISTS brain_account_roster_revisions (
  account_id TEXT PRIMARY KEY,
  roster_revision BIGINT NOT NULL CHECK (roster_revision > 0),
  updated_at TIMESTAMPTZ NOT NULL
);
