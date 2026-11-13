-- Descriptive operator attributions only. No foreign key or trigger can
-- cascade into access, account associations, identities or Chat state.
CREATE TABLE IF NOT EXISTS brain_scoped_record_operations (
    id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL UNIQUE,
    manifest_sha256 TEXT NOT NULL CHECK (manifest_sha256 ~ '^[0-9a-f]{64}$'),
    action TEXT NOT NULL CHECK (action IN ('apply', 'retire')),
    brain_server TEXT NOT NULL,
    brain_id TEXT NOT NULL,
    operator_account_id TEXT NOT NULL REFERENCES users(id),
    operator_workos_subject TEXT NOT NULL,
    operator_email TEXT NOT NULL,
    target_operation_id TEXT REFERENCES brain_scoped_record_operations(operation_id),
    recorded_at TIMESTAMPTZ NOT NULL,
    receipt JSONB,
    CHECK ((action = 'apply' AND target_operation_id IS NULL)
        OR (action = 'retire' AND target_operation_id IS NOT NULL
            AND target_operation_id <> operation_id)),
    CHECK (receipt IS NULL OR jsonb_typeof(receipt) = 'object')
);

CREATE TABLE IF NOT EXISTS brain_scoped_records (
    id TEXT PRIMARY KEY,
    revision TEXT NOT NULL,
    brain_server TEXT NOT NULL,
    brain_id TEXT NOT NULL,
    public_key_hex TEXT NOT NULL CHECK (public_key_hex ~ '^[0-9a-f]{64}$'),
    kind TEXT NOT NULL CHECK (kind IN (
        'managed_agent', 'account_chat_key', 'historical_identity', 'invitation_key')),
    account_id TEXT REFERENCES users(id),
    workos_subject TEXT,
    project_id TEXT REFERENCES projects(id),
    annotation_label TEXT,
    evidence_sha256 TEXT NOT NULL CHECK (evidence_sha256 ~ '^[0-9a-f]{64}$'),
    evidence_reference TEXT NOT NULL CHECK (octet_length(evidence_reference) BETWEEN 1 AND 1024),
    -- Preserve the exact evidence timestamp in the manifest digest and receipt.
    evidence_observed_at TEXT NOT NULL CHECK (
        octet_length(evidence_observed_at) <= 40 AND evidence_observed_at ~ 'Z$'
        AND evidence_observed_at::timestamptz IS NOT NULL),
    creating_operation_id TEXT NOT NULL REFERENCES brain_scoped_record_operations(operation_id),
    created_at TIMESTAMPTZ NOT NULL,
    retired_at TIMESTAMPTZ,
    CHECK (
        (kind = 'managed_agent' AND account_id IS NOT NULL AND workos_subject IS NOT NULL
            AND project_id IS NOT NULL AND annotation_label IS NULL)
        OR (kind = 'account_chat_key' AND account_id IS NOT NULL AND workos_subject IS NOT NULL
            AND project_id IS NULL AND annotation_label IS NULL)
        OR (kind IN ('historical_identity', 'invitation_key') AND account_id IS NULL
            AND workos_subject IS NULL AND project_id IS NULL AND annotation_label IS NOT NULL
            AND octet_length(annotation_label) BETWEEN 1 AND 120
            AND annotation_label = btrim(annotation_label)
            AND annotation_label COLLATE "C" ~ '^[ -~]+$')
    )
);
CREATE UNIQUE INDEX IF NOT EXISTS brain_scoped_records_active_key
    ON brain_scoped_records(brain_server, brain_id, public_key_hex) WHERE retired_at IS NULL;
CREATE INDEX IF NOT EXISTS brain_scoped_records_creating_operation
    ON brain_scoped_records(creating_operation_id, public_key_hex);
