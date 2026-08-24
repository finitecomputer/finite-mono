# Legacy Hermes migration contract

This bridge moves one box1 Hermes bot into a newly created v2 Runtime. It
preserves user-authored Hermes history, memories, skills, and files. It does
not copy the box1 machine identity, known credential stores, gateway ownership,
or process state.

Use [the runbook](../../infra/runbooks/legacy-hermes-box1-to-lat3.md) for the
operator procedure. `scripts/legacy-hermes-migration` is the executable
contract.

## Identity boundary

The target is a normal Core-created Project and Runtime. Core, the Runner, and
Finite Chat create its Agent Principal and Chat client store before import.
The importer requires and hashes both files:

```text
/data/agent/identity/identity.json
/data/agent/client.sqlite3
```

Those hashes must be unchanged after import. The old box1 `.finite` state is
never accepted as target identity. The migrated bot therefore has a new Finite
Chat identity and new Finite Chat history; legacy Hermes conversation history
appears in Hermes, not as fabricated Finite Chat messages.

The install command requires the operator-approved manifest, identity, and
Chat client SHA-256 values. A valid bundle aimed at the wrong fresh target, or
a rewritten manifest presented after approval, fails before import.

## Source storage boundary

The box1 source is a k3s pod, not a full persistent VM. Its root filesystem is
read-only. The `home` PVC is mounted at `/home/node`; `/tmp` and `/run` are
ephemeral. This matters because Austin could have written durable files
anywhere below `/home/node`, not only in the directories Hermes normally uses.

The exporter inventories every non-directory entry on the frozen PVC before it
builds the bundle. Each entry must have one reviewed disposition:

| Disposition | Meaning |
| --- | --- |
| `bundle` | copied into the sealed bundle |
| `converted` | rebuilt through the Hermes session or memory API |
| `archive-only` | retained in the source Recovery Set, not made live on lat3 |
| `rebuild` | generated output that is deliberately recreated later |
| `unresolved` | migration stops until the PR and runbook assign a disposition |

The inventory contains paths, types, sizes, and counts, not file contents. It
is mode `0600`, stays root-only, and is hashed into the migration manifest.
Rehearsal and cutover both require zero unresolved entries. The operator also
rechecks the live pod's read-only root and `/home/node` mount. If the deployed
pod has another writable durable mount, the migration stops because this
contract does not cover it.

## Bundle boundary

`finite.legacy-hermes-migration.v1` admits only:

| Source | Target | Behavior |
| --- | --- | --- |
| Hermes session JSONL | rebuilt target `state.db` | Imported through target Hermes; live gateway routing, handoff, and activity state reset; structured paths into admitted roots rewritten |
| structured memory SQLite API snapshot | rebuilt target `memory_store.db` | Opened and vector-rebuilt through target Hermes; a non-empty fresh target fails closed |
| `memories/` | active Hermes memories | File collision fails closed |
| `skills/` | migration review-only area | Preserved but never allowed to shadow the Managed Skills Baseline during the canary |
| cron definitions and scripts | migration review-only area | Preserved but never placed in the active scheduler path |
| `workspace/` | `/data/workspace/legacy-box1/workspace` | Preserved without colliding with v2 workspace state |
| `dev/` | `/data/workspace/legacy-box1/dev` | Preserved without colliding with v2 workspace state |
| `uploads/` | `/data/workspace/legacy-box1/uploads` | Preserved without colliding with v2 workspace state |

Every admitted file, mode, and contained workspace symlink is manifested and
hashed. The source-volume inventory accounts for everything outside this
allow-list.
Symlinks in active Hermes state or symlinks escaping their admitted source root
are rejected. Session working-directory paths under the three box1 roots are
rewritten to their v2 locations.

The manifest records the source image reference, its containerd manifest
digest, and the running container image ID separately. Operators must prove all
three before export; a mutable tag by itself is not source provenance.

Known credential stores, Hermes config, gateway/platform state, cron execution
state, old `.finite` state, venvs, binaries, logs, caches, and raw
session/auxiliary SQLite files remain in the frozen source recovery archive.
Admitted user-authored files and scripts are still sensitive and may themselves
contain secrets. The structured memory file in the bundle is a frozen SQLite
backup-API snapshot, not a live database copy. Cron definitions and their
Hermes helper scripts are copied only under
`/data/migration/legacy-hermes-v1/review-only/`; the scheduler never reads that
path. Reauthorization and per-job cron recreation are separate, explicit work.

Legacy message fields named `path` or ending in `_path` are rewritten only when
they point into the admitted workspace, dev, or uploads roots. Cache-backed
audio and image paths below the legacy Hermes home remain in the frozen
Recovery Set. The manifest and receipt count those cache-media references
separately from other unmapped legacy paths without embedding message content
in either evidence file. The first canary must sample an old session containing
media and record that its attachment is intentionally archive-only unless a
separately reviewed media bridge is added.

The source Finite Brain Working Tree and its identity state are not bundle
inputs. The new Agent Principal must receive its own Email Access Delegation
and Folder Key Grants, then open and sync a fresh Working Tree. Imported memory
documents are not rewritten as prose; operators use the
[post-cutover repair brief](legacy-hermes-post-cutover-repair.md) to find and
repair stale source paths.

## Transaction

1. Create the target normally and record its exact Core/Runner binding and
   Agent Principal.
2. Freeze box1 and keep its PVC plus off-host archive intact.
3. Prove no host process retains a writable file descriptor or writable memory
   map below the frozen PVC, inventory the whole PVC, export sessions from a
   SQLite API snapshot, and seal the allow-listed bundle plus the inventory.
4. Stop the target through Core.
5. Run the importer once, offline, against the exact target durable root.
6. Restart through Core and verify the same target Agent Principal, Chat round
   trip, imported counts, memories, review-only skills, and workspace files.
7. Keep box1 frozen through the observation window. Decommissioning requires a
   later approval.

The importer builds new v0.20 session and structured-memory databases beside
the active files, rebuilds memory vectors through v0.20, checkpoints both
WALs, runs SQLite `quick_check`, and swaps only after every source session and
memory fact imports.
The target's pre-import database and a receipt remain under
`/data/migration/legacy-hermes-v1/`. A failed import removes files it created
and restores the prior database. The receipt repeats the source-volume
inventory summary and hash so post-cutover checks do not depend on an
operator's memory of the bundle.

## Writers and readers

- box1 Hermes is the only source writer until freeze;
- the exporter reads a SQLite backup, never the live database directly;
- the offline importer is the only target writer during installation;
- the v2 Runtime remains stopped while the importer owns `/data`;
- Core and the Runner remain authoritative for stop, restart, artifact,
  placement, and target identity;
- Hermes v0.20 reads the rebuilt session database after restart;
- Finite Chat reads its unchanged target identity and client store.

No Core schema, Runner lifecycle change, or Runtime image publication is
required. Operators hash the reviewed tool archive, mount the extracted modules
read-only into the existing digest-pinned target image, and run them with that
image's Hermes v0.20 Python environment. Source export also verifies that its
executing Hermes environment is v0.14 before reading state; target installation
verifies v0.20 before any target mutation.

## Compatibility and removal

The first supported pair is source Hermes v0.14.0 to target Hermes v0.20.0.
Tests exercise the legacy export shape against the pinned target Hermes
importer and prove that gateway ownership is not retained. A different source
or target version requires a new tested pair; editing the manifest version in
place is forbidden.

Delete this bridge after every admitted legacy bot has a successful receipt,
its observation window has closed, and each frozen source Recovery Set has
either been retained under policy or decommissioned by separate approval.

## Complete when

- the receipt matches the approved manifest and exact source machine;
- the target identity and Chat client hashes match their pre-import values;
- imported session/message counts match the bundle;
- the manifest binds a complete source-volume inventory with zero unresolved
  entries;
- the target passes Chat, memory, review-only skill, and workspace
  verification;
- cron definitions exist only in the review-only area during the canary;
- legacy skills exist only in the review-only area during the canary;
- cache-backed legacy media has an explicit archive-only count, other unmapped
  source paths have a separate count, and the sampled media outcome is recorded;
- Brain access is reauthorized and synced without copying legacy identity
  state;
- box1 remains stopped and recoverable; and
- `scripts/finite-status --json` is green after the cutover.
