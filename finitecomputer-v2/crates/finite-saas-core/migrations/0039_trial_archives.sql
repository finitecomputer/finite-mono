-- Archive and reclaim are distinct leased operations. A committed receipt is
-- required before local reclamation; no remote archive is deleted by this flow.
ALTER TABLE runtime_control_requests DROP CONSTRAINT IF EXISTS runtime_control_requests_kind_check;
ALTER TABLE runtime_control_requests ADD CONSTRAINT runtime_control_requests_kind_check
    CHECK (kind IN ('restart','recover_known_good_chat_runtime','upgrade','stop','destroy','archive_trial','reclaim_trial'));

CREATE TABLE IF NOT EXISTS trial_runtime_archives (
    archive_request_id TEXT PRIMARY KEY REFERENCES runtime_control_requests(id),
    agent_runtime_id TEXT NOT NULL REFERENCES agent_runtimes(id),
    archive_lease_sha256 TEXT NOT NULL,
    reclaim_lease_sha256 TEXT,
    snapshot JSONB NOT NULL CHECK (jsonb_typeof(snapshot) = 'object'),
    reclaim_request_id TEXT UNIQUE REFERENCES runtime_control_requests(id),
    restore_request_id TEXT UNIQUE REFERENCES agent_creation_requests(id),
    reclaimed_at TIMESTAMPTZ,
    restored_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (reclaimed_at IS NULL OR reclaim_request_id IS NOT NULL),
    CHECK (restore_request_id IS NULL OR reclaimed_at IS NOT NULL)
);
CREATE UNIQUE INDEX IF NOT EXISTS trial_runtime_archives_one_residency
    ON trial_runtime_archives(agent_runtime_id) WHERE restored_at IS NULL;
