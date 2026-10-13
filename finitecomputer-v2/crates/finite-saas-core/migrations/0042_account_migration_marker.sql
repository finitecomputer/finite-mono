-- When the new Finite product's import flow records that an account moved
-- there. Additive and nullable: NULL means not migrated, and an N-1 Core that
-- never reads the column keeps working unchanged. The first mark wins and is
-- never rewritten by the service route; there is no unset API. The column
-- holds no user data and never gates Agent Runtime lifecycle, billing, or
-- recovery. The legacy dashboard's front door reads it through `/me` only.
ALTER TABLE users ADD COLUMN IF NOT EXISTS migrated_at TIMESTAMPTZ;
