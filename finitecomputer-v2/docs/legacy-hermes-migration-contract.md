# Legacy Hermes migration contract

This bridge moves one box1 Hermes bot into a newly created v2 Runtime. It
transfers a sealed, hash-verified copy of the entire durable `/home/node` and
materializes only compatible state into active target paths. No owner decides
the fate of individual files. Old machine identity, credentials, gateway
ownership, and process state remain present in the sealed snapshot but inert.

Use [the runbook](../../infra/runbooks/legacy-hermes-box1-to-v2.md) for the
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

The exporter inventories every directory, regular file, and symlink on the
frozen PVC before it builds the bundle. The fleet policy assigns every entry a
deterministic disposition:

| Disposition | Meaning |
| --- | --- |
| `activate` | copied into a compatible active or review-only target path |
| `converted` | rebuilt through the Hermes session or memory API |
| `preserve` | retained in the sealed source snapshot but not activated |
| `quarantine` | identity, credentials, or executable behavior retained but inactive |
| `rebuild` | generated state retained in the snapshot and recreated for active use |
| `blocked` | unreadable or structurally unsafe state that stops migration |

The inventory contains paths, types, modes, sizes, symlink targets, content
hashes, dispositions, and counts, not file contents. It is mode `0600`, stays
root-only, and is hashed into the migration manifest. Unknown safe paths
default to `preserve`. Rehearsal and cutover require zero `blocked` entries.
They do not require per-user classification.

Sites and external connections have separate inventories because neither can
be reconstructed from file names alone. The Sites inventory comes from the
legacy control plane's authoritative published-endpoint records. For every
endpoint with a local run command, it proves that `run_cwd` is below
`/home/node` and present in the complete source inventory. Reserved or
externally managed endpoints remain listed without inventing a source path.
The migration records these Sites but does not republish them.

The integrations inventory reads the frozen Hermes configuration without
executing it. It records configured environment-variable names, platform
names, and known credential locations, but never their values. Each detected
connection receives one deterministic policy:

- Telegram and Signal are eligible for controlled transfer only after the
  rehearsal proves the target flow.
- Google Workspace and FiniteBrain require fresh target authorization.
- Model-provider credentials are replaced by target-managed configuration.
- Anything else remains preserved and disabled until a supported target setup
  exists.

Unknown integrations do not require an owner to classify their files and do
not disappear. Their state stays in `source-home.tar`; only activation waits.
Both inventories are root-only files hashed into the manifest and receipt.

The operator also rechecks the live pod's read-only root and `/home/node`
mount. Another writable durable mount, a special file, unreadable data, an
escaping symlink in active or otherwise unclassified state, a concurrent
writer, or an integrity mismatch stops the migration because the lossless
contract cannot be proven. External symlinks below known generated or
quarantined roots are retained as inert link metadata in the sealed snapshot
and assigned `rebuild` or `quarantine`; they are never followed or activated.

## Bundle boundary

`finite.legacy-hermes-migration.v2` contains:

| Source | Target | Behavior |
| --- | --- | --- |
| complete `source-home.tar` | `/data/migration/legacy-hermes-v2/preserved/source-home.tar` | Every safe source entry preserved, mode `0600`, never extracted into active state automatically |
| authoritative Sites inventory | migration manifest and receipt | Every published or reserved endpoint recorded; local source paths proven present; nothing republished automatically |
| secret-free integrations inventory | migration manifest and receipt | Every detected connection assigned a transfer, reauthorization, target-managed, or preserve-disabled policy; nothing activated automatically |
| Hermes session JSONL | rebuilt target `state.db` | Imported through target Hermes; live gateway routing, handoff, and activity state reset; structured paths into admitted roots rewritten |
| structured memory SQLite API snapshot | rebuilt target `memory_store.db` | Opened and vector-rebuilt through target Hermes; a non-empty fresh target fails closed |
| `memories/` | active Hermes memories | File collision fails closed |
| `skills/` | migration review-only area | Preserved but never allowed to shadow the Managed Skills Baseline during the canary |
| cron definitions and scripts | migration review-only area | Preserved but never placed in the active scheduler path |
| `workspace/` | `/data/workspace/legacy-box1/workspace` | Preserved without colliding with v2 workspace state |
| `dev/` | `/data/workspace/legacy-box1/dev` | Preserved without colliding with v2 workspace state |
| `uploads/` | `/data/workspace/legacy-box1/uploads` | Preserved without colliding with v2 workspace state |

The manifest proves that every inventory entry appears in `source-home.tar`
with matching type, mode, size, symlink target, and content hash. It also hashes
the complete archive. Active Hermes files are independently manifested and
hashed. Symlinks escaping an active source root are rejected. Escaping links
in known generated or quarantined roots remain inert in the sealed source
snapshot and are recreated only through a supported target flow.
Session working-directory paths under the three active roots are rewritten to
their v2 locations.

The manifest records the source image reference, its containerd manifest
digest, and the running container image ID separately. Operators must prove all
three before export; a mutable tag by itself is not source provenance.

Known credential stores, Hermes config, gateway/platform state, cron execution
state, old `.finite` state, venvs, binaries, logs, caches, and raw databases
exist in `source-home.tar` but receive no active target mapping. The archive and
all user-authored files are sensitive and remain root-only. The structured
memory file used for conversion is a frozen SQLite backup-API snapshot, not a
live database copy. Because box1 is mounted read-only and SQLite WAL readers
may need a writable directory, the exporter first copies the frozen database,
WAL, and shared-memory files into private scratch storage beside the output.
It opens only that scratch set through SQLite, creates the backup-API snapshot,
and removes the scratch set. Cron definitions and Hermes helper scripts are
copied only under `/data/migration/legacy-hermes-v2/review-only/`; the scheduler
never reads that path.

Legacy message fields named `path` or ending in `_path` are rewritten only when
they point into the admitted workspace, dev, or uploads roots. Cache-backed
audio and image paths below the legacy Hermes home remain in the sealed source
snapshot. The manifest and receipt count those cache-media references
separately from other unmapped legacy paths without embedding message content
in either evidence file. The first canary must sample an old session containing
media and prove that its attachment is preserved even though it is not active.

The source Finite Brain Working Tree and identity state are preserved inside
`source-home.tar` but never activated. The new Agent Principal must receive its
own explicit Brain access and Folder Key Grants, then open and sync a fresh
Working Tree. Imported memory documents remain preserved as source evidence; stale absolute
paths must be checked before using imported content in the target environment.

## Full-fidelity mode

The default mode above is deliberately cautious, so a moved bot looks
different to its person: Telegram is disconnected, cron and skills are
review-only and the persona is the target default. The opt-in full-fidelity
mode exists so an operator can move a box1 bot without the person noticing
anything except a few minutes of silence. It is selected with
`--mode full-fidelity` on both `manifest` and `install`; the install fails if
the two disagree. Everything the default mode guarantees still holds: the
sealed `source-home.tar`, the hash-verified manifest and receipt, unchanged
target identity and Chat client hashes, an offline import into a stopped
Runtime, rollback of a failed import, and no secret value in any inventory,
manifest, receipt or error.

`legacy_hermes_fidelity.py` derives a plan from the sealed bundle at manifest
time. The plan holds names and counts only and is bound into the approved
manifest; `verify` re-derives it and refuses a mismatch. The installer reads
every activated file from the payload or the verified `source-home.tar`,
preflights the stopped target before writing anything, and keeps the
target's pre-import `config.yaml` and `SOUL.md` under
`preimport-state/full-fidelity/`.

| Source | Target | Rule |
| --- | --- | --- |
| `.hermes/SOUL.md` | `/data/agent/hermes-home/SOUL.md` | Replaces only an absent or Hermes-default target persona |
| allowlisted `config.yaml` keys | target `config.yaml` | `personalities`, `personality`, `timezone`, `display` (not `display.platforms`), `tts`, `stt`, `voice`, `human_delay`, `session_reset`, `onboarding`, `agent.personality`, `cron.wrap_response` and the `memory` on/off and size limits; secret-shaped nested keys are dropped. The target keeps model credentials and routing, managed skill sources, terminal, gateway, plugins, security, approvals and delegation |
| Telegram token and home channel | `gateway.platforms.telegram` | Exactly the block finite-agentd's `telegram_connect_offer` and `telegram_home_offer` write, plus `extra.preserve_pending_updates: true`. Token sources that disagree, a token without the Bot API shape and a non-numeric home channel fail closed |
| `pairing/` and `platforms/pairing/` Telegram stores | the pairing dir target Hermes reads | Approved and pending entries are merged, `pairing/` wins a conflict as target Hermes merges; rate limits are not carried; a non-empty target store fails closed |
| user-created skills | `/data/agent/hermes-home/skills` | Hermes-bundled skills (from `.bundled_manifest` and its origin hash) and box1 image-overlay skills (`.runtime-managed`) are not activated. A name already in the target's skills, Managed Skills Baseline or bundled set stays review-only and is listed |
| `.hermes/scripts` | `/data/agent/hermes-home/scripts` | Collisions fail closed |
| `cron/jobs.json` | `/data/agent/hermes-home/cron/jobs.json` | Enabled and paused states kept; recurring next runs recomputed from install time in the bot's timezone so nothing fires in a burst; an overdue one-shot, a job delivering to an inactive platform or a job whose script is missing is paused with a reason; per-job inference routes the target does not offer fall back to the target default; stale run claims are cleared; a target with jobs fails closed |
| loose `/home/node` entries | `/data/workspace/legacy-box1/<name>` | Non-hidden top-level files and directories beside `workspace`, `dev` and `uploads`. `AGENTS.md` (box1 rewrites it from its platform render on every boot), the Brain working tree, `node_modules` and `__pycache__` are withheld; collisions fail closed; symlinks are kept only when they stay inside placed or active roots |

`/home/node/<placed entry>` and `~/<placed entry>` become
`/data/workspace/legacy-box1/<placed entry>`; `/home/node/.hermes/` paths for
SOUL.md, memories, skills, scripts and cron become `/data/agent/hermes-home/`.
These rewrites apply to SOUL.md, memories, skill and script text files, cron
prompts, scripts and workdirs, and every string in imported sessions. Other
legacy paths are left unchanged and counted.

Target Hermes drops the Telegram update queue on every cold start, which would
lose messages sent while the bot moves. The Runtime image's
`hermes-telegram-preserve-pending.patch` keeps the queue when
`extra.preserve_pending_updates` is true. The installer refuses a Telegram
activation on a target whose Hermes lacks the patch. One gap remains: if a
second poller makes Telegram return a 409 conflict at start, target Hermes
still drops the queue to recover, so box1 must be frozen before the target
starts.

The receipt's `fidelity.installed` section, and the manifest's
`fidelity.omitted` section, list what stays out: channels the target does not
support (Matrix, Signal and others), every tool key in the legacy `.env` with
whether the target supplies its own (none is copied), Google state (out of
scope), Sites (recorded, not republished), and other inactive gateway state.

The first message after cutover starts a new Hermes session, as after a
session reset; older conversations stay searchable in the imported sessions.

Proof: `scripts/tests/test_legacy_hermes_full_fidelity.py` (unit and contract,
census-shaped fixture in `scripts/tests/legacy_box1_fixture.py`) and
`finitechat/tests/hermes/test_legacy_hermes_full_fidelity.py`, which starts the
real target gateway on an imported home against a fake Telegram Bot API.

## Transaction

1. Create the target normally and record its exact Core/Runner binding and
   Agent Principal.
2. Freeze box1 and keep its PVC plus off-host archive intact.
3. Prove no host process retains a writable file descriptor or writable memory
   map below the frozen PVC, inventory the whole PVC, export sessions from a
   SQLite API snapshot, export authoritative Sites and secret-free integration
   inventories, and seal the complete source snapshot and active inputs.
4. Stop the target through Core.
5. Run the importer once, offline, against the exact target durable root.
6. Restart through Core and verify the same target Agent Principal, Chat round
   trip, imported counts, memories, review-only skills, and workspace files.
7. Keep box1 frozen through the observation window. Decommissioning requires a
   later approval.

The importer builds new session and structured-memory databases beside
the active files using the selected supported target Hermes version,
rebuilds memory vectors through that environment, checkpoints both
WALs, runs SQLite `quick_check`, and swaps only after every source session and
memory fact imports.
The target's pre-import database, complete source snapshot, and receipt remain
under `/data/migration/legacy-hermes-v2/`. A failed import removes files it created
and restores the prior database. The receipt repeats the source-volume,
Sites, and integrations summaries and hashes so post-cutover checks do not
depend on an operator's memory of the bundle.

## Writers and readers

- box1 Hermes is the only source writer until freeze;
- the exporter reads a SQLite backup, never the live database directly;
- the offline importer is the only target writer during installation;
- the v2 Runtime remains stopped while the importer owns `/data`;
- Core and the Runner remain authoritative for stop, restart, artifact,
  placement, and target identity;
- the selected target Hermes version reads the rebuilt session database after restart;
- Finite Chat reads its unchanged target identity and client store.

No Core schema, Runner lifecycle change, or Runtime image publication is
required. Operators hash the reviewed tool archive, mount the extracted modules
read-only into the existing digest-pinned target image, and run them with that
image's Hermes Python environment. Source export also verifies that its
executing Hermes environment is v0.14 before reading state; target installation
verifies v0.20.0 or v0.21.0 before any target mutation. A supported version does
not qualify an image: the exact intended target image still requires the
real-library compatibility test and its durable smoke.

## Compatibility and removal

The first supported pair is source Hermes v0.14.0 to target Hermes v0.20.0.
The tested target versions now include v0.20.0 and v0.21.0. Tests exercise the
legacy export shape against the pinned target Hermes
importer and prove that gateway ownership is not retained. A different source
or target version requires a new tested pair; editing the manifest version in
place is forbidden.

Delete this bridge after every legacy bot has a successful receipt,
its observation window has closed, and each frozen source Recovery Set has
either been retained under policy or decommissioned by separate approval.

## Complete when

- the receipt matches the approved manifest and exact source machine;
- the target identity and Chat client hashes match their pre-import values;
- imported session/message counts match the bundle;
- the manifest binds a complete source-volume inventory and matching
  `source-home.tar` with zero structurally blocked entries;
- every authoritative legacy Site is recorded, and every local Site source
  path is present in the complete snapshot;
- every detected integration has a deterministic migration policy, the
  inventory contains no secret values, and every target integration remains
  inactive during import;
- the target passes Chat, memory, review-only skill, and workspace
  verification;
- in default mode, cron definitions and legacy skills exist only in the
  review-only area during the canary; in full-fidelity mode, the receipt lists
  every activated, review-only and paused item and the target answers the
  owner in the same Telegram chat;
- cache-backed legacy media has an explicit preserved count, other unmapped
  source paths have a separate count, and the sampled media outcome is recorded;
- Brain access is reauthorized and synced without copying legacy identity
  state;
- box1 remains stopped and recoverable; and
- `scripts/finite-status --json` is green after the cutover.
