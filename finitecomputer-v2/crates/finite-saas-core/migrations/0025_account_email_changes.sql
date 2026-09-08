-- Account Auth changes preserve every Core and cryptographic identity.
CREATE TABLE IF NOT EXISTS account_email_changes (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  workos_user_id TEXT NOT NULL,
  old_email TEXT NOT NULL,
  new_email TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('prepared', 'completed', 'cancelled')),
  prepared_by TEXT NOT NULL,
  completed_by TEXT,
  evidence_reference TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL,
  CHECK (old_email <> new_email)
);
CREATE UNIQUE INDEX IF NOT EXISTS account_email_changes_one_pending_user
  ON account_email_changes(user_id) WHERE status = 'prepared';
CREATE UNIQUE INDEX IF NOT EXISTS account_email_changes_one_pending_destination
  ON account_email_changes(new_email) WHERE status = 'prepared';
