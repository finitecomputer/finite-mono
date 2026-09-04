\set ON_ERROR_STOP on

\if :{?fiknight_cross_account_handoff_ready}
\else
\set fiknight_cross_account_handoff_ready false
\endif

\if :fiknight_cross_account_handoff_ready
\else
\echo 'BLOCKED: cross-account canonical Room membership and complete-history handoff is not implemented or rehearsed; see docs/runs/fiknight-chat-handoff-investigation-2026-09-02.md'
SELECT CAST('blocked_cross_account_handoff_not_ready' AS integer);
\endif

-- One-time, exact FiKnight production operation, refreshed on 2026-09-03
-- after the Runtime advanced from the paused attempt's .1 artifact to .2.
-- Run from the operator workstation; do not install this file on a host.
-- This stage is additive for Chat access: Austin remains a room member until
-- scripts/ops/fiknight-account-transfer-finalize.sql is run after acceptance.

BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
SELECT pg_advisory_xact_lock(hashtextextended('fiknight-account-transfer-2026-09-02', 0));

DO $transfer$
DECLARE
  project_row projects%ROWTYPE;
  request_row agent_creation_requests%ROWTYPE;
  target_user users%ROWTYPE;
  target_org customer_orgs%ROWTYPE;
  existing_identity chat_identities%ROWTYPE;
  existing_membership project_room_memberships%ROWTYPE;
  existing_grant finite_private_grants%ROWTYPE;
  key_row finite_private_api_keys%ROWTYPE;
  changed_count bigint;
BEGIN
  SELECT * INTO STRICT target_user
  FROM users
  WHERE id = 'user_b9540ab702bd98195b98'
    AND normalized_email = 'fiknight@finite.vip'
    AND link_status = 'linked'
    AND workos_user_id = 'user_01M1H6XVR2JYE55RS87ZX1FCFH'
  FOR UPDATE;

  SELECT * INTO STRICT target_org
  FROM customer_orgs
  WHERE id = 'org_696a800e548d65b8be93'
    AND owner_user_id = target_user.id
  FOR UPDATE;

  IF EXISTS (
    SELECT 1 FROM projects
    WHERE owner_user_id = target_user.id
      AND id <> 'project_b7e3a5beaf06095c6465'
  ) THEN
    RAISE EXCEPTION 'FiKnight target account unexpectedly owns another Project';
  END IF;

  SELECT * INTO STRICT project_row
  FROM projects
  WHERE id = 'project_b7e3a5beaf06095c6465'
  FOR UPDATE;

  IF NOT (
    (
      project_row.customer_org_id = 'org_d0b9190a14c836b00ffc'
      AND project_row.owner_user_id = 'user_85d05606925d474442d7'
      AND project_row.display_name = 'Austin Finite'
      AND project_row.agent_email = 'austin-finite-b7e3a5beaf06095c@finite.vip'
    ) OR (
      project_row.customer_org_id = target_org.id
      AND project_row.owner_user_id = target_user.id
      AND project_row.display_name = 'FiKnight'
      AND project_row.agent_email = 'fiknight@finite.vip'
    )
  ) THEN
    RAISE EXCEPTION 'Project is neither at the expected source nor staged state';
  END IF;

  IF project_row.import_candidate_id IS NOT NULL
     OR project_row.hosting_tier <> 'standard'
     OR project_row.placement_runner_class <> 'kata'
     OR project_row.runtime_resource_class <> 'vcpu4_memory8_gib' THEN
    RAISE EXCEPTION 'Project product/placement fence changed';
  END IF;

  IF NOT EXISTS (
    SELECT 1
    FROM project_runtime_links link
    JOIN agent_runtimes runtime ON runtime.id = link.agent_runtime_id
    WHERE link.project_id = project_row.id
      AND link.active
      AND runtime.id = 'runtime_d8ceb9b4f4e9bacb85b0'
      AND runtime.source_host_id = 'finite-lat-3'
      AND runtime.source_machine_id = 'finite-kata-9edb9d1d2e2ce1c9073f'
      AND runtime.runtime_artifact_id = 'finite-agent-runtime-2026-09-02.2'
      AND runtime.state_schema_version = 'runtime-state-v1'
  ) THEN
    RAISE EXCEPTION 'Exact active Runtime binding changed';
  END IF;

  IF EXISTS (
    SELECT 1 FROM runtime_control_requests
    WHERE agent_runtime_id = 'runtime_d8ceb9b4f4e9bacb85b0'
      AND status IN ('requested', 'launching', 'compute_up', 'ready')
  ) THEN
    RAISE EXCEPTION 'Runtime has an in-flight control operation';
  END IF;

  SELECT * INTO STRICT request_row
  FROM agent_creation_requests
  WHERE id = 'agent_request_9edb9d1d2e2ce1c9073f'
    AND project_id = project_row.id
    AND agent_runtime_id = 'runtime_d8ceb9b4f4e9bacb85b0'
    AND status = 'running'
  FOR UPDATE;

  IF NOT (
    (
      request_row.customer_org_id = 'org_d0b9190a14c836b00ffc'
      AND request_row.owner_user_id = 'user_85d05606925d474442d7'
      AND request_row.display_name = 'Austin Finite'
    ) OR (
      request_row.customer_org_id = target_org.id
      AND request_row.owner_user_id = target_user.id
      AND request_row.display_name = 'FiKnight'
    )
  ) THEN
    RAISE EXCEPTION 'Creation request is neither at the expected source nor staged state';
  END IF;

  SELECT * INTO STRICT existing_membership
  FROM project_room_memberships
  WHERE id = 'room_member_d9b3b3e8b96c05b39011'
    AND project_id = project_row.id
    AND chat_identity_id = 'chat_identity_9a8590f157554b14c4b7'
    AND role = 'owner'
    AND archived_at IS NULL
  FOR UPDATE;

  IF NOT EXISTS (
    SELECT 1 FROM chat_identities
    WHERE id = existing_membership.chat_identity_id
      AND user_id = 'user_85d05606925d474442d7'
      AND kind = 'hosted_web'
      AND device_id = 'dashboard-bridge-v1'
  ) THEN
    RAISE EXCEPTION 'Austin source Chat identity changed';
  END IF;

  SELECT * INTO existing_identity
  FROM chat_identities
  WHERE id = 'chat_identity_87d8015f0f068fd16b6d'
  FOR UPDATE;

  IF FOUND THEN
    IF existing_identity.user_id <> target_user.id
       OR existing_identity.kind <> 'hosted_web'
       OR existing_identity.device_id <> 'dashboard-bridge-v1' THEN
      RAISE EXCEPTION 'FiKnight deterministic Chat identity conflicts';
    END IF;
  ELSE
    INSERT INTO chat_identities (id, user_id, kind, device_id, created_at)
    VALUES (
      'chat_identity_87d8015f0f068fd16b6d', target_user.id,
      'hosted_web', 'dashboard-bridge-v1', clock_timestamp()
    );
  END IF;

  SELECT * INTO existing_membership
  FROM project_room_memberships
  WHERE id = 'room_member_def47a34609964844c10'
  FOR UPDATE;

  IF FOUND THEN
    IF existing_membership.project_id <> project_row.id
       OR existing_membership.chat_identity_id <> 'chat_identity_87d8015f0f068fd16b6d'
       OR existing_membership.role <> 'owner' THEN
      RAISE EXCEPTION 'FiKnight deterministic room membership conflicts: project=%, identity=%, role=%, archived=%',
        existing_membership.project_id,
        existing_membership.chat_identity_id,
        existing_membership.role,
        existing_membership.archived_at;
    END IF;
    UPDATE project_room_memberships
    SET archived_at = NULL
    WHERE id = existing_membership.id
      AND archived_at IS NOT NULL;
  ELSE
    INSERT INTO project_room_memberships
      (id, project_id, chat_identity_id, role, created_at, archived_at)
    VALUES (
      'room_member_def47a34609964844c10', project_row.id,
      'chat_identity_87d8015f0f068fd16b6d', 'owner', clock_timestamp(), NULL
    );
  END IF;

  SELECT * INTO existing_grant
  FROM finite_private_grants
  WHERE id = 'fp_grant_6771988c5b6d5513b75f'
  FOR UPDATE;

  IF FOUND THEN
    IF existing_grant.user_id <> target_user.id
       OR existing_grant.limit_profile_id <> 'finite-private-generous-v2'
       OR existing_grant.status <> 'active' THEN
      RAISE EXCEPTION 'FiKnight Finite Private grant conflicts';
    END IF;
  ELSE
    INSERT INTO finite_private_grants (
      id, user_id, limit_profile_id, status, current_window_started_at,
      current_window_used_units, burst_window_epoch, created_at, updated_at
    ) VALUES (
      'fp_grant_6771988c5b6d5513b75f', target_user.id,
      'finite-private-generous-v2', 'active', NULL, 0, 0,
      clock_timestamp(), clock_timestamp()
    );
  END IF;

  SELECT * INTO STRICT key_row
  FROM finite_private_api_keys
  WHERE id = 'fp_key_deb646d119995d3bd52a'
    AND project_id = project_row.id
    AND agent_runtime_id IS NULL
    AND status = 'active'
  FOR UPDATE;

  IF key_row.grant_id NOT IN (
    'fp_grant_d0b9190a14c836b00ffc',
    'fp_grant_6771988c5b6d5513b75f'
  ) THEN
    RAISE EXCEPTION 'Project-scoped Finite Private key has an unexpected grant';
  END IF;

  IF (SELECT count(*) FROM finite_private_api_keys
      WHERE project_id = project_row.id AND status = 'active') <> 1 THEN
    RAISE EXCEPTION 'Project does not have exactly one active Finite Private key';
  END IF;

  UPDATE finite_private_api_keys
  SET grant_id = 'fp_grant_6771988c5b6d5513b75f', updated_at = clock_timestamp()
  WHERE id = key_row.id
    AND grant_id = 'fp_grant_d0b9190a14c836b00ffc';

  UPDATE projects
  SET customer_org_id = target_org.id,
      owner_user_id = target_user.id,
      display_name = 'FiKnight',
      agent_email = 'fiknight@finite.vip',
      updated_at = clock_timestamp()
  WHERE id = project_row.id
    AND customer_org_id = 'org_d0b9190a14c836b00ffc'
    AND owner_user_id = 'user_85d05606925d474442d7'
    AND display_name = 'Austin Finite'
    AND agent_email = 'austin-finite-b7e3a5beaf06095c@finite.vip';

  UPDATE agent_creation_requests
  SET customer_org_id = target_org.id,
      owner_user_id = target_user.id,
      display_name = 'FiKnight',
      updated_at = clock_timestamp()
  WHERE id = request_row.id
    AND customer_org_id = 'org_d0b9190a14c836b00ffc'
    AND owner_user_id = 'user_85d05606925d474442d7'
    AND display_name = 'Austin Finite';

  INSERT INTO finite_private_admin_audit_events (
    id, action, target_type, target_id, grant_id, api_key_id,
    actor, metadata, created_at
  ) VALUES (
    'fp_audit_' || substr(md5('fiknight-account-transfer-stage-2026-09-02'), 1, 20),
    'finite_private.api_key.account_transfer', 'api_key', key_row.id,
    'fp_grant_6771988c5b6d5513b75f', key_row.id, 'austin@finite.vip',
    jsonb_build_object(
      'projectId', project_row.id,
      'fromUserId', 'user_85d05606925d474442d7',
      'toUserId', target_user.id,
      'fromGrantId', 'fp_grant_d0b9190a14c836b00ffc',
      'toGrantId', 'fp_grant_6771988c5b6d5513b75f'
    ),
    clock_timestamp()
  ) ON CONFLICT (id) DO NOTHING;

  SELECT count(*) INTO changed_count
  FROM projects
  WHERE id = project_row.id
    AND customer_org_id = target_org.id
    AND owner_user_id = target_user.id
    AND display_name = 'FiKnight'
    AND agent_email = 'fiknight@finite.vip';
  IF changed_count <> 1 THEN
    RAISE EXCEPTION 'Staged Project postcondition failed';
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM agent_creation_requests
    WHERE id = request_row.id
      AND customer_org_id = target_org.id
      AND owner_user_id = target_user.id
      AND display_name = 'FiKnight'
      AND status = 'running'
      AND agent_runtime_id = 'runtime_d8ceb9b4f4e9bacb85b0'
  ) THEN
    RAISE EXCEPTION 'Staged creation-request postcondition failed';
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM finite_private_api_keys
    WHERE id = key_row.id
      AND grant_id = 'fp_grant_6771988c5b6d5513b75f'
      AND status = 'active'
  ) THEN
    RAISE EXCEPTION 'Staged Finite Private key postcondition failed';
  END IF;
END
$transfer$;

COMMIT;

SELECT p.id AS project_id, p.display_name, p.agent_email,
       owner.normalized_email AS owner_email,
       org.name AS organization,
       runtime.id AS runtime_id, runtime.source_host_id,
       runtime.source_machine_id, runtime.runtime_artifact_id,
       runtime.state_schema_version
FROM projects p
JOIN users owner ON owner.id = p.owner_user_id
JOIN customer_orgs org ON org.id = p.customer_org_id
JOIN project_runtime_links link ON link.project_id = p.id AND link.active
JOIN agent_runtimes runtime ON runtime.id = link.agent_runtime_id
WHERE p.id = 'project_b7e3a5beaf06095c6465';

SELECT membership.id, owner.normalized_email, membership.role, membership.archived_at
FROM project_room_memberships membership
JOIN chat_identities identity ON identity.id = membership.chat_identity_id
JOIN users owner ON owner.id = identity.user_id
WHERE membership.project_id = 'project_b7e3a5beaf06095c6465'
ORDER BY owner.normalized_email;
