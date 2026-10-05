-- One inference key per pinned restore, without persisting recoverable key material.
ALTER TABLE trial_runtime_archives
    ADD COLUMN IF NOT EXISTS restore_private_key_id TEXT REFERENCES finite_private_api_keys(id);
CREATE UNIQUE INDEX IF NOT EXISTS trial_archive_restore_private_key
    ON trial_runtime_archives(restore_private_key_id) WHERE restore_private_key_id IS NOT NULL;
