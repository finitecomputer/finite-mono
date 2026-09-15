-- A binding is immutable and must precede redemption. Its host stays reserved
-- for explicitly targeted creation until a separate capacity-release change.
CREATE TABLE IF NOT EXISTS launch_code_host_targets (
    launch_code_id TEXT PRIMARY KEY REFERENCES launch_codes(id),
    source_host_id TEXT NOT NULL CHECK (length(source_host_id) > 0),
    created_by_workos_user_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS launch_code_host_targets_host_idx
    ON launch_code_host_targets(source_host_id);
