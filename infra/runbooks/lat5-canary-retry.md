# Retry the lat5 control canary

Status: **control canary passed on September 15, 2026** (final receipt crossed
into September 16 UTC). Lat5 is drained again with one retained canary. The
execution receipt below supersedes the preparation status; do not repeat the
one-time issuance or binding steps.
This procedure authorizes neither cohort admission nor cleanup of the first
canary. The source TRF and Box 1 agents remain in place.

## Exact scope

The September 15 [failed attempt](lat5-nixos-runner-install.md#controlled-canary-attempt-2026-09-15)
has this recorded chain:

| Assertion | Required value |
| --- | --- |
| Operator | `austin@finite.vip` / `user_01KSNFQE5SSD07EMMKN9GV1H2S` |
| Original code | `launch_code_54432c99f217fa5cbb6f` |
| Original request | `agent_request_d8a77342192705324b16` |
| Original Project | `project_0ca6e90da32abcb9f11b` |
| Original Runtime | `runtime_86af3c895170b9d3be4f` |
| Actual original host | `finite-lat-4` |
| Intended host | `finite-lat-5` |

Preserve the original code, binding, request, Project, Runtime and Chat state.
The original code remains consumed. Add **one fresh code binding** that names
the original binding as its retry parent. The original host reservation remains
present throughout. Ordinary creation stays excluded from lat5.

## Preparation and deployment

1. Run the canonical `scripts/finite-status` on lat2 and lat5. Require the exact
   chain above: original request `running`, original request target NULL, and
   original Runtime still on lat4. Require lat5 drained, zero Kata containers,
   no Core-recorded Runtime on lat5, and no pending targeted creation or active
   control operation for the original Runtime. Verify the effective Runner
   configuration still has the Nix-owned one-runtime ceiling and no operator
   override. Missing or changed evidence stops the procedure.
2. Preserve a **fresh** Core Postgres backup and an off-host copy with matching
   checksum under [the backup runbook](postgres-backup-restore.md). Name the
   previous exact system closure and the candidate CI artifact in the execution
   receipt. The pre-canary backup is not a rollback over newer writes.
3. Deploy the reviewed CI-built lat2 closure through
   [deploy-core.md](deploy-core.md). This installs migration 0027 and the operator
   command. Require the deploy helper's executable check and canonical status's
   six executable checks to pass, with Chat healthy. A stale executable, failed
   activation, or unknown identity stops the procedure before issuing a code.
   The helper does not automatically restart or roll back a service.

## Bind and preview one fresh code

4. In Austin's signed-in admin dashboard, issue a **one-code Standard batch**,
   named `Lat5 controlled canary retry`, with a 24-hour expiry. Save its plaintext
   privately, mode 0600. Record only its code id and batch id in the receipt.
   Do not redeem it yet. Do not reset or reuse the original code.
5. Run the deployed Core binary with the root-only `/etc/finite/core.env`
   environment. Replace the two `NEW_*` identifiers with those exact new records.
   The default is a rollback-only preview; it does not perform schema DDL.

   ```sh
   finite-saas-core launch-code-retry-target-exact \
     --code-id NEW_CODE_ID --expected-batch-id NEW_BATCH_ID \
     --previous-code-id launch_code_54432c99f217fa5cbb6f \
     --expected-previous-request-id agent_request_d8a77342192705324b16 \
     --expected-previous-project-id project_0ca6e90da32abcb9f11b \
     --expected-previous-runtime-id runtime_86af3c895170b9d3be4f \
     --expected-previous-source-host-id finite-lat-4 \
     --target-source-host-id finite-lat-5 \
     --operator-email austin@finite.vip \
     --operator-workos-user-id user_01KSNFQE5SSD07EMMKN9GV1H2S
   ```

6. Require a successful preview with those exact identifiers and `dryRun=true`.
   Recheck drain/ceiling/empty target, then repeat the same command with
   `--execute`. It repeats all database checks in the write transaction and
   appends the binding plus an audit event. A conflicting state fails without
   changing the original records. Repeating an identical unused-code binding
   is idempotent; a different second retry or consumed retry is refused.
7. Capture canonical status. Require the original binding unchanged and exactly
   one new row for `NEW_CODE_ID`, with `retry_of_launch_code_id` equal to the
   original code, `source_host_id=finite-lat-5`, and no creation request yet.
   Do not infer the new row from sort order or its display name.

## Launch and prove the target

8. Redeem the new code through the dashboard for `Lat5 Canary Retry` while lat5
   is still drained. Record its exact new Project and request identifiers.
   Require `request_status=requested`, `request_target_source_host_id=finite-lat-5`,
   and empty Runner, Runtime and actual-host fields. Any other result stops the
   procedure with lat5 still drained. Preserve the records for investigation.
9. Enable only lat5 at its one-runtime ceiling. Use canonical status to verify
   the resulting Runtime belongs to the exact new Project and actually runs on
   `finite-lat-5`. Verify identity readiness and a real reply in that Project's
   Chat. Reload the conversation and verify the reply remains available.
10. Drain lat5 again. Record the final code/request/Runtime/host chain, Chat
    result, unchanged original Runtime, and existing lat3/lat4 readiness. Keep
    the new Runtime and all history. The misplaced lat4 canary's eventual
    retirement is a separate reviewed operation.

## Database boundary and recovery

Migration 0027 adds retry lineage to the existing binding table. The original
host uniqueness rule remains for root bindings; each root permits one retry
child, on the same host. The operator refuses a retry of a retry, wrong issuer
or owner, missing or changed original records, an occupied target, in-flight
work, and a replacement code that is used, expired, revoked, from another
issuer, or not from a one-code Standard batch. A concurrent second retry cannot
win. Drain and physical capacity remain explicit host-local preflight checks;
Core cannot observe them atomically with this database transaction.

The retry writer appends one binding and `launch_code.retry_target_host` audit
record. Redemption still reads one binding by code id; leasing still reads the
saved request target and the host reservation. Those production reader queries
and Runner/dashboard contracts are unchanged. Tests replay migration 0026
against the new rows to prove N-1 startup retains the index, and exercise the
unchanged code lookup and lease paths. This is not proof that a pre-0026 Core
can run safely: it cannot enforce target bindings.

On any failure, keep lat5 drained. Before redemption, the new unused batch can
be revoked through the normal admin path; its binding and audit remain. After
redemption, preserve the new request/Runtime and diagnose the exact failure.
Do not reset codes, delete bindings, rewrite Runtime location, or restore a
whole database over newer writes. Retain migration 0027 and its history during
binary rollback; Core versions before targeted-code support remain prohibited.
The existing [targeted-canary rollback rules](targeted-agent-canary.md#writers-readers-and-rollback)
also apply. Neither a failed retry nor a successful one releases cohort capacity.

## Completed canary — September 15, 2026

[PR #899](https://github.com/finitecomputer/finite-mono/pull/899) deployed Core
revision `2a89fbe2bd45140376937dfdf0037ef4d0f99a8c` through
[CI build 35034520806](https://github.com/finitecomputer/finite-mono/actions/runs/35034520806).
All six executable checks passed; Chat and the other application services kept
their processes. [Execution evidence](https://github.com/finitecomputer/finite-mono/pull/900)
records the backup checksum, system paths, and verification details.

| Retry record | Verified value |
| --- | --- |
| Batch / code | `launch_batch_cf32944485aecf41834a` / `launch_code_1b61135ee574150fe107` |
| Request / Project | `agent_request_0d08e066bb8b01014ddf` / `project_7447b276571011033370` |
| Runtime | `runtime_304dc2797ddc2e7d4f7c` |
| Target / Runner / actual host | `finite-lat-5` / `finite-kata-runner-5` / `finite-lat-5` |

The request was verified targeted and unclaimed while lat5 was drained. After
launch, the Chat reply `LAT5-CANARY-20260915-OK` survived a fresh page load.
Lat5 is drained again, capped at one runtime, with the healthy canary retained.
Final readiness: lat3 31/31, lat4 28/28, lat5 1/1. Both bindings and the original
misplaced lat4 canary remain intact. No TRF/Box 1 migration or cohort admission
occurred; lat1 was untouched.

The pre-deploy backup is on lat2 at
`/data/backups/postgres/finite_core-pre-lat5-retry-20260915.dump`, with a verified
off-host copy under `~/.local/state/finite/lat5-canary-20260915/core-pre-retry.dump`.
Its local restore and migration test passed. Do not restore it over newer writes.
