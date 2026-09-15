ALTER TABLE poc2_versions ADD COLUMN IF NOT EXISTS version_no serial;
ALTER TABLE poc2_versions ADD COLUMN IF NOT EXISTS source_hash text;
CREATE TABLE IF NOT EXISTS poc2_publishers (pubkey text PRIMARY KEY CHECK(length(pubkey)=64), allowed boolean NOT NULL DEFAULT true);
CREATE TABLE IF NOT EXISTS poc2_projects (
 slug text PRIMARY KEY,site_id text NOT NULL UNIQUE REFERENCES poc2_sites(id),
 owner_pubkey text NOT NULL REFERENCES poc2_publishers(pubkey),
 git_remote_url text NOT NULL,branch text NOT NULL,deploy_path text NOT NULL,
 publisher_hash text NOT NULL CHECK(length(publisher_hash)=64)
);
CREATE TABLE IF NOT EXISTS poc2_native_grants (site_id text NOT NULL REFERENCES poc2_sites(id),pubkey text NOT NULL CHECK(length(pubkey)=64),PRIMARY KEY(site_id,pubkey));
CREATE TABLE IF NOT EXISTS poc2_native_handoffs (
 token_hash text PRIMARY KEY,site_id text NOT NULL REFERENCES poc2_sites(id),pubkey text NOT NULL,
 hostname text NOT NULL,return_to text NOT NULL,nonce text NOT NULL,expires_at timestamptz NOT NULL,consumed_at timestamptz,
 UNIQUE(site_id,pubkey,nonce)
);
CREATE TABLE IF NOT EXISTS poc2_native_sessions (
 token_hash text PRIMARY KEY,site_id text NOT NULL REFERENCES poc2_sites(id),pubkey text NOT NULL,hostname text NOT NULL,expires_at timestamptz NOT NULL
);
