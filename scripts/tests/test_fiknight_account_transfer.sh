#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/fiknight-transfer-test.XXXXXX")
cleanup() {
  pg_ctl -D "$scratch/postgres" -m immediate stop >/dev/null 2>&1 || true
  rm -rf -- "$scratch"
}
trap cleanup EXIT

mkdir "$scratch/socket"
initdb --no-locale --encoding=UTF8 --auth=trust --username=postgres \
  -D "$scratch/postgres" >/dev/null
pg_ctl -D "$scratch/postgres" \
  -o "-F -k $scratch/socket -c listen_addresses=''" \
  -w start >/dev/null

export PGHOST="$scratch/socket"
export PGUSER=postgres
export PGDATABASE=postgres

fixture_kind=synthetic
if [[ -n "${FIKNIGHT_PRODUCTION_DUMP_RSYNC:-}" ]]; then
  fixture_kind=production-snapshot
  test -n "${FIKNIGHT_PRODUCTION_DUMP_SHA256:-}"
  rsync --archive --quiet --rsync-path='sudo rsync' \
    "$FIKNIGHT_PRODUCTION_DUMP_RSYNC" "$scratch/finite_core.dump"
  test "$(shasum -a 256 "$scratch/finite_core.dump" | awk '{print $1}')" = \
    "$FIKNIGHT_PRODUCTION_DUMP_SHA256"
  pg_restore --no-owner --no-acl -d postgres "$scratch/finite_core.dump"
else
  psql -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
CREATE TABLE users (
  id text PRIMARY KEY,
  normalized_email text UNIQUE NOT NULL,
  link_status text NOT NULL,
  workos_user_id text,
  created_at timestamptz NOT NULL,
  updated_at timestamptz NOT NULL
);
CREATE TABLE customer_orgs (
  id text PRIMARY KEY,
  owner_user_id text UNIQUE NOT NULL REFERENCES users(id),
  name text NOT NULL,
  billing_class text NOT NULL,
  created_at timestamptz NOT NULL,
  updated_at timestamptz NOT NULL
);
CREATE TABLE projects (
  id text PRIMARY KEY,
  customer_org_id text NOT NULL REFERENCES customer_orgs(id),
  owner_user_id text NOT NULL REFERENCES users(id),
  display_name text NOT NULL,
  import_candidate_id text,
  created_at timestamptz NOT NULL,
  updated_at timestamptz NOT NULL,
  hosting_tier text,
  placement_runner_class text,
  runtime_resource_class text,
  agent_email text UNIQUE
);
CREATE TABLE agent_runtimes (
  id text PRIMARY KEY,
  project_id text NOT NULL REFERENCES projects(id),
  source_host_id text NOT NULL,
  source_machine_id text NOT NULL,
  runtime_artifact_id text,
  state_schema_version text
);
CREATE TABLE project_runtime_links (
  id text PRIMARY KEY,
  project_id text NOT NULL REFERENCES projects(id),
  agent_runtime_id text NOT NULL REFERENCES agent_runtimes(id),
  active boolean NOT NULL
);
CREATE TABLE runtime_control_requests (
  id text PRIMARY KEY,
  agent_runtime_id text NOT NULL REFERENCES agent_runtimes(id),
  status text NOT NULL
);
CREATE TABLE agent_creation_requests (
  id text PRIMARY KEY,
  customer_org_id text NOT NULL REFERENCES customer_orgs(id),
  owner_user_id text NOT NULL REFERENCES users(id),
  project_id text NOT NULL REFERENCES projects(id),
  display_name text NOT NULL,
  status text NOT NULL,
  agent_runtime_id text REFERENCES agent_runtimes(id),
  updated_at timestamptz NOT NULL
);
CREATE TABLE chat_identities (
  id text PRIMARY KEY,
  user_id text NOT NULL REFERENCES users(id),
  kind text NOT NULL,
  device_id text NOT NULL,
  created_at timestamptz NOT NULL,
  UNIQUE (user_id, kind, device_id)
);
CREATE TABLE project_room_memberships (
  id text PRIMARY KEY,
  project_id text NOT NULL REFERENCES projects(id),
  chat_identity_id text NOT NULL REFERENCES chat_identities(id),
  role text NOT NULL,
  created_at timestamptz NOT NULL,
  archived_at timestamptz,
  UNIQUE (project_id, chat_identity_id)
);
CREATE TABLE finite_private_grants (
  id text PRIMARY KEY,
  user_id text UNIQUE NOT NULL REFERENCES users(id),
  limit_profile_id text NOT NULL,
  status text NOT NULL,
  current_window_started_at timestamptz,
  current_window_used_units bigint NOT NULL,
  burst_window_epoch bigint NOT NULL,
  created_at timestamptz NOT NULL,
  updated_at timestamptz NOT NULL
);
CREATE TABLE finite_private_api_keys (
  id text PRIMARY KEY,
  grant_id text NOT NULL REFERENCES finite_private_grants(id),
  project_id text REFERENCES projects(id),
  agent_runtime_id text REFERENCES agent_runtimes(id),
  status text NOT NULL,
  updated_at timestamptz NOT NULL
);
CREATE TABLE finite_private_admin_audit_events (
  id text PRIMARY KEY,
  action text NOT NULL,
  target_type text NOT NULL,
  target_id text NOT NULL,
  grant_id text REFERENCES finite_private_grants(id),
  api_key_id text REFERENCES finite_private_api_keys(id),
  actor text NOT NULL,
  metadata jsonb NOT NULL,
  created_at timestamptz NOT NULL
);

INSERT INTO users VALUES
  ('user_85d05606925d474442d7', 'austin@finite.vip', 'linked',
   'user_01KSNFQE5SSD07EMMKN9GV1H2S', now(), now()),
  ('user_b9540ab702bd98195b98', 'fiknight@finite.vip', 'linked',
   'user_01M1H6XVR2JYE55RS87ZX1FCFH', now(), now());
INSERT INTO customer_orgs VALUES
  ('org_d0b9190a14c836b00ffc', 'user_85d05606925d474442d7',
   'austin@finite.vip', 'sponsored', now(), now()),
  ('org_696a800e548d65b8be93', 'user_b9540ab702bd98195b98',
   'fiknight@finite.vip', 'standard', now(), now());
INSERT INTO projects VALUES (
  'project_b7e3a5beaf06095c6465', 'org_d0b9190a14c836b00ffc',
  'user_85d05606925d474442d7', 'Austin Finite', NULL, now(), now(),
  'standard', 'kata', 'vcpu4_memory8_gib',
  'austin-finite-b7e3a5beaf06095c@finite.vip'
);
INSERT INTO agent_runtimes VALUES (
  'runtime_d8ceb9b4f4e9bacb85b0', 'project_b7e3a5beaf06095c6465',
  'finite-lat-3', 'finite-kata-9edb9d1d2e2ce1c9073f',
  'finite-agent-runtime-2026-08-29.5', 'runtime-state-v1'
);
INSERT INTO project_runtime_links VALUES (
  'link-fixture', 'project_b7e3a5beaf06095c6465',
  'runtime_d8ceb9b4f4e9bacb85b0', true
);
INSERT INTO agent_creation_requests VALUES (
  'agent_request_9edb9d1d2e2ce1c9073f', 'org_d0b9190a14c836b00ffc',
  'user_85d05606925d474442d7', 'project_b7e3a5beaf06095c6465',
  'Austin Finite', 'running', 'runtime_d8ceb9b4f4e9bacb85b0', now()
);
INSERT INTO chat_identities VALUES (
  'chat_identity_9a8590f157554b14c4b7', 'user_85d05606925d474442d7',
  'hosted_web', 'dashboard-bridge-v1', now()
);
INSERT INTO project_room_memberships VALUES (
  'room_member_d9b3b3e8b96c05b39011', 'project_b7e3a5beaf06095c6465',
  'chat_identity_9a8590f157554b14c4b7', 'owner', now(), NULL
);
INSERT INTO finite_private_grants VALUES (
  'fp_grant_d0b9190a14c836b00ffc', 'user_85d05606925d474442d7',
  'finite-private-generous-v2', 'active', now(), 778487, 177, now(), now()
);
INSERT INTO finite_private_api_keys VALUES (
  'fp_key_deb646d119995d3bd52a', 'fp_grant_d0b9190a14c836b00ffc',
  'project_b7e3a5beaf06095c6465', NULL, 'active', now()
);
SQL
fi

stage="$repo_root/scripts/ops/fiknight-account-transfer-stage.sql"
finalize="$repo_root/scripts/ops/fiknight-account-transfer-finalize.sql"
rollback="$repo_root/scripts/ops/fiknight-account-transfer-rollback.sql"

if psql -v ON_ERROR_STOP=1 -f "$stage" >/dev/null 2>&1; then
  echo "stage unexpectedly ran without the cross-account handoff readiness gate" >&2
  exit 1
fi

psql -v ON_ERROR_STOP=1 -v fiknight_cross_account_handoff_ready=1 -f "$stage" >/dev/null
psql -v ON_ERROR_STOP=1 -v fiknight_cross_account_handoff_ready=1 -f "$stage" >/dev/null

test "$(psql -Atc "SELECT owner_user_id || '|' || display_name || '|' || agent_email FROM projects WHERE id='project_b7e3a5beaf06095c6465'")" = \
  'user_b9540ab702bd98195b98|FiKnight|fiknight@finite.vip'
test "$(psql -Atc "SELECT count(*) FROM project_room_memberships WHERE project_id='project_b7e3a5beaf06095c6465' AND archived_at IS NULL")" = 2
test "$(psql -Atc "SELECT grant_id FROM finite_private_api_keys WHERE id='fp_key_deb646d119995d3bd52a'")" = \
  'fp_grant_6771988c5b6d5513b75f'

psql -v ON_ERROR_STOP=1 -f "$rollback" >/dev/null
test "$(psql -Atc "SELECT owner_user_id || '|' || display_name || '|' || agent_email FROM projects WHERE id='project_b7e3a5beaf06095c6465'")" = \
  'user_85d05606925d474442d7|Austin Finite|austin-finite-b7e3a5beaf06095c@finite.vip'
test "$(psql -Atc "SELECT grant_id FROM finite_private_api_keys WHERE id='fp_key_deb646d119995d3bd52a'")" = \
  'fp_grant_d0b9190a14c836b00ffc'

psql -v ON_ERROR_STOP=1 -v fiknight_cross_account_handoff_ready=1 -f "$stage" >/dev/null
psql -v ON_ERROR_STOP=1 -f "$finalize" >/dev/null
psql -v ON_ERROR_STOP=1 -f "$finalize" >/dev/null

test "$(psql -Atc "SELECT count(*) FROM project_room_memberships m JOIN chat_identities i ON i.id=m.chat_identity_id WHERE m.project_id='project_b7e3a5beaf06095c6465' AND i.user_id='user_b9540ab702bd98195b98' AND m.archived_at IS NULL")" = 1
test "$(psql -Atc "SELECT count(*) FROM project_room_memberships m JOIN chat_identities i ON i.id=m.chat_identity_id WHERE m.project_id='project_b7e3a5beaf06095c6465' AND i.user_id='user_85d05606925d474442d7' AND m.archived_at IS NOT NULL")" = 1

printf 'fiknight account transfer SQL: %s stage/replay/rollback/finalize passed\n' \
  "$fixture_kind"
