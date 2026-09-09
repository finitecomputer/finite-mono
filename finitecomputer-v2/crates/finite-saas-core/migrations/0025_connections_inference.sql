-- Inactive foundation for FIN-37. No backfill: no row means unmanaged.
-- These records belong to the Project, not to disposable compute.
CREATE TABLE IF NOT EXISTS connection_inference_settings (
    project_id TEXT PRIMARY KEY REFERENCES projects(id),
    revision BIGINT NOT NULL CHECK (revision > 0),
    profile TEXT NOT NULL CHECK (profile IN ('finite_private', 'openrouter')),
    model TEXT,
    credential_version BIGINT NOT NULL CHECK (credential_version >= 0),
    credential_key_id TEXT,
    credential_nonce BYTEA,
    credential_ciphertext BYTEA,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((credential_version = 0 AND credential_key_id IS NULL
            AND credential_nonce IS NULL AND credential_ciphertext IS NULL)
        OR (credential_version > 0 AND credential_key_id IS NOT NULL
            AND credential_nonce IS NOT NULL AND octet_length(credential_nonce) = 24
            AND credential_ciphertext IS NOT NULL AND octet_length(credential_ciphertext) > 16))
);

-- Redacted receipts survive later changes so a lost response cannot create a
-- second revision. Fingerprints are keyed: never store a plain hash of secrets.
CREATE TABLE IF NOT EXISTS connection_inference_changes (
    project_id TEXT NOT NULL REFERENCES projects(id),
    change_id TEXT NOT NULL,
    actor_user_id TEXT NOT NULL REFERENCES users(id),
    fingerprint_key_id TEXT NOT NULL,
    fingerprint BYTEA NOT NULL CHECK (octet_length(fingerprint) = 32),
    revision BIGINT NOT NULL CHECK (revision > 0),
    profile TEXT NOT NULL CHECK (profile IN ('finite_private', 'openrouter')),
    model TEXT,
    credential_version BIGINT NOT NULL CHECK (credential_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, change_id),
    UNIQUE (project_id, revision)
);
