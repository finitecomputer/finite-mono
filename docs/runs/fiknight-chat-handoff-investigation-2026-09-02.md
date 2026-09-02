# FiKnight cross-account Chat handoff investigation — 2026-09-02

Status: **blocked; do not retry the account-transfer stage or `Finish chat setup`.**

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
Austin canonical Room:   present=1 messages=462
FiKnight canonical Room: present=0 messages=0
FiKnight other Rooms:    room-37139f43b1268158
FAIL: FiKnight does not have the existing canonical Room and its message history
```

The authoritative existing FiKnight Room is
`room-c9289c5d35f365f3`. It contains 462 messages: 428 from the unchanged Agent
Principal and 34 from Austin's historical Chat Principal. Austin's hosted store
has three other Agent Rooms, so the migration scope is this one Room only—not
Austin's full hosted store.

The two hosted stores prove this is not dashboard filtering:

| Owner | Hosted store | Chat account | Rooms | Messages |
| --- | --- | --- | ---: | ---: |
| Austin | `fba78e0c…844b2` | `a4c4ae91…68362` | 4 | 4,449 |
| FiKnight | `ab4b8016…bdfcc` | `a976b00a…d978` | 1 | 0 |

Hosted storage is selected by SHA-256 of the WorkOS user subject. Therefore the
new WorkOS subject necessarily opened a different encrypted User Key and
`client.sqlite3`; a Core `project_room_memberships` row does not transfer MLS
membership, key material, or history.

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

## Required clean solution

A future attempt needs a purpose-built, one-time cross-account canonical Room
handoff. It may remain local and unmerged, but it is protocol work—not SQL-only
operator bookkeeping. It must be proven on synthetic and restored production
state before another production mutation.

The minimum safe design is:

1. Reproduce Austin having four Rooms and FiKnight having the failed empty
   binding/Room in a fixture restored from the Recovery Set.
2. Define an exact, replay-safe cleanup or retirement operation for FiKnight's
   failed sealed binding and empty Room. Do not blindly delete the server Room.
3. Have FiKnight publish fresh KeyPackages.
4. Have Austin's admitted Device add FiKnight's distinct Chat Principal/Device
   to only `room-c9289c5d35f365f3` through a real MLS Commit.
5. Freeze the accepted membership sequence as the immutable history cutoff.
6. Export and authenticate complete history for only that Room through a new
   cross-account member-authorized history-share variant.
7. Atomically import the exact ordered history, profiles, metadata, and digest
   receipt into FiKnight's store.
8. Seal the target Project binding to the existing canonical Room through an
   explicit repair intent, not the normal creation bootstrap.
9. Remove Austin's Chat Principal from that Room and rekey only after FiKnight
   proves the full 462-message baseline, authorship, new send/reply, restart,
   and the unchanged Agent Principal.
10. Only then rerun the Core ownership/name transfer, same-artifact Runtime
    replacement, and NIP-05 binding.

The fixture must also prove exact replay, interruption recovery at every durable
boundary, rollback before and after membership changes, and isolation from
Austin's other three Rooms.

## Decision

PR #813 remains draft and unmerged. The existing Core SQL is retained as a
review artifact but now refuses to run without an explicit
`fiknight_cross_account_handoff_ready` psql variable. That variable must not be
used until the cross-account handoff above exists, its synthetic/restored-state
tests pass, and a fresh production mutation is explicitly authorized.
