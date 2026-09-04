# FiKnight cross-account Chat handoff investigation — 2026-09-02

Status: **local handoff rehearsal passed; the authorized production cutover was
rolled back and is now paused because the current method causes platform-wide
Chat downtime. Do not reuse an attempt artifact or retry `Finish chat setup`.**

This investigation explains why transferring the Core Project did not transfer
the existing FiKnight conversation to the new `fiknight@finite.vip` login. It
uses the post-attempt coordinated Recovery Snapshot at
`/data/recovery-snapshots/hosted-web-chat/20260902T144154Z`. All SQLite evidence
was queried from manifested snapshot copies through `scripts/snapshot-sqlite`.
No live Chat database was opened directly.

Snapshot evidence:

- manifest SHA-256:
  `3247ac406c4a026a22d4d3bbbf9e762561b46b0de28eae90b74eeaf2a645ad40`
- Core dump SHA-256:
  `bc0d32c74ed3a0442b90aba3751de9a20c400b5492a1da6ba1fecae114efcc61`
- Austin hosted client database size: 16,048,128 bytes
- FiKnight hosted client database size: 176,128 bytes

## Production result and rollback state

The staged Core transaction succeeded, and the FiKnight dashboard showed the
existing Project. Invoking `Finish chat setup` then created a new, empty Chat
Room for FiKnight instead of opening Austin's existing Room. The Core transfer
was immediately rolled back before Runtime replacement or NIP-05 binding.

The authoritative Core and Runtime state is restored:

- Project owner, organization, display name, and NIP-05 are Austin's original
  values.
- Project, Runtime, machine, artifact, durable state, and Agent Principal are
  unchanged.
- Austin's Core Project membership is active; FiKnight's staged membership is
  archived.
- `fiknight@finite.vip` remains unbound in Finite Identity.
- The live Runtime `/contact` still reports
  `npub1r83u6s59v5956l5gd6my6vjqk9x0rkjef78ntchs494m5y6tq4dqychqrv`.

The hosted Chat bootstrap did leave durable residue outside Core: FiKnight now
has an independent hosted Chat account/store, a sealed Project binding, and an
empty Room. That residue is not removed by the Core rollback and must be an
explicit input to any future repair. It must not be treated as harmless audit
state.

## Minimal reproduction

Run the red-capable check against a local copy of the post-attempt snapshot:

```sh
nix shell nixpkgs#sqlite nixpkgs#coreutils -c \
  scripts/ops/fiknight-chat-history-repro SNAPSHOT_COPY
```

Observed twice:

```text
Austin canonical Room:   present=1 cached_application_rows=462
FiKnight canonical Room: present=0 cached_application_rows=0
FiKnight other Rooms:    room-37139f43b1268158
FAIL: FiKnight does not have the existing canonical Room and its message history
```

The authoritative existing FiKnight Room is
`room-c9289c5d35f365f3`. Its legacy `client_app_messages` cache contains 462
application rows: 428 from the unchanged Agent Principal and 34 from Austin's
historical Chat Principal. The table name predates typed projections and those
rows include 31 conversation/control entries; the room projects 431 actual
user-visible chat messages. Austin's hosted store has three other Agent Rooms,
so the migration scope is this one Room only—not Austin's full hosted store.

The two hosted stores prove this is not dashboard filtering:

| Owner | Hosted store | Chat account | Rooms | Cached application rows |
| --- | --- | --- | ---: | ---: |
| Austin | `fba78e0c…844b2` | `a4c4ae91…68362` | 4 | 4,449 |
| FiKnight | `ab4b8016…bdfcc` | `a976b00a…d978` | 1 | 0 |

Hosted storage is selected by SHA-256 of the WorkOS user subject. Therefore the
new WorkOS subject necessarily opened a different encrypted User Key and
`client.sqlite3`; a Core `project_room_memberships` row does not transfer MLS
membership, key material, or history.

## Local production-shaped rehearsal

The one-time implementation in this draft PR was exercised only from the
operator workstation against private scratch clones of the manifested
post-attempt snapshot. The Room server bound only to `127.0.0.1`; both hosted
stores carried an exact migration marker; decrypted history remained in process
memory and was never serialized. No binary was deployed and no production
service, database, membership, binding, or account record was changed.

The rehearsal passed this sequence:

1. FiKnight published a fresh KeyPackage and Austin added only FiKnight's exact
   Chat Device to `room-c9289c5d35f365f3` through MLS.
2. The joined freeze point was sequence 809. It contained 807 authenticated
   retained application events, including FiKnight's normal welcome-activation
   entry. The source cache contained 463 application rows: the snapshot's 462
   rows plus that activation entry. The typed projection remained 431 visible
   chat messages.
3. The room-only history was staged in four in-memory chunks and atomically
   committed to FiKnight's encrypted store. A fresh-process replay returned
   `exact_replay: true` with the same digests:

   - history SHA-256:
     `cfcdf0a064ca2e52c6c256d618f233f6c787e39dc649c71af2af260306feaee1`
   - manifest SHA-256:
     `081a36e03dc4631976811c0636ecabb67d452fd2e3ba06929738489c5c8d1ce6`

4. A separately recorded MLS Commit removed only Austin at sequence 810. The
   exact remaining accounts were FiKnight and the unchanged Agent.
5. The sealed hosted binding was authenticated and replaced atomically so the
   historical Room is canonical. Exact replay succeeded without a second
   replacement.
6. A fresh-process verifier projected all 431 chat messages, selected the
   historical Room, paired the unchanged Agent to it, and found no copy of
   Austin's other three Rooms.

FiKnight's empty `room-37139f43b1268158` is preserved as an associated audit
Room rather than deleted speculatively. It is not canonical or selected and has
no historical messages. R2D2 should use this handoff before first-time hosted
chat setup so it never creates the equivalent empty residue.

The temporary artifacts are intentionally ordinary source files in this draft
branch so their behavior is reviewable:

- `finitechat-core/src/one_time_room_handoff.rs` contains the room-scoped
  export/import receipts and two-phase source removal.
- `finitechat-core/examples/one_time_room_handoff.rs` is the marked-scratch,
  loopback-only `inspect`, `join`, `plan`, `apply`, `prepare-remove`,
  `submit-remove`, and `verify` operator sequence.
- `finitechat-hosted-device/examples/one_time_agent_rebind.rs` performs the
  exact sealed-binding compare-and-replace without adding an HTTP route.

The examples load account keys only from the copied hosted identity files. No
key is accepted on the command line, printed, written to an evidence ledger, or
committed to this branch. The handoff bundle deliberately has neither
`Serialize` nor `Debug`, keeping decrypted retained history in memory.

## Root cause

Four facts compose the failure:

1. Core Project membership is dashboard authorization/navigation metadata. It
   is not membership in a cryptographic FiniteChat Room.
2. `finitechat-hosted-device` derives a separate storage root from each WorkOS
   subject and validates Project bindings against that store's Chat account.
3. `Finish chat setup` is a first-creation recovery action. ADR 0012 requires it
   to create/resume only its journaled Room and explicitly forbids scanning for
   or adopting a retained Room. Transferring the durable creation request made
   the new bootstrap valid, so it correctly created a different Room.
4. The existing complete-history mechanism is same-account device enrollment.
   `link_device` assigns the source account to the target Device, while the
   receiver rejects a bootstrap whose sender/source account differs from the
   target owner. FiKnight is a new Chat Principal, not another Austin Device.

Protocol v1 also prohibits the Room server from replaying ordinary
pre-membership history. Retained history must come from encrypted backup or an
explicit member-to-member history share. The observed empty history is therefore
the current safety contract, not an intermittent sync failure.

Focused existing tests passed:

- `runtime_device_link_fanout_enrolls_same_account_device_idempotently`
- `new_agent_binding_stays_unchanged_across_duplicate_selection_and_restart`

They validate the two relevant invariants: history fanout is same-account, and a
sealed Agent binding is immutable rather than reconciled to another Room.

## Rejected shortcuts

- **Core rows only:** reproduced failure; no MLS membership or history moves.
- **Copy Austin's hosted store:** exposes Austin's other three Agent Rooms and
  transfers Austin's User Key, not FiKnight's identity.
- **Copy selected SQLite rows:** ciphertext, MLS group state, receipts, and the
  sealed binding are account-key-bound; row copying cannot establish valid
  membership.
- **Point FiKnight's binding at the old Room:** binding validation fails because
  FiKnight lacks that Room's MLS state and exact membership.
- **Retry `Finish chat setup`:** resumes the already sealed empty-Room binding;
  ordinary bootstrap is intentionally not a migration path.
- **Use NIP-AB/device link:** the implementation and receipts require the source
  and target Devices to share one Chat account.
- **Replay server history:** forbidden for pre-membership entries.
- **Rewrite messages into a new Room:** breaks cryptographic provenance,
  authorship, ordering, and recovery integrity.

## Implemented local solution and remaining cutover work

A purpose-built, one-time cross-account canonical Room handoff now exists on
this draft branch. It is intentionally absent from UniFFI and HTTP product
surfaces, accepts only loopback Room-server URLs and marked scratch roots, and
must remain unmerged. It has been proven on synthetic state and the restored
production-shaped snapshot. It was later used to build transformed files from
a stopped production copy during the authorized cutover. The first install was
automatically rolled back before the Core transaction started, and the second
attempt stopped after its fresh ciphertext copy. No production handoff
completed. See the
[paused cutover checkpoint](fiknight-account-transfer-2026-09-02.md#paused-production-cutover-checkpoint).

The implemented phases are:

1. Reproduce Austin having four Rooms and FiKnight having the failed empty
   binding/Room in a fixture restored from the Recovery Set.
2. Preserve FiKnight's failed empty Room as explicit associated audit state; do
   not blindly delete or rewrite it.
3. Have FiKnight publish fresh KeyPackages.
4. Have Austin's admitted Device add FiKnight's distinct Chat Principal/Device
   to only `room-c9289c5d35f365f3` through a real MLS Commit.
5. Freeze the accepted membership sequence as the immutable history cutoff.
6. Export and authenticate complete history for only that Room through the
   one-time cross-account in-memory handoff.
7. Atomically import the exact ordered history, profiles, metadata, and digest
   receipt into FiKnight's store.
8. Seal the target Project binding to the existing canonical Room through an
   explicit repair intent, not the normal creation bootstrap.
9. Record and submit a separate MLS Commit removing Austin only after FiKnight
   proves the 807-event digest, 431-message projection, restart, and unchanged
   Agent Principal.
10. Only then rerun the Core ownership/name transfer, same-artifact Runtime
    replacement, and NIP-05 binding.

Synthetic tests also prove a post-handoff target message, Agent receipt, source
send rejection, exact replay, restart, and isolation from Austin's other three
Rooms. A real Agent reply remains a production acceptance gate because the
production Agent Runtime is intentionally absent from the local copied server.

## Verification record

The migration-specific local gates pass:

- the one-time Core handoff test, including exact replay, source removal, and
  isolation from Austin's unrelated Rooms;
- the hosted-device sealed-binding repair test, including restart and replay;
- the existing same-account history-fanout and immutable-binding regression
  tests;
- the synthetic Core stage/replay/rollback/finalize test;
- targeted Clippy with warnings denied, Rust formatting, shell syntax,
  ShellCheck, and `git diff --check`.

The broader `finitechat-client --lib` run passed 36 tests and failed the
existing timing-sensitive
`sync_stream_survives_healthy_heartbeats_longer_than_the_timeout` SSE test. An
isolated rerun failed at the same SSE response-body boundary. The handoff does
not change SSE or heartbeat code; this result is recorded rather than treated
as migration-path proof or silently omitted.

## Decision

PR #813 remains draft and must never be merged. The existing Core SQL is
retained as a review artifact and refuses to run without an explicit
`fiknight_cross_account_handoff_ready` psql variable. The paused attempt's
copies, ledgers, prepared commits, migration IDs, and install images are stale
evidence and must not be reused. A future attempt requires a newly approved
availability design, fresh coordinated backup and state capture, updated
Runtime fence and rehearsal, and new explicit production authorization.
FiKnight is the first migration; R2D2 must not begin until FiKnight completes
its live acceptance checks.
