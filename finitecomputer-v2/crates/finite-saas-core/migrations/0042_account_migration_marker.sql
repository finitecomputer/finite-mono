-- The signed-in user's "use the new Finite by default" preference, set and
-- cleared by that user through PUT /api/core/v1/me/front-door-preference.
-- Additive and nullable: NULL means off, and an N-1 Core that never reads the
-- column keeps working unchanged. It is a routing preference for the legacy
-- dashboard's front door, holds no user data, and never gates Agent Runtime
-- lifecycle, billing, or recovery. `/me` exposes it read-only.
ALTER TABLE users ADD COLUMN IF NOT EXISTS migrated_at TIMESTAMPTZ;
