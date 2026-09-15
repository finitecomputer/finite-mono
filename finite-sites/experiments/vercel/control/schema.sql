-- Disposable experiment only. Never apply to a production Sites database.
CREATE TABLE IF NOT EXISTS poc_sites (
  id text PRIMARY KEY CHECK (id ~ '^[a-z][a-z0-9-]{2,62}$'),
  gate_hash text NOT NULL CHECK (length(gate_hash) = 64),
  visibility text NOT NULL DEFAULT 'private' CHECK (visibility IN ('private','shared','public')),
  disabled boolean NOT NULL DEFAULT false
);
CREATE TABLE IF NOT EXISTS poc_grants (
  site_id text NOT NULL REFERENCES poc_sites(id), email text NOT NULL,
  PRIMARY KEY (site_id, email)
);
CREATE TABLE IF NOT EXISTS poc_handoffs (
  token_hash text PRIMARY KEY, site_id text NOT NULL REFERENCES poc_sites(id),
  email text NOT NULL, expires_at timestamptz NOT NULL, consumed_at timestamptz
);
CREATE TABLE IF NOT EXISTS poc_sessions (
  token_hash text PRIMARY KEY, site_id text NOT NULL REFERENCES poc_sites(id),
  email text NOT NULL, expires_at timestamptz NOT NULL
);
