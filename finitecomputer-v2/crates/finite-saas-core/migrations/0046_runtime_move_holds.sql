-- FIN-463: a Runtime the finite.chat mover stopped for a move. While it has
-- a row here, or is marked in agent_runtime_moves, nothing starts it again:
-- no restart, recovery, upgrade or relocation from the dashboard, staff,
-- billing or a trial restore. The one exception is the mover's own
-- runtime-restart-exact (staff gave the agent back), which removes the row
-- in the same transaction; a Runtime marked moved is never started.
--
-- Writers: runtime-stop-exact (inserts) and runtime-restart-exact (deletes),
-- which only the mover runs. Additive and safe to reapply at every Core
-- startup.
--
-- Delete when every old agent is moved or declined (FIN-463).
CREATE TABLE IF NOT EXISTS agent_runtime_move_holds (
  agent_runtime_id TEXT PRIMARY KEY REFERENCES agent_runtimes(id),
  stop_request_id TEXT NOT NULL REFERENCES runtime_control_requests(id),
  held_at TIMESTAMPTZ NOT NULL
);
