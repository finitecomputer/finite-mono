-- New namespace: old multi-project proof remains historical, never migrated.
CREATE TABLE IF NOT EXISTS poc2_sites (
  id text PRIMARY KEY CHECK (id ~ '^[a-z][a-z0-9-]{1,61}[a-z0-9]$'),
  hostname text NOT NULL UNIQUE,
  disabled boolean NOT NULL DEFAULT false,
  active_version text
);
CREATE TABLE IF NOT EXISTS poc2_versions (
  site_id text NOT NULL REFERENCES poc2_sites(id),
  id text NOT NULL CHECK (length(id)=64),
  source_commit text NOT NULL CHECK (length(source_commit)=40),
  deploy_path text NOT NULL,
  ready boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (site_id,id)
);
DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname='poc2_active_version_fk' AND connamespace=current_schema()::regnamespace) THEN
    ALTER TABLE poc2_sites ADD CONSTRAINT poc2_active_version_fk
      FOREIGN KEY (id,active_version) REFERENCES poc2_versions(site_id,id);
  END IF;
END $$;
CREATE TABLE IF NOT EXISTS poc2_files (
  site_id text NOT NULL,
  version_id text NOT NULL,
  path text NOT NULL,
  sha256 text NOT NULL CHECK (length(sha256)=64),
  size integer NOT NULL CHECK (size BETWEEN 0 AND 1048576),
  uploaded boolean NOT NULL DEFAULT false,
  PRIMARY KEY (site_id,version_id,path),
  FOREIGN KEY (site_id,version_id) REFERENCES poc2_versions(site_id,id)
);
CREATE TABLE IF NOT EXISTS poc2_grants (
  site_id text NOT NULL REFERENCES poc2_sites(id), email text NOT NULL,
  PRIMARY KEY (site_id,email)
);
CREATE TABLE IF NOT EXISTS poc2_handoffs (
  token_hash text PRIMARY KEY, site_id text NOT NULL REFERENCES poc2_sites(id),
  hostname text NOT NULL, email text NOT NULL,
  expires_at timestamptz NOT NULL, consumed_at timestamptz
);
CREATE TABLE IF NOT EXISTS poc2_sessions (
  token_hash text PRIMARY KEY, site_id text NOT NULL REFERENCES poc2_sites(id),
  hostname text NOT NULL, email text NOT NULL, expires_at timestamptz NOT NULL
);
