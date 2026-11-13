-- The finite.chat mover's role on Core's database (FIN-463 M1): it reads
-- these columns and nothing else, and writes nothing. Its marks go through
-- Core's own CLI (runtime-*-exact), never this role. The mover's tests run
-- every query as a role granted exactly this file.
--
-- Ops applies it once, as Core's database owner, after creating the role
-- (peer authentication from the mover's own system user):
--   CREATE ROLE finite_mover LOGIN;
-- Drop the role when FIN-463's removal plan runs.
GRANT SELECT (id, normalized_email, workos_user_id) ON users TO finite_mover;
GRANT SELECT (id, owner_user_id, display_name, import_candidate_id) ON projects TO finite_mover;
GRANT SELECT (project_id, agent_runtime_id, active) ON project_runtime_links TO finite_mover;
GRANT SELECT (id, project_id, source_host_id, source_machine_id, placement_runner_class, created_at) ON agent_runtimes TO finite_mover;
-- The runtime spec names the durable tree and the runtime it belongs to.
-- It also holds the runtime's non-secret environment; secrets are only
-- named there, never held.
GRANT SELECT (id, project_id, agent_runtime_id, status, relocation_spec, created_at, profile_picture_url, runtime_spec)
  ON agent_creation_requests TO finite_mover;
GRANT SELECT (agent_runtime_id) ON agent_runtime_moves TO finite_mover;
GRANT SELECT (agent_runtime_id, request_id) ON runtime_retirement_snapshots TO finite_mover;
