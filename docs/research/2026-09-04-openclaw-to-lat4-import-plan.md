# One-time OpenClaw migration to Hermes on lat4

Status: planning record, 2026-09-04. Investigation baseline: `529c1c63`.
Execution has not started. The draft PR records this plan; it does not authorize
production changes.

## Scope and outcome

Migrate `~/Downloads/OpenClaw-Complete-Export-2026-09-04` from the operator Mac
into one fresh, Core-owned Hermes Agent Runtime on lat4. The result must retain
the complete export, make historical conversations searchable in Hermes, and
provide the portable workspace, projects, documents, media, persona, and memory.
The new Agent keeps its own Finite identity and genuine Chat history.

Run custom conversion and database preparation locally. Use existing Core
lifecycle commands, SSH, and file-transfer tools for server operations. Temporary
scripts, private evidence, staging, and execution reports stay in the worktree's
ignored `.local-state/openclaw-import/` directory. Only this plan enters Git.
There is no migration framework, reusable installer, new API, runtime image
change, or other product implementation in this work.

## Established facts

These are inspection results from September 4, not a completed migration rehearsal.

| Evidence | Consequence |
| --- | --- |
| The package contains about 12.54 GB of regular files. All 232,036 manifest-listed files passed SHA-256 verification; the manifest itself was hashed separately. | Reuse the sealed inventory. Preserve the entire package independently of the live Runtime. |
| Ten supplemental SQLite snapshots and the canonical OpenClaw memory DB passed integrity checks on scratch copies; five Git bundles verified. | Received bytes are readable. An independent restore is still required. Never open original snapshot DBs directly. |
| OpenClaw `2026.3.8` remained running during capture. The canonical backup timestamp is `2026-09-04T21:53:58.970Z`; supplemental files have separate capture times. | The owner must accept this cutoff or provide a consistent final refresh. Checksums do not establish a frozen source. |
| There are 31 JSONL transcripts and 4,066 messages, including 24 deleted and four reset transcript files. The session index has only five entries. | Read every transcript, including retained deleted/reset history. The index is not the source inventory. |
| History contains two branch points, two compaction events, 11 tool calls without results, and 57 image content blocks. The largest transcript is about 14.6 MB. | Preserve ancestry and unfinished actions; account for images. Size this local import's limits explicitly. |
| Persona and memory exceed the upstream migration tool's default active-memory limits. The package also contains credentials, wallet state, Mac software, and 311 absolute symlinks. | Preserve complete originals; select active memory and portable behavior deliberately. Preserve nonportable and sensitive material inactive. |

The inspected [upstream OpenClaw migration tool][openclaw-migration] does not
convert transcripts, and its presets also change configuration. The existing
[Finite legacy bridge][legacy-contract] accepts legacy Hermes sources. Neither
is a complete importer for this export.

The only required custom transformation is a temporary transcript converter:

```text
OpenClaw JSONL events
  -> Hermes session/message records + source-to-output map
  -> existing SessionDB.import_sessions API on a local scratch database
  -> verified database and staged files for the stopped target
```

The inspected [Hermes import API][hermes-import] clears live channel/process
ownership and defaults to a 5 MiB per-session limit. The repository pins revision
`3c27eb6234bf91b8ceee9e9071591b31e9b148cb`. Verify the actual target image digest,
installed Hermes version, and schema before relying on this API.

## What becomes active

| Source material | Destination and treatment |
| --- | --- |
| Original export, manifests, omissions, and metadata | Independent protected recovery copy, restored and checked before cutover. Keep originals through acceptance. |
| All transcripts and media | Hermes historical sessions plus accessible original/readable archives. Preserve deleted/reset labels, branch and compaction ancestry, timestamps, tool IDs/results, and supported content. Map every source event; retain unsupported fields in the archive. Never replay actions. |
| Workspace, external projects, documents, and Git state | Portable trees under `/data/workspace/openclaw` and named project directories. Preserve relative layout and uncommitted work. Remap only explicit operational paths; keep Mac dependencies and escaping links inactive. |
| Persona, profile, memory, daily notes, instructions, and skills | Complete originals in the workspace; reviewed Hermes persona and bounded active memory with remaining notes retrievable. Activate compatible instructions/skills after rehearsal, preserving Finite's managed baseline. |
| Credentials, wallet/other-agent state, Mac services, gateway state, queues, and old device identity | Preserve protected and inactive. Keep target-managed identity, inference configuration, and Chat authoritative. Restore required integrations through supported target flows. |

Imported conversations are searchable Hermes history and archives. They are
not inserted as past messages into the new Finite Chat Room. Shared branch
context may produce more than 31 native sessions; the report must account for
duplication and prove no source message was dropped.

## Execution sequence

### 1. Prepare and rehearse locally

1. Reuse the verified inventory, retain an independent recovery copy, and restore
   it into an empty directory. Compare hashes and required metadata. Preserve
   the capture/omission record; regenerate inventory if the source is refreshed.
2. Write the one-time converter and stage the portable files. Keep a simple
   mapping of source events and files to outputs, retained archives, and any
   unresolved entries. Reject malformed input, ambiguous mappings, collisions,
   and paths escaping the intended destination.
3. Check synthetic examples for the observed history edges, oversized input,
   and memory overflow. Rehearse the entire export with the intended target's
   Hermes version/schema using the pinned local environment. Include preexisting
   target sessions to prove they survive import. Reconcile any version difference
   discovered when the actual target is selected and repeat affected checks.
4. Verify message accounting, history search, representative images, complete
   memory retention, and file hashes. Measure staging space and duration; prove
   scratch backup/restore after an interrupted install. No live credentials or
   source services run during rehearsal.

Output: prepared payload, local verification report, measured space/outage
estimate, and a one-Agent execution sheet. An inability to reproduce the target
Hermes implementation locally remains a rehearsal gap to resolve before cutover.

### 2. Prove the exact target

Record the owner, Agent, Runtime, host, provider handle, durable-state root,
image digest, and Hermes version privately from authoritative Core/Runner state.
Use an identified fresh lat4 Runtime, or establish placement through existing
creation/relocation procedures. Normal creation has no exact-host selector;
[cold relocation][relocation] requires admission coordination. Do not force
placement with SQL, host-directory creation, or a temporary fleet drain.

Run `scripts/finite-status` before server changes and after each rollout. The
September 4 report at approximately `22:34:49Z` showed 22 ready lat4 Runtimes and
about 1.56 TiB free on `/data`, but stale inference launch overrides. The overall
report was not green, and existing readiness did not prove fresh admission or
backup coverage. Recheck this evidence. If the stale overrides remain, use the
[existing reconciliation procedure][runner-route] with scoped authorization and
its rollback boundary. Report any prerequisite needing platform code as separate
work rather than adding it to this migration.

Once authorized, create/prepare the exact target and prove enrollment/admission,
launch, Agent identity, canonical Room binding, and a real Chat/inference round
trip. The execution sheet must identify the selected target, tested versions,
expected file changes, backup location, measured outage estimate, and rollback.
Obtain authorization for that concrete cutover before stopping the Runtime.

### 3. Back up, install, and restart

1. Stop only the selected Runtime through Core. Verify its stop receipt, absence
   of writers, and lifecycle fencing against restart throughout installation.
2. Back up all target `/data`, including identity, Chat, baseline Hermes history,
   configuration, and SQLite sidecars. Restore to empty scratch storage and
   verify it. Record the stopped target binding and protected-file hashes.
3. Work from a local scratch copy of the stopped Hermes database and required
   sidecars. Import converted history through the rehearsed API; checkpoint,
   close, and validate the candidate. Preserve baseline sessions and reject ID
   collisions. Original snapshots remain untouched.
4. Transfer and verify the prepared database and staged files. Confirm the
   target remains stopped with the same binding and protected hashes. Install
   only the approved paths, handling SQLite sidecars consistently so stale WAL
   state cannot be applied to the replacement database.
5. Verify unchanged Finite identity, Chat, and runtime configuration while
   stopped. Record installed hashes/counts and restart through Core. An uncertain
   or interrupted install keeps the target stopped for backup restoration.

### 4. Verify completion

- Same Agent identity and canonical Room, preserved initial Chat history, and
  successful new bidirectional Chat/inference, including after a restart.
- All 31 transcripts and 4,066 source messages accounted for, unless a refreshed
  source changes the sealed totals. Search works for old user/assistant/tool
  content; deleted/reset archives, branches, images, and the largest session
  remain accessible to the owner.
- Persona and memory behave as agreed, complete notes remain retrievable, and
  portable files/Git state match the prepared inventory.
- Required integrations work through supported flows. Telegram handoff, if
  included, fences the old consumer before enabling the new one; no replay or
  duplicate poller. Website publication, wallet activation, and machine identity
  cloning are not automatic effects of copying files.
- `scripts/finite-status` rerun with target readiness demonstrated and unrelated
  exceptions recorded explicitly. Verify a post-import off-host backup by
  restoring it, covering the Agent's new writes.

Keep source and pre-import backups through acceptance. Source decommissioning/deletion
and optional additional observation are separate from completing the import.

## State ownership and recovery

| State | Writers and readers |
| --- | --- |
| Source export | Local converter reads the sealed copy. Source applications may have written during capture; the accepted cutoff determines continuity. |
| Lifecycle and placement | Core/Runner remain authoritative. Their bindings, receipts, and status establish the exact stopped target. |
| Finite identity and Chat | Normal bootstrap and Chat components write/read this state. Migration tools preserve it byte-for-byte while stopped; after restart verify logical history continuity. |
| Hermes history, memory, and workspace | Local tools prepare candidates; existing transfer/install tools write approved paths while stopped. The selected Hermes implementation and Agent tools read/write after restart. |
| External connections | The source owns consumption until an explicit handoff; the target owns it afterward. Reversal must preserve messages received after handoff. |

Before restart, recover a failed install by restoring the verified full target
backup while keeping it stopped. Restore and reverify before retrying; a custom
transaction engine or blind rerun is unnecessary.

After restart, stop and preserve newly written state before any recovery action.
**Never restore an old Chat database over new messages.** Revert imported
Hermes/workspace data only where preservation of new writes is proved; a wider
restore requires reconciliation. Returning an integration to the source also
requires fencing the target consumer first.

## Inputs still needed before execution

- Exact Finite owner/Agent and a proven fresh lat4 target binding.
- Acceptance of the supplied snapshot cutoff, or a final source refresh.
- Required external integrations and their handoff scope.
- Completed local rehearsal, source/target restore proofs, current target
  readiness, and authorization for the concrete cutover.

This draft records the procedure. Conversion, rehearsal, target creation,
production repair, and import remain unperformed.

[openclaw-migration]: https://github.com/NousResearch/hermes-agent/blob/3c27eb6234bf91b8ceee9e9071591b31e9b148cb/optional-skills/migration/openclaw-migration/scripts/openclaw_to_hermes.py
[hermes-import]: https://github.com/NousResearch/hermes-agent/blob/3c27eb6234bf91b8ceee9e9071591b31e9b148cb/hermes_state_portability.py
[legacy-contract]: ../../finitecomputer-v2/docs/legacy-hermes-migration-contract.md
[relocation]: ../../infra/runbooks/runtime-cold-relocation.md
[runner-route]: ../../infra/runbooks/runner-finite-private-route.md
