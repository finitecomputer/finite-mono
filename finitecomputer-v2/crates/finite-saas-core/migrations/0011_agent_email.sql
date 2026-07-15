-- A hosted Agent Principal has one stable, user-facing Finite Identity name.
-- Imported projects stay nullable until their identity is explicitly adopted.
ALTER TABLE projects
  ADD COLUMN IF NOT EXISTS agent_email TEXT;

-- This product is unreleased, but durable local and pre-release self-service
-- rows already exist. Name those deterministically so the expand migration
-- does not strand them on the old npub-first UX. Imported Projects remain
-- unset until their identity is explicitly adopted.
UPDATE projects
SET agent_email = concat(
  COALESCE(
    NULLIF(
      trim(both '-' FROM left(
        regexp_replace(lower(display_name), '[^a-z0-9]+', '-', 'g'),
        40
      )),
      ''
    ),
    'agent'
  ),
  '-',
  CASE
    WHEN length(regexp_replace(lower(regexp_replace(id, '^project_', '')), '[^a-z0-9]', '', 'g')) >= 16
      THEN left(regexp_replace(lower(regexp_replace(id, '^project_', '')), '[^a-z0-9]', '', 'g'), 16)
    ELSE left(md5(id), 16)
  END,
  '@finite.vip'
)
WHERE agent_email IS NULL
  AND import_candidate_id IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS projects_agent_email_unique
  ON projects(agent_email)
  WHERE agent_email IS NOT NULL;
