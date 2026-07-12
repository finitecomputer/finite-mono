-- Recovery support is immutable release material, not a property of the
-- currently running worker binary. Existing/N-1 artifacts stay fail-closed;
-- only a newly registered receiver image may opt in explicitly.

ALTER TABLE runtime_artifacts
  ADD COLUMN IF NOT EXISTS recover_known_good_chat BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN runtime_artifacts.recover_known_good_chat IS
  'Whether this exact immutable runtime artifact implements recover-known-good-chat.';
