-- Replayable, service-authenticated lifecycle facts consumed by Finite Brain.
-- A row is emitted only after an irreversible Agent lifecycle operation has
-- completed. Runtime stop/restart/relocation/health state never enters here.
CREATE TABLE IF NOT EXISTS brain_agent_departure_facts (
  fact_id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  human_mailbox TEXT NOT NULL,
  managed_agent_nip05 TEXT NOT NULL,
  principal_binding_reference TEXT NOT NULL,
  departure_kind TEXT NOT NULL CHECK (departure_kind IN ('unlinked', 'retired', 'deleted')),
  occurred_at TIMESTAMPTZ NOT NULL,
  source_operation_id TEXT NOT NULL UNIQUE
);

CREATE INDEX IF NOT EXISTS brain_agent_departure_facts_account_idx
  ON brain_agent_departure_facts(account_id, occurred_at, fact_id);
