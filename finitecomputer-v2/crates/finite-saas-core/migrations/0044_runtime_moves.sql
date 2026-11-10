-- FIN-463: which finite.computer Agent Runtimes have moved to finite.chat.
-- Additive and safe to reapply at every Core startup; no existing row is
-- rewritten.
--
-- Writer: the operator CLI `runtime-mark-moved-exact` only, which the
-- finite.chat mover runs for one exact Runtime when finite.chat's Migrations
-- tells it the move is done (`migrated`) or the Runtime was archived at the
-- cutoff without an import (`archived`). Reader: the mover's roster, which
-- leaves out every Runtime listed here. `archived` may later become
-- `migrated` (a late import); nothing else changes a row. Drop the table
-- when FIN-463's removal plan runs.
CREATE TABLE IF NOT EXISTS agent_runtime_moves (
  agent_runtime_id TEXT PRIMARY KEY REFERENCES agent_runtimes(id),
  project_id TEXT NOT NULL REFERENCES projects(id),
  moved TEXT NOT NULL CHECK (moved IN ('migrated', 'archived')),
  moved_at TIMESTAMPTZ NOT NULL,
  -- The finite.chat agent the Runtime became; only a migrated Runtime has one.
  agent_npub TEXT CHECK (agent_npub IS NULL OR agent_npub ~ '^npub1[02-9ac-hj-np-z]{58}$'),
  CONSTRAINT agent_runtime_moves_npub_when_migrated
    CHECK ((moved = 'migrated') = (agent_npub IS NOT NULL))
);
