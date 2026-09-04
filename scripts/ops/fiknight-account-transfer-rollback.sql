\set ON_ERROR_STOP on

-- Pre-commit rollback for the staged Core transfer. Do not run after binding
-- fiknight@finite.vip in Finite Identity without first reviewing that durable,
-- intentionally non-reassignable public identity state.

BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
SELECT pg_advisory_xact_lock(hashtextextended('fiknight-account-transfer-2026-09-02', 0));

DO $rollback$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM projects
    WHERE id = 'project_b7e3a5beaf06095c6465'
      AND customer_org_id = 'org_696a800e548d65b8be93'
      AND owner_user_id = 'user_b9540ab702bd98195b98'
      AND display_name = 'FiKnight'
      AND agent_email = 'fiknight@finite.vip'
  ) THEN
    RAISE EXCEPTION 'Exact staged Project state is absent';
  END IF;

  IF EXISTS (
    SELECT 1 FROM runtime_control_requests
    WHERE agent_runtime_id = 'runtime_d8ceb9b4f4e9bacb85b0'
      AND status IN ('requested', 'launching', 'compute_up', 'ready')
  ) THEN
    RAISE EXCEPTION 'Runtime has an in-flight control operation';
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM project_room_memberships
    WHERE id = 'room_member_d9b3b3e8b96c05b39011'
      AND project_id = 'project_b7e3a5beaf06095c6465'
      AND chat_identity_id = 'chat_identity_9a8590f157554b14c4b7'
      AND role = 'owner'
  ) THEN
    RAISE EXCEPTION 'Austin source membership changed or disappeared';
  END IF;

  UPDATE project_room_memberships
  SET archived_at = NULL
  WHERE id = 'room_member_d9b3b3e8b96c05b39011';

  UPDATE project_room_memberships
  SET archived_at = COALESCE(archived_at, clock_timestamp())
  WHERE id = 'room_member_def47a34609964844c10'
    AND project_id = 'project_b7e3a5beaf06095c6465'
    AND chat_identity_id = 'chat_identity_87d8015f0f068fd16b6d';

  UPDATE finite_private_api_keys
  SET grant_id = 'fp_grant_d0b9190a14c836b00ffc', updated_at = clock_timestamp()
  WHERE id = 'fp_key_deb646d119995d3bd52a'
    AND project_id = 'project_b7e3a5beaf06095c6465'
    AND grant_id = 'fp_grant_6771988c5b6d5513b75f'
    AND status = 'active';

  IF NOT FOUND THEN
    RAISE EXCEPTION 'FiKnight project key was not staged on the expected grant';
  END IF;

  UPDATE agent_creation_requests
  SET customer_org_id = 'org_d0b9190a14c836b00ffc',
      owner_user_id = 'user_85d05606925d474442d7',
      display_name = 'Austin Finite',
      updated_at = clock_timestamp()
  WHERE id = 'agent_request_9edb9d1d2e2ce1c9073f'
    AND customer_org_id = 'org_696a800e548d65b8be93'
    AND owner_user_id = 'user_b9540ab702bd98195b98'
    AND display_name = 'FiKnight'
    AND project_id = 'project_b7e3a5beaf06095c6465'
    AND agent_runtime_id = 'runtime_d8ceb9b4f4e9bacb85b0'
    AND status = 'running';

  IF NOT FOUND THEN
    RAISE EXCEPTION 'FiKnight creation request was not staged exactly';
  END IF;

  UPDATE projects
  SET customer_org_id = 'org_d0b9190a14c836b00ffc',
      owner_user_id = 'user_85d05606925d474442d7',
      display_name = 'Austin Finite',
      agent_email = 'austin-finite-b7e3a5beaf06095c@finite.vip',
      updated_at = clock_timestamp()
  WHERE id = 'project_b7e3a5beaf06095c6465'
    AND customer_org_id = 'org_696a800e548d65b8be93'
    AND owner_user_id = 'user_b9540ab702bd98195b98'
    AND display_name = 'FiKnight'
    AND agent_email = 'fiknight@finite.vip';

  IF NOT FOUND THEN
    RAISE EXCEPTION 'FiKnight Project was not staged exactly';
  END IF;
END
$rollback$;

COMMIT;
