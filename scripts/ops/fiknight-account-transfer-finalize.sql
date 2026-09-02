\set ON_ERROR_STOP on

-- Run only after the FiKnight account has opened the existing Project and its
-- historical Chat successfully, and after the new NIP-05 resolves correctly.

BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
SELECT pg_advisory_xact_lock(hashtextextended('fiknight-account-transfer-2026-09-02', 0));

DO $finalize$
DECLARE
  archived timestamp with time zone;
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM projects
    WHERE id = 'project_b7e3a5beaf06095c6465'
      AND customer_org_id = 'org_696a800e548d65b8be93'
      AND owner_user_id = 'user_b9540ab702bd98195b98'
      AND display_name = 'FiKnight'
      AND agent_email = 'fiknight@finite.vip'
  ) THEN
    RAISE EXCEPTION 'FiKnight staged Project state is absent';
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM project_room_memberships
    WHERE id = 'room_member_def47a34609964844c10'
      AND project_id = 'project_b7e3a5beaf06095c6465'
      AND chat_identity_id = 'chat_identity_87d8015f0f068fd16b6d'
      AND role = 'owner'
      AND archived_at IS NULL
  ) THEN
    RAISE EXCEPTION 'FiKnight active owner membership is absent';
  END IF;

  SELECT archived_at INTO archived
  FROM project_room_memberships
  WHERE id = 'room_member_d9b3b3e8b96c05b39011'
    AND project_id = 'project_b7e3a5beaf06095c6465'
    AND chat_identity_id = 'chat_identity_9a8590f157554b14c4b7'
    AND role = 'owner'
  FOR UPDATE;

  IF NOT FOUND THEN
    RAISE EXCEPTION 'Austin source membership changed or disappeared';
  END IF;

  IF archived IS NULL THEN
    UPDATE project_room_memberships
    SET archived_at = clock_timestamp()
    WHERE id = 'room_member_d9b3b3e8b96c05b39011';
  END IF;
END
$finalize$;

COMMIT;

SELECT membership.id, owner.normalized_email, membership.role, membership.archived_at
FROM project_room_memberships membership
JOIN chat_identities identity ON identity.id = membership.chat_identity_id
JOIN users owner ON owner.id = identity.user_id
WHERE membership.project_id = 'project_b7e3a5beaf06095c6465'
ORDER BY owner.normalized_email;
