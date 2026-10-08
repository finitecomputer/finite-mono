-- Additive account support outbox. No chat, identity, or runtime state is changed.
CREATE TABLE IF NOT EXISTS support_requests (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 128),
  project_id TEXT REFERENCES projects(id),
  recipient TEXT NOT NULL,
  reply_to TEXT NOT NULL,
  message TEXT NOT NULL CHECK (length(message) BETWEEN 1 AND 4000),
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sent', 'failed')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  lease_token TEXT,
  lease_until TIMESTAMPTZ,
  attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  UNIQUE (user_id, idempotency_key)
);
CREATE INDEX IF NOT EXISTS support_requests_pending ON support_requests (next_attempt_at)
  WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS support_requests_user_time ON support_requests (user_id, created_at);
