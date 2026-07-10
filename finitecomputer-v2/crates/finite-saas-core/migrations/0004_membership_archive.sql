ALTER TABLE project_room_memberships
    ADD COLUMN IF NOT EXISTS archived_at TIMESTAMPTZ;

