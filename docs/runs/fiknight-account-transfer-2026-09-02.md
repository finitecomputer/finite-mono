# FiKnight account transfer — 2026-09-02

> **OUTAGE BOUNDARY PAUSED — do not stop a shared writer without Austin's
> fresh explicit confirmation.** Online preparation resumed on 2026-09-03,
> but the current writer-fenced method causes platform-wide Chat downtime.
> Every artifact from the 2026-09-02 attempt remains audit-only. The resumed
> attempt uses a fresh backup, migration ID, receipts, binaries, and scratch
> copies; it still requires a new stopped-writer copy after the outage is
> authorized. The Core-only transfer does not transfer
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

## Resumed production-cutover checkpoint — 2026-09-03 CDT

Austin authorized resuming the FiKnight production cutover, including the
previously described platform-wide Chat outage and 30-minute rollback window.
This authorization remains limited to FiKnight; it does not authorize R2D2 or
either additional candidate Agent. On resumption, Austin also required a fresh,
explicit confirmation immediately before any shared writer is stopped; online
preparation and the separate apex-route repair do not cross that boundary.

The required online preflight was repeated from current state:

- canonical `finite-status --json` at `2026-09-04T02:22:43Z` found Chat, host
  health, recovery boundaries, and rollout state green; the aggregate remained
  red only for the pre-existing stale `Smoke Studio` Core row. The private raw
  report SHA-256 is
  `34db582ac978ee4dd93d78513dcfc7f3b544cc9ad62b2786fee2d5ee2cb7229b`;
- Core still had the exact Austin-owned Project and creation request, active
  Austin owner membership, archived FiKnight owner membership, Austin-scoped
  Finite Private grant, no other FiKnight-owned Project, and no in-flight
  Runtime control request;
- the exact active Runtime remained on `finite-lat-3` machine
  `finite-kata-9edb9d1d2e2ce1c9073f`, but now used
  `finite-agent-runtime-2026-09-02.2`; its live `/contact` was ready and
  returned the unchanged Agent Principal;
- the SQL fence and synthetic fixture were refreshed from `.1` to `.2`.
  Synthetic stage/replay/rollback/finalize passed, then the same lifecycle
  passed against the fresh production dump below;
- the corrected Core handoff streams the local reviewed SQL into remote
  `psql` over stdin. The `postgres` process never opens a root-only remote SQL
  path, eliminating the first live attempt's failure mode.

The new coordinated Recovery Snapshot is
`/data/recovery-snapshots/hosted-web-chat/20260904T022527Z`. Its sealed v3
manifest and snapshot-health unit passed, the encrypted off-host Borg job
completed successfully at `2026-09-04T02:30:20Z`, and all fenced services
returned active. SHA-256 evidence:

- manifest: `3bd3eee691b69de51622f217e0c1688acdbab50c9abf16e2b7cc6b788409da5c`
- Core dump: `90e79e0048876b778cf99e654bda06b6a30b9e8be09cba67d5f1e76abfa4a1c6`
- Finite Identity database: `b84f041978d69a353e23f1c5109d72a74e318a894d655032f834e7bb8f490fb5`
- Room server: `b6b4ed7c53c027fe0f3224c18c7542df7b31710c528a7340c381688f1b541b4a`
- Austin hosted store: `d1c1b5612148a8f8bd397f56f8add2c35e17c2adabaf6224ebc30f2f4f3ed682`
- FiKnight hosted store: `01dc6f4093e9e499b5bc68939df79cc2a2232ea411237a3887bd0338d26c6135`
- FiKnight sealed binding: `144a40ca67af479750b51c3512443fbca52ca270a5628cc992b40d0311f11d60`

One separate pre-existing acceptance blocker was discovered and repaired under
its own rollback boundary. The direct `identity.finite.vip` NIP-05 route was
healthy and reported `fiknight` unbound, but the canonical `finite.vip` apex
route returned HTTP 502. Read-only evidence from clawland proved its
selectorless Endpoint named the retired `64.34.82.77`; that address was
unreachable, while the replacement app-plane address `64.34.80.19` returned
byte-identical Authority output when called from clawland with the pinned TLS
server name.

Austin authorized this separate online repair on 2026-09-03. Commit `940e1420`
changed only the checked-in Endpoint from `64.34.82.77` to `64.34.80.19`;
server-side dry-run passed and `kubectl diff` showed only that IP change. The
live apply preserved Endpoint UID `99edd2cb-23ad-4cbf-9210-667d84d1667b` and
advanced it to resource version `37486600`. Both public origins then returned
HTTP 200 and byte-identical empty `names` objects for an unbound sentinel and
for `fiknight`; the durable FiKnight name remains unbound. The post-repair
`finite-status` at `2026-09-04T02:41:54Z` retained SHA-256
`3b6df628f59b62ebfef275993eeb8809fedd5d9eecff8baaec5d781f6e7c952e` and
showed Chat, host health, recovery, and rollout state green, with only the
accepted stale `Smoke Studio` row keeping aggregate fleet convergence red.

The fresh snapshot was copied into owner-only private root
`/tmp/fiknight-production-cutover-20260904T022527Z` without using any earlier
attempt as an rsync basis. Independent hashes of its Room, Austin client,
FiKnight client, and FiKnight sealed binding matched the sealed manifest above.
The old and current private roots contain no group- or other-accessible path.

The first two current-snapshot rehearsals failed closed before producing an
accepted handoff:

- `rehearsal-v3` timed out while starting the loopback Room server; no handoff
  phase ran. The driver now waits up to 60 seconds and also fails if its exact
  spawned server PID exits.
- `rehearsal-v3b` reached the read-only inspect phase but could not decrypt the
  production client state. The snapshot identities and database hashes still
  matched production. The cause was source compatibility: the draft branch
  reader supported encrypted state AAD v9/v8, while the current production
  snapshot was v10.

The branch was brought forward to current `origin/main` (`97538f8d682f`) while
preserving the intentionally isolated one-time handoff seams. Merge commit
`435d4165` is the exact source for all successful evidence below. Workspace
check, rustfmt, and clippy with warnings denied passed. The complete Core and
Hosted Device suites passed, including cross-account handoff and exact rebind
replay. One unchanged `main` client heartbeat timing test failed consistently
with a response-body error; the client compatibility tests, including actual
v9 and v8 encrypted snapshots and the v10 currency-gate round trip, passed.
The migration code does not alter that transport or test.

Release binaries built from `435d4165` have these SHA-256 values:

- loopback `finitechat-server`:
  `22441bce648f58e85f922381becba9d1fc8563c35752a82708221dff618c1909`
- `one_time_room_handoff`:
  `11feef688905e36b3102dfdb369463e9fba74f3f27e4d3df6c4c99076a0de6ed`
- `one_time_agent_rebind`:
  `4f37671f254694e3957839e62422cddf0b05ee9637794337acff580528ecbc2a`

Fresh `rehearsal-v3c` then passed the complete local lifecycle with migration
ID `fiknight-cross-account-room-handoff-2026-09-03-production-v3`: inspect,
join, plan, first apply, exact apply replay, prepared source removal, submitted
source removal, first binding replacement, exact binding replay, and final
verify. Non-secret acceptance evidence:

- canonical Room: `room-c9289c5d35f365f3`
- through sequence: `811`
- Room history events: `809`
- source cached application rows: `465`
- projected chat messages: `433`
- transfer chunks: `4`
- history SHA-256:
  `755d190dacc8a3f79dcf15fb163ac9fe9b8dfdb0dd0a5c1e80e6c9d64d4c8053`
- manifest SHA-256:
  `5cad4c250f7de20f49a359f944006c8278b167cc26d08091e31c7afef6b58336`
- final membership was exactly FiKnight plus the unchanged Agent; both replay
  checks passed.

The successful scratch state was converted into the same four-file install
shape required during the outage. SQLite-native backups for all three
databases returned `PRAGMA integrity_check = ok`, no process retained an open
handle, and every file remained owner-only. Rehearsal install hashes:

- Room database:
  `3cf53a2a9b8f069e42f310a4151cff5b2988b4f11c6099660fdf99da923b4a5b`
- Austin client database:
  `ee8ec95bc3909d5e7759e2de3abc03b2e5547e7c67e89e410332e718740efbc4`
- FiKnight client database:
  `4b029a5209cea32afb22284ad87827f2724616954a8c185fb1edd3274a222b88`
- FiKnight sealed binding:
  `048367253c78e57db6d2a1c9dd3e5ad8c5d6bb688b26246f75c4ecd3831ba45b`

An exact-prefix driver rerun refused to overwrite existing evidence with exit
73. Pre- and post-refusal hashes of all three working databases were identical.
No result above is an install candidate after writers resume; the outage run
must still start from new stopped-writer images and produce new exact hashes.

### Final go/no-go boundary

Complete every item below while Chat remains online, then ask Austin for a
fresh explicit confirmation. Do not stop any shared writer before receiving
that confirmation.

1. Prebuild the loopback Room server and both handoff utilities from the exact
   committed checkout; record their SHA-256 hashes.
2. Set `umask 077`, keep every prior and current cutover root owner-only, and
   require `find CUTOVER_ROOT -perm -007 -print -quit` to return nothing.
3. Seed only from Recovery Snapshot `20260904T022527Z`; match the Room and both
   hosted-client hashes to its sealed manifest. Do not use an earlier attempt
   as an rsync basis or migration input.
4. Use the new migration ID
   `fiknight-cross-account-room-handoff-2026-09-03-production-v3`. Create new
   evidence, removal, and rebind receipts; do not reuse v1/v2 receipts,
   commits, databases, or install images.
5. Rehearse the full Room join/apply/replay/source-removal/rebind/replay/verify
   lifecycle on a copy-on-write clone of the fresh seed using
   `scripts/ops/one-time-room-handoff-local`. The driver refuses
   reused outputs, captures owner-only receipts, checks both idempotent
   replays, verifies final membership/history counts, and stops its loopback
   server before returning. Retain only hashes and non-secret counts in this
   ledger.
6. Immediately before the confirmation request, recheck exact Core and Runtime
   fences, both public NIP-05 routes, available disk, service health, and the
   canonical `finite-status` report.

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
- Runtime artifact: `finite-agent-runtime-2026-09-02.2` (refreshed from the
  paused attempt's `.1` fence on 2026-09-03)
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
