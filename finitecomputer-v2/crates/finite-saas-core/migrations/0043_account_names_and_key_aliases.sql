-- Account display names from WorkOS and private per-account key aliases.
-- Purely additive and safe to reapply at every Core startup. No existing row
-- is rewritten or backfilled: a name appears after the account's next
-- verified sign-in, and aliases start empty.
--
-- users.workos_display_name writer: the verified-account link on
-- GET /api/core/v1/me and /me/dashboard-summary, from the WorkOS user that
-- request already fetched. NULL means WorkOS gave no usable name. Reader: the
-- Brain identity description route, for human rows it already releases.
ALTER TABLE users
  ADD COLUMN IF NOT EXISTS workos_display_name TEXT,
  ADD COLUMN IF NOT EXISTS workos_display_name_updated_at TIMESTAMPTZ;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'users_workos_display_name_bounded'
  ) THEN
    ALTER TABLE users ADD CONSTRAINT users_workos_display_name_bounded
      CHECK (workos_display_name IS NULL
             OR (octet_length(workos_display_name) BETWEEN 1 AND 200
                 AND workos_display_name = btrim(workos_display_name)));
  END IF;
END $$;

-- A private label one account gives one exact key. Visible only to that
-- account; never identity evidence and never read by access decisions.
-- Writer and reader: the account-authenticated /api/core/v1/me/key-aliases
-- routes only.
CREATE TABLE IF NOT EXISTS account_key_aliases (
  viewer_user_id TEXT NOT NULL REFERENCES users(id),
  public_key_hex TEXT NOT NULL CHECK (public_key_hex ~ '^[0-9a-f]{64}$'),
  alias TEXT NOT NULL CHECK (
    char_length(alias) BETWEEN 1 AND 64
    AND octet_length(alias) <= 256
    AND alias = btrim(alias)
  ),
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (viewer_user_id, public_key_hex)
);
