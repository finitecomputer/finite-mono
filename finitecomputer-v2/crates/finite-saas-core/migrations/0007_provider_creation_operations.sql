-- Durable, provider-neutral creation-operation ledger. These are new tables,
-- so N-1 Core binaries continue to read and write agent_creation_requests
-- without seeing new columns or changing their row shape.

CREATE TABLE IF NOT EXISTS agent_creation_provider_operations (
  agent_creation_request_id TEXT PRIMARY KEY
    REFERENCES agent_creation_requests(id) ON DELETE CASCADE,
  schema_name TEXT NOT NULL
    CHECK (schema_name = 'provider_operation.v1'),
  correlation_id TEXT NOT NULL,
  placement_runner_class TEXT NOT NULL,
  runtime_resource_class TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL,
  CHECK (char_length(correlation_id) BETWEEN 1 AND 128),
  CHECK (placement_runner_class IN ('local_docker', 'apple_container', 'kata', 'phala', 'enclavia')),
  CHECK (runtime_resource_class IN ('vcpu4_memory8_gib', 'vcpu2_memory4_gib'))
);

CREATE TABLE IF NOT EXISTS agent_creation_provider_operation_transitions (
  agent_creation_request_id TEXT NOT NULL
    REFERENCES agent_creation_provider_operations(agent_creation_request_id)
    ON DELETE CASCADE,
  sequence INTEGER NOT NULL CHECK (sequence >= 0),
  transition JSONB NOT NULL CHECK (jsonb_typeof(transition) = 'object'),
  recorded_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (agent_creation_request_id, sequence)
);

COMMENT ON TABLE agent_creation_provider_operations IS
  'One immutable provider-operation identity per Core agent creation request.';
COMMENT ON TABLE agent_creation_provider_operation_transitions IS
  'Append-only, lease-fenced provider creation acknowledgments; provision_started must be durable before the first provider mutation.';
