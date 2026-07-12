-- Current Core allocates the durable Runtime identity while leasing, before
-- the runner creates and registers the provider runtime. The original FK was
-- written for the older flow where agent_runtime_id appeared only after an
-- agent_runtimes row existed, and now rejects that intentional provisional
-- identity. RuntimeSpec binding plus register/complete validation enforce the
-- identity until the runtime row is committed.

ALTER TABLE agent_creation_requests
  DROP CONSTRAINT IF EXISTS agent_creation_requests_agent_runtime_id_fkey;

COMMENT ON COLUMN agent_creation_requests.agent_runtime_id IS
  'Core-allocated runtime identity; may be provisional while status is launching.';
