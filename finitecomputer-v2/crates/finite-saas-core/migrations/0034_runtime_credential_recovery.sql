-- Explicit credential replacement authority belongs to one exact upgrade.
-- Append-only from the operator transaction; ordinary upgrades have no row.
CREATE TABLE IF NOT EXISTS runtime_credential_recoveries (
    runtime_control_request_id TEXT PRIMARY KEY REFERENCES runtime_control_requests(id),
    predecessor_creation_request_id TEXT NOT NULL REFERENCES runtime_core_credentials(creation_request_id),
    successor_creation_request_id TEXT NOT NULL UNIQUE REFERENCES runtime_core_credentials(creation_request_id),
    CHECK (predecessor_creation_request_id <> successor_creation_request_id)
);
