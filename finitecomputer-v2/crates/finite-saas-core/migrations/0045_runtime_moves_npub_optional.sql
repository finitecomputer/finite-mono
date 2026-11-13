-- FIN-463: a migrated mark may lack its finite.chat agent's npub. The
-- mover marks a Runtime when finite.chat's Migrations tells it to retire
-- one, and that command (fragment's wire) carries no npub; staff may add
-- one by hand. An archived mark still has none. Safe to reapply at every
-- Core startup.
ALTER TABLE agent_runtime_moves
  DROP CONSTRAINT IF EXISTS agent_runtime_moves_npub_when_migrated;
DO $$
BEGIN
  ALTER TABLE agent_runtime_moves
    ADD CONSTRAINT agent_runtime_moves_no_npub_when_archived
    CHECK (moved = 'migrated' OR agent_npub IS NULL);
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;
