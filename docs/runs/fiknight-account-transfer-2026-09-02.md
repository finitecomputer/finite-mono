# FiKnight account transfer — 2026-09-02

> **PRODUCTION PAUSED — do not resume this cutover or reuse any staged
> artifact.** The local Chat handoff rehearsal passed, but the current
> writer-fenced method causes platform-wide Chat downtime. Austin paused the
> migration on 2026-09-02 after service was restored. Any later attempt requires
> a newly approved availability plan, fresh backup and stopped-writer copy,
> refreshed Runtime fence, and new explicit authorization. The Core-only
> transfer does not transfer
> cryptographic Chat Room membership or retained history. The first attempt was
> rolled back after FiKnight received a new empty Room. See
> [the cross-account Chat handoff investigation](fiknight-chat-handoff-investigation-2026-09-02.md).

## Authorized live cutover checkpoint

Austin explicitly authorized steps 1–4 end to end in the operator chat on
2026-09-02. The authorization covers the FiKnight production cutover only; it
does not authorize the later R2D2 migration.

The current Runtime artifact drifted after the first rehearsal from
`finite-agent-runtime-2026-08-29.5` to
`finite-agent-runtime-2026-09-02.1`. The exact SQL fence and synthetic fixture
were updated to the new artifact. Stage, exact replay, rollback, a second
stage, and finalize passed against both synthetic state and the fresh
production snapshot dump before the live mutation boundary.

The fresh coordinated Recovery Snapshot and rollback boundary is
`/data/recovery-snapshots/hosted-web-chat/20260902T162316Z`. Its sealed v3
manifest passed in full. SHA-256 evidence:

- manifest: `4553f70a8d6890cd9cd50f266951331cf1111dab5e750209915cd8dd1ba81c3e`
- Core dump: `c11aab9e167e94cd1c2e64a934c70f9d971c62c2a5a5506fdb08486b97db3cac`
- Finite Identity database: `b84f041978d69a353e23f1c5109d72a74e318a894d655032f834e7bb8f490fb5`
- Room server: `8b2049b20ff61b27075b51a6d11f4174d7d7b451e761dd914b7c78ac930a292d`
- Austin hosted store: `b43d2557560d00bfc72aa653b72c4bb14e897af52ced61547f74716c61460000`
- FiKnight hosted store: `01dc6f4093e9e499b5bc68939df79cc2a2232ea411237a3887bd0338d26c6135`

The canonical pre-cutover `finite-status` result is retained locally. Chat,
the app host, storage, HTTP services, and all recovery boundaries were green.
The aggregate remained red only for the already-recorded stale `Smoke Studio`
Core row; lat3 lifecycle detail was unavailable from the lat2 collector. The
FiKnight Runtime remained online on the exact expected lat3 machine and current
artifact.

### Paused production-cutover checkpoint

The authorized live cutover was started on 2026-09-02 and then paused at the
user's request because the current procedure stops the shared Room server and
Hosted Device service. The durable-data scope remained limited to the selected
FiKnight/Austin Project and Room, but the availability impact applied to every
Chat user while those shared writers were stopped.

The live attempt behaved as follows:

1. Several initial writer-fence attempts failed closed before mutation on
   service-state, SQLite-path, and unit-ordering preconditions. Each guard
   restarted the original services.
2. The first actual install atomically swapped the three transformed encrypted
   SQLite files and FiKnight's sealed binding. The subsequent Core stage could
   not open the root-only SQL path as the `postgres` user. The Core transaction
   never started. The rollback trap stopped the transformed files, restored all
   four originals and their sidecars, and restarted the original services.
3. Read-only rollback verification found the original sealed-binding hash, the
   Project still owned by Austin, Austin's membership active, and FiKnight's
   membership archived. The failed transformed files were retained root-only
   under the `.fiknight-failed-20260902` suffix for audit; they are not install
   candidates.
4. Because services had resumed and the global Room database advanced, attempt
   2 deliberately did not reuse the first install image. A new writer-fenced
   ciphertext copy was pulled with a new migration ID. The user then paused the
   migration. No attempt-2 handoff ledger, transformed install image, atomic
   replacement, or Core transaction was produced.
5. The original services were restored. No migration process or writer fence
   remains active.

The operator estimate for an otherwise successful run of this exact method is
15–20 minutes of platform-wide Chat unavailability; a 30-minute maintenance
window is required for verification or rollback margin. That estimate is not a
measured availability guarantee. Eliminating platform-wide downtime requires a
different online/per-Room migration design and a new rehearsal; the existing
local-only utility intentionally refuses production endpoints and unmarked
stores.

A canonical read-only `finite-status --json` checkpoint was collected from
`finite-lat-2` at `2026-09-03T16:44:53Z` after the pause:

- host health was green; Core, dashboard, Room server, Hosted Device, Brain,
  Sites, and node exporter were all active, and all recorded HTTP probes were
  green;
- the Chat plane was green, including the server watermark;
- snapshot, Litestream, and Borg recovery boundaries were green;
- rollout state was green;
- the overall result remained red only because fleet convergence still contains
  the previously documented stale `Smoke Studio` row;
- the promoted Runtime target is now `finite-agent-runtime-2026-09-02.2`, so
  the `.1` SQL/artifact fence proven before the attempt is stale again.

The committed
[post-pause status evidence](fiknight-pause-status-2026-09-03.json) contains
the exact summarized values and the SHA-256 of the private raw collector output.

All workstation attempt directories, host staging files, rollback copies,
prepared MLS removals, evidence ledgers, migration IDs, digests, and install
hashes from this attempt are evidence only. They must never be submitted,
installed, or used as the starting point for a later cutover after services
have resumed.

Before any future production mutation, choose one of these paths explicitly:

- approve a platform-wide maintenance window, then create a fresh coordinated
  Recovery Snapshot and repeat the entire stopped-writer export, rehearsal,
  install, and verification sequence from new state; or
- implement and rehearse an online/per-Room handoff that does not replace the
  shared live database.

Either path also requires refreshing the exact Runtime artifact fence, rerunning
stage/replay/rollback/finalize against the fresh Core dump, confirming current
Room/account membership, and obtaining new explicit production authorization.

### Writer-fenced local-to-production procedure

1. Stop the dashboard, Core, Hosted Device, and Room server. Confirm no process
   has any of the three exact SQLite databases open. The Agent Runtime may stay
   online; its Chat retries cannot write while the Room server is stopped.
2. While the writers remain stopped, create clean SQLite online-backup images
   of the Room server and the exact Austin and FiKnight hosted stores in a
   root-only cutover directory. Copy FiKnight's exact sealed binding. These
   files are the immediate pre-install rollback boundary; the sealed Recovery
   Snapshot above remains the full-system rollback boundary.
3. Seed private workstation copies from the immutable snapshot, rsync only the
   stopped-writer deltas, add the exact scratch markers, and run the loopback
   `inspect`, `join`, `plan`, `apply`, replay, `prepare-remove`,
   `submit-remove`, binding replacement/replay, and restart `verify` phases.
   Decrypted history remains in process memory.
4. Stop the loopback server. Create clean SQLite backup images of its three
   modified databases, run `PRAGMA integrity_check`, and record SHA-256 for the
   three databases and sealed binding.
5. Upload to a root-only staging directory on the app host. Verify hashes and
   SQLite integrity before installation. Preserve the deployed owner/group and
   mode, move any old WAL/SHM sidecars into the rollback directory, and replace
   only the exact Room server database, two hosted client databases, and one
   FiKnight sealed binding while all writers remain stopped.
6. Run the staged Core SQL with
   `fiknight_cross_account_handoff_ready=1`, then start the Room server, Hosted
   Device, Core, and dashboard in dependency order. A failure before the public
   NIP-05 commit restores the four files from the immediate rollback directory,
   removes their new sidecars, runs the Core rollback SQL if stage committed,
   and restarts the original services.
7. Require the FiKnight browser to project the exact recorded history, the
   historical Room as canonical, only FiKnight plus the unchanged Agent, and a
   fresh successful message/reply before Runtime replacement or public NIP-05
   binding.

This is the operator ledger for moving the existing `Austin Finite` Project to
the dedicated `fiknight@finite.vip` account. The operation is run from this
unmerged draft branch. No application code, image, or NixOS generation is
deployed.

## Exact identity

- Project: `project_b7e3a5beaf06095c6465`
- Runtime: `runtime_d8ceb9b4f4e9bacb85b0`
- machine: `finite-kata-9edb9d1d2e2ce1c9073f` on `finite-lat-3`
- Runtime artifact: `finite-agent-runtime-2026-09-02.1`
- state schema: `runtime-state-v1`
- Agent Principal: `npub1r83u6s59v5956l5gd6my6vjqk9x0rkjef78ntchs494m5y6tq4dqychqrv`
- source account: `austin@finite.vip`
- target account: `fiknight@finite.vip`
- source name/NIP-05: `Austin Finite` / `austin-finite-b7e3a5beaf06095c@finite.vip`
- target name/NIP-05: `FiKnight` / `fiknight@finite.vip`

The target Google Workspace and WorkOS account was created as `Fiknight
Finite`. Core linked it as `user_b9540ab702bd98195b98` with personal
organization `org_696a800e548d65b8be93`. No credential value belongs in this
ledger or branch.

The fresh pre-change coordinated Recovery Snapshot is
`/data/recovery-snapshots/hosted-web-chat/20260902T142826Z`. Its manifest
passed in full. SHA-256 evidence:

- manifest: `053512249efce2f0001d41717977b1200f027f831a0229dfd2528b01f66132cd`
- Core dump: `529248577465112bad76a1a4fd1c15d3ecf3846a75d83d52eb33d7209f1357db`
- Finite Identity database: `546f32284902cc0e6502bdb8a28eafc9bb09f798ab3993460d0330ee66dd1f6a`

The exact Core dump was restored into an isolated local PostgreSQL instance.
The production rows passed stage, exact replay, rollback, a second stage, and
finalization without modifying production.

`fiknight@finite.vip` is intentionally both a deliverable Google mailbox and a
Managed Agent NIP-05. Gmail remains deliverable. Finite Sites callers must use
the typed `--nip05 fiknight@finite.vip` form for Agent grants rather than the
typed `--email` form.

## Recovery and ordering

The ordering below is retained as the original reviewed proposal and is not an
executable runbook. Steps 3–4 demonstrated the missing cross-account Chat
handoff. The replacement now passes a local production-shaped rehearsal, but a
writer-fenced local-to-production cutover procedure is still required before
another live attempt.

1. Require a fresh successful `finite-hosted-web-chat-snapshot.service` run.
   Record its directory and verify its manifest. This is the pre-change
   rollback boundary for Core, Chat, Hosted Device, and Finite Identity.
2. Confirm the Runtime `/contact` document returns the exact Agent Principal
   above and that `fiknight@finite.vip` is still unbound.
3. Run the staged Core transaction from the workstation:

   ```sh
   ssh finite-lat-2 'sudo -u postgres psql -d finite_core' \
     < scripts/ops/fiknight-account-transfer-stage.sql
   ```

   This transfers the Project, creation request, and scoped Finite Private key;
   creates FiKnight's deterministic hosted Chat identity and active owner
   membership; and deliberately keeps Austin's membership active.
4. In FiKnight's isolated browser session, open the existing Project. Require
   the existing conversation history and a successful new message/reply before
   continuing. If this fails, run the rollback SQL before creating the new
   NIP-05 binding.
5. Recreate only this Runtime with its existing digest-pinned artifact through
   the existing exact rollout command. This blue/green path preserves the
   durable state, verifies the Agent Principal, and refreshes the three runtime
   name variables to `FiKnight`. First run `--plan-only`; the execution must
   name only the exact Project, Runtime, host, machine, and current artifact.
6. Recheck `/contact`, the Runtime's three name variables, chat history, and a
   new message/reply.
7. Bind `fiknight@finite.vip` to the exact Agent Principal through Finite
   Identity's loopback operator endpoint. This is the public identity commit
   point: the name is intentionally durable and non-reassignable. Keep the old
   NIP-05 active as a rollback alias during observation.
8. Require both public NIP-05 routes to resolve `fiknight` to the exact Agent
   Principal. Then run:

   ```sh
   ssh finite-lat-2 'sudo -u postgres psql -d finite_core' \
     < scripts/ops/fiknight-account-transfer-finalize.sql
   ```

   This archives only Austin's old Project membership. It does not delete the
   user, history, Runtime, key material, or old NIP-05.
9. From FiKnight's Connections page, start a fresh Google Workspace OAuth
   authorization. The old Austin Workspace token is not copied. Confirm the
   connected address is exactly `fiknight@finite.vip`, then make one read-only
   Drive or Calendar request through the Agent.

## Rollback boundary

Before the new NIP-05 is bound, run
`scripts/ops/fiknight-account-transfer-rollback.sql` to return Project control,
the creation request, the scoped inference key, and active Chat membership to
Austin. The retained FiKnight user, personal organization, inactive Core
membership, and unused grant are audit state. A completed hosted Chat bootstrap
is separate durable state: the first attempt left FiKnight with a new empty
Room and sealed binding, which the Core rollback does not remove.

If the same-artifact replacement already completed, first restore the Core
state with the rollback SQL and then run the same exact replacement again so
the Runtime name variables return to `Austin Finite`. Do not disable the new
NIP-05 as an automatic rollback: a disabled v1 binding cannot be silently
re-enabled.

## Acceptance

- Project owner and organization resolve only to `fiknight@finite.vip`.
- Austin's Project membership is archived; FiKnight's is active and `owner`.
- Project, Runtime, machine, artifact, schema, Agent Principal, durable state,
  rooms, messages, and message authorship are unchanged.
- Project display and all three runtime name variables equal `FiKnight`.
- The active Project-scoped Finite Private key belongs to FiKnight's grant.
- `fiknight@finite.vip` resolves as `managed_agent` to the exact Agent
  Principal through `identity.finite.vip` and the `finite.vip` apex route.
- FiKnight's Google Workspace connection reports exactly
  `fiknight@finite.vip`; Austin's connection was not copied.
