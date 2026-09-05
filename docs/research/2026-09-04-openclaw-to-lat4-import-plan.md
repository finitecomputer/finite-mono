# One-time OpenClaw migration to Hermes: execution record

Status: scoped import completed on lat3, 2026-09-05 UTC (September 4 locally).
Investigation baseline: `529c1c63`. The original filename retains the initially
planned lat4 destination. The owner authorized lat3, the September 4 export
cutoff, the source persona/profile, and inactive external integrations.
Telegram was requested afterward and remains a separate blocked handoff.
This document records the procedure and execution evidence; it grants no new
production authorization.

## Scope and outcome

The received `~/Downloads/OpenClaw-Complete-Export-2026-09-04` was imported from
the operator Mac into one fresh, Core-owned Hermes Agent Runtime. The selected
destination was lat3, accepted after normal Core creation placed the Agent there.
The result retains the complete export, makes historical conversations
searchable in Hermes, and provides the portable workspace, projects, documents,
media, persona, and memory.
The new Agent keeps its own Finite identity and genuine Chat history.

Run custom conversion and database preparation locally. Use existing Core
lifecycle commands, SSH, and file-transfer tools for server operations. Temporary
scripts, private evidence, staging, and execution reports stay in the worktree's
ignored `.local-state/openclaw-import/` directory. Only this sanitized record enters Git.
There is no migration framework, reusable installer, new API, runtime image
change, or other product implementation in this work.

## Established facts

These are the September 4 inspection results. Later rehearsal and production
verification are recorded under Execution evidence below; pending statements in
this baseline table describe the prerequisites at inspection time.

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
Use the owner-accepted fresh Runtime, or establish placement through existing
creation/relocation procedures. Normal creation has no exact-host selector;
[cold relocation][relocation] requires admission coordination. Do not force
placement with SQL, host-directory creation, or a temporary fleet drain.

Run `scripts/finite-status` before server changes and after each rollout. The
September 4 report at approximately `22:34:49Z` showed 22 ready lat4 Runtimes and
about 1.56 TiB free on `/data`, but stale inference launch overrides. The overall
report was not green, and existing readiness did not prove fresh admission or
backup coverage. Recheck effective application configuration, not just raw
environment values. The installed launcher already normalizes historical
inference overrides; those values did not require a lat3 repair. Use the
[existing reconciliation procedure][runner-route] only for an evidenced,
separately authorized configuration change with its rollback boundary.
Report any prerequisite needing platform code as separate
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

## Execution evidence

All results below were captured during execution on September 4–5, 2026.
They are observations at that time, not a claim about current fleet status.
Raw records remain private in `.local-state/openclaw-import/`; the filenames
below identify evidence without committing transcripts, profiles, credentials,
account details, or runtime identity keys.

### Destination and accepted scope

Core enrollment, sponsored admission, launch, and a real Finite Chat response
passed before import. The exact owner/Project/Runtime/provider-handle binding
was recorded privately and rechecked at cutover. Core selected lat3; the owner
accepted it. No relocation, shared Runner drain, or identity rewrite occurred.

The deployed artifact was `finite-agent-runtime-2026-09-02.2`, schema
`runtime-state-v1`, image digest
`sha256:c7f1ec6a8d4454d8e7b40fbeadfcfd789eea27304a511800aae0716d49e2328e`.
Installed Hermes matched revision `3c27eb6234bf91b8ceee9e9071591b31e9b148cb`.
The target used SQLite 3.53.3; local preparation used the repo-pinned Python
with SQLite 3.50.4. Candidates were prepared without concurrent writers,
checkpointed and closed; the installed target subsequently passed live checks.

Raw inference environment values initially led to an incorrect repair
recommendation. Inspection of the installed launcher and effective Hermes
configuration confirmed that the existing normalization fix was working.
No inference repair was required on lat3. An earlier authorized lat4 cleanup
removed stale overrides, but was not established as necessary for this image.

### Data conversion and preservation

| Check | Observed result | Private evidence |
| --- | --- | --- |
| Source accounting | 31 transcripts, 4,390 events and all 4,066 original messages retained. Branch expansion produced 33 sessions and 5,754 rows, including 1,688 repeated context rows. | `history-conversion-report.json` |
| Exact installed import implementation | Compared all 5,754 rows, including content, roles, timestamps, tool links, reasoning and ancestry. Verified 78 native image blocks after branch expansion. | `history-rehearsal-report.json` |
| Failure and recovery rehearsal | Oversize/malformed rejection, duplicate behavior, preserved synthetic baseline, SQLite integrity/FK checks, empty restore and interrupted-install recovery passed. | `history-rehearsal-report.json` |
| Actual stopped baseline | Preserved the existing Hermes session and its three messages, plus eight other tables, when preparing the final candidate from a scratch copy of the stopped backup. | `stopped-candidate-report.json` |
| Portable payload | 1,379 files / 278,425,043 bytes staged and hash-verified. Original instruction files and Git metadata retained as source material; executable bits removed. Sensitive and nonportable material remained in protected recovery storage. | `portable-staging-report.json`, `portable-file-map.json` |
| Approved active profile | Existing managed SOUL prefix retained. Bounded user memory and general memory loaded successfully and matched candidate hashes in the live Runtime. Full originals and notes retained. | `profile-option-report.json`, `running-import-check.json` |
| Scoped installation | Full stopped-tree comparison verified 3,015 installed entries and 1,184 protected original entries with no unexpected changes. | `installed-stopped-verification.json` |
| Production readers | Live SQLite integrity/FK checks passed; all 33 imported sessions / 4,066 unique source messages remained present. User, assistant and tool history search passed. | `running-import-check.json` |
| Finite Chat continuity | Original Room/message IDs and Agent Principal preserved. Real Chat searched imported history; a fresh Chat read the approved active memory. Final Chat answered after the last restart. | `chat-binding-before.json`, `fresh-chat-memory-verification.json`, `final-chat-verification.json` |

Installation used normal Core Stop and Restart operations. The existing
per-Runtime operation lock and Chat writer lease fenced the stopped backup
and installation. Only the approved Hermes history/profile paths and the new
portable workspace tree changed. The locks were released before restart.

### Recovery evidence

| Recovery copy | Verification |
| --- | --- |
| Complete source archive | Empty local restore verified 232,036 manifest-listed files and 284,365 metadata entries, including 16,461 symlinks. A separate compressed copy on lat3 decompressed to the original SHA-256. |
| Full pre-import target archive | Copies on lat3 and off-host. Empty remote restore matched the stopped Runtime state manifest; local file/link verification passed. |
| Installed, still-stopped target archive | Full copy on lat3 and off-host, used for the scoped-install comparison before the first restart. |
| Final post-import archive | Normal Core stop succeeded. State manifests before/after capture and after empty remote restore matched. Off-host empty restore verified 3,017 entries; scratch-copy SQLite integrity/FK checks and preservation of all pre-backup Chat message IDs passed. |

Archive SHA-256 values:

```text
source-recovery.tar
54dfd57da5cb6f0d800983f81d9bb3a49b8dc0efc7c2c807812f7b3f84131730
target-preimport.tar
97e3ae950adfde7717f5eb6326c97d763fa840da3a5ef367ef6f8d606ce003c2
target-final.tar
12016755f5dfb52dcc67fe13bb8ba703aa60fb199d0bbb6e206f31f45a8aa626
```

Evidence: `source-restore-report.json`, `final-backup-result.txt`, and
`final-restore-report.json`. Source Mac quarantine/provenance attributes remain
in the archive but were not asserted as restored by GNU tar. Target ownership
was checked in the remote restore; local verification covered bytes, modes and
links. These are one-time recovery proofs, not a new scheduled-backup guarantee.
The final archive includes the import verification conversations made before
its capture; the last post-restart Chat check is later than that snapshot.
Source and recovery copies remain retained. No source decommissioning occurred.

### Verification-command incident and resolution

After the first restart, Chat continued to work but `nerdctl` status/exec reads
timed out. The deployed lifecycle probe reported `orphaned_task`. Live process
and Kata state inspection showed that the same VM was still running; that
probe result was not evidence of absent compute.

The abandoned read-only verifier, launched with `nerdctl exec -i ... python3 -`,
remained on the host after its local SSH client was canceled. It held the write
end of the exact exec input FIFO. TERM did not end that client. After checking
its identity and FIFO again, ending only the verifier with KILL immediately
restored status reads; the VM PID was unchanged. No shim, VM, shared containerd
service, or host was restarted to clear the blockage.

Kata's [`CloseIO` implementation][kata-close-io] waits for input completion while holding the service mutex,
consistent with the observed behavior. An internal stack dump was unavailable,
so this record does not claim a complete internal deadlock trace. The same
verification passed using `python3 -c` without interactive stdin. The full
lifecycle probe then reported `operable`; normal Core stop/restart and the
final recovery test also passed.

Separate maintenance finding: [containerd 2.3.0 task listing][containerd-task-list] skips entries whose
state reads fail. The Runner probe incorrectly treats a missing list entry as
proven absence. That diagnostic defect was recorded, not patched or deployed
by this migration. Evidence: `exec-pipes-private.txt`, `recovered-lifecycle-probe.json`,
`running-import-check.json`, and `completed-import-lifecycle.json`.

### Completion and remaining handoff

The scoped import is complete. Final canonical status reported all 30 active
lat3 Runtimes and all 22 active lat4 Runtimes health-ready, with no active
controls. The unrelated Smoke Studio fleet exception remained red; global green
status was not claimed. Davy's final lifecycle probe was operable, and Chat
responded after restart. Evidence: `completed-import-status.json` and
`final-chat-verification.json`.

External integrations were intentionally inactive at import acceptance. The
owner subsequently requested Telegram. The original bot token was recovered
privately and validated with Telegram; the new Runtime remains disconnected.
The source Mac is online on the tailnet, but ordinary SSH authentication failed,
the Tailscale browser console offered SSH setup rather than an active terminal,
and Screen Sharing failed. No source consumer was stopped or proved inactive.
No bot token was entered into the target, rotated, or committed, and no second
consumer was enabled. Handoff remains blocked on source access or confirmation
that the source Telegram consumer is stopped. Pairing and bidirectional message
verification remain required afterward. Google Workspace activation and source
retirement are also separate work, not completed migration steps.

[openclaw-migration]: https://github.com/NousResearch/hermes-agent/blob/3c27eb6234bf91b8ceee9e9071591b31e9b148cb/optional-skills/migration/openclaw-migration/scripts/openclaw_to_hermes.py
[hermes-import]: https://github.com/NousResearch/hermes-agent/blob/3c27eb6234bf91b8ceee9e9071591b31e9b148cb/hermes_state_portability.py
[legacy-contract]: ../../finitecomputer-v2/docs/legacy-hermes-migration-contract.md
[relocation]: ../../infra/runbooks/runtime-cold-relocation.md
[runner-route]: ../../infra/runbooks/runner-finite-private-route.md
[kata-close-io]: https://github.com/kata-containers/kata-containers/blob/3.29.0/src/runtime/pkg/containerd-shim-v2/service.go
[containerd-task-list]: https://github.com/containerd/containerd/blob/v2.3.0/plugins/services/tasks/local.go
