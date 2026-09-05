# Winston OpenClaw migration evidence

Status: core migration complete; Telegram handoff incomplete. Winston's import
is installed on lat3. Normal lifecycle,
installed-data checks, real Chat verification and the final off-host backup
empty-restore test passed.

This record concerns export
`OpenClaw-Complete-Export-2026-09-04-2026-09-04T23-01-01Z`, separate from the
completed Davy import recorded alongside it in PR #846. Private data, scripts,
detailed identifiers, and reports remain in the operator worktree
`finite-mono-openclaw-230101-import` under ignored `.local-state/openclaw-import/`.
This document grants no production cutover authority.

## Enrollment and target

The owner authorized new Google Workspace and WorkOS production user records,
completed Google's password setup, and supplied a Winston Launch Code. Normal
Core creation launched the Agent on lat3. Dashboard identity and a read-only
Core ownership join agree. The deployed image is 2026-09-02.2; its installed
Hermes code was copied read-only for local rehearsal.

Finite Chat delivered the readiness request and Winston replied
“Chat connection working.” The post-launch platform check reports all 31 active
lat3 and 22 active lat4 Runtimes ready. The independent smoke entry remains red.
Credentials and the supplied Launch Code are excluded from the record.

## Source and recovery evidence

- All 185 checksum-listed files match, totaling 11,503,333,640 bytes.
- All four nested archives traverse successfully, with no unsafe member names
  or duplicate names. Their regular files expand to 20,303,261,655 bytes.
- All 67 received SQLite snapshots pass integrity checks on scratch copies;
  the canonical OpenClaw memory DB also passed the initial inspection.
- All 15 Git bundles verify.
- 653 transcripts contain 25,194 messages and 33,522 events, including 32
  deleted transcripts and 31 compactions. No JSONL parse errors or missing
  parent references were found.
- The full received package restored into an empty local directory: all 186
  regular files matched bytes and file modes, and 185 manifest entries passed.
  Recovery archive SHA-256:
  `280fd4037af6f08426ac44e179a7eed4ec70c3ec3b07dadb9d4dbd7ca3828625`.

The recovery archive is also stored in protected recovery storage on lat3,
outside Winston's Runtime; its remote SHA-256 matches the locally restored
archive. The portable installation archive also passed remote hash verification.
The restore proves the received package, including exact nested archive bytes;
it does not establish consistent
restoration of every archived live Mac database or application.

OpenClaw 2026.2.17 remained live during capture. The capture reports 26 failed
optional browser/application SQLite snapshots, an unreadable advertising cache,
and some unreadable extended attributes. Retain these limitations through
acceptance; raw files and journals alone do not prove consistent recovery.

## History transformation and rehearsal

The initial full-branch expansion repeated shared context excessively. The
replacement splits at branch points and stores each message exactly once.
Every continuation uses Hermes's native `parent_session_id`; per-message source
ancestry and all original JSONL events remain available. A parent segment ends
at the precise shared branch point. Synthetic checks reject duplicate IDs,
missing/forward parents, and self references.

The candidate contains 1,097 history segments, 444 parent links, and exactly
25,194 messages, with no duplicated source events. Its JSON payload is
37,665,597 bytes; the largest session is below the installed 5 MiB limit.
Local one-time import limits allow 1,200 sessions and 64 MiB total, without any
production source/configuration changes.

The installed Hermes API passed:

- Comparison of all 25,194 message rows, including content, timestamps, tool
  links, source metadata, 38 image blocks, 5,934 tool calls, and 1,130 reasoning
  messages; verification of every native parent-session link.
- Historical search for user, assistant, and tool content.
- Existing synthetic session preservation; default-limit and duplicate-payload
  rejection; repeat import skipping existing IDs.
- SQLite integrity and foreign keys; empty restore/reopen; rollback after a
  truncated replacement.
- Import against a scratch copy of Winston's actual live Hermes snapshot,
  preserving his existing session, five messages, and eight other tables.

These were local rehearsals. The local SQLite version differs from the target;
installed-runtime verification and a refreshed stopped baseline were separate
cutover gates, completed below.
The snapshot was obtained through SQLite's online backup into memory and was
opened locally only through a scratch copy. No piped `nerdctl exec -i` was used.

## Portable files, profile, and installation rehearsal

The owner confirmed that this export and its active profile are correct for
Winston. Preparation uses the supplied capture as the history cutoff. The
bounded active profile preserves the target SOUL prefix and loads through the
installed MemoryStore: USER is 1,030 characters and MEMORY is 1,785 characters.
Complete original notes remain available under inactive source filenames.

Portable staging contains 2,509 files totaling 1,284,113,828 bytes: documents,
projects, media, transcripts, source maps and Git bundles. Source instructions,
Git metadata and links use inactive filenames; source files have no executable
bits. Machine applications, dependencies, credential/configuration files and
older full backups remain in protected recovery. Historical notes can contain
sensitive data and are private, even when not active configuration.

The actual Hermes session_search tool read every segment and found historical
user, assistant and tool content. Discovery groups a lineage under its root;
it does not expose every intermediate parent. Exact-ID reads retrieve each
segment, and original event/message maps retain intermediate ancestry.

The full-tree assembly rehearsal preserved all seven protected fixture entries
and checked 2,882 installed entries with no missing, extra or unexpected changes.
It used Winston's actual Hermes snapshot with synthetic surrounding files;
the real stopped full tree received the same checks before restart. A
canonical digest also verified all imported message fields and ancestry,
including finish reasons, against the scratch database.

The private execution sheet names every writer/reader, exact target binding,
existing operation and Chat writer locks, backup/rollback boundary, four changed
Hermes files and the new archived-workspace directory. It prohibits replacing
newer Chat history with a pre-import backup after restart.

## Authorized cutover evidence

The owner explicitly authorized the reviewed one-Runtime stop, import, restart
and final backup scope. Fresh authoritative binding and lifecycle checks passed.
The pre-import full stopped backup restored on lat3 with an identical Runner
state hash and was copied off-host with matching SHA-256:
`f921084a49ff0ad3e7c049f6de36829b5525cc1e40217543ec2b0e6bdce6f3c3`.

The final candidate was rebuilt from that stopped backup under the existing
operation and Chat writer locks. It preserved the original session, five
Hermes messages, and eight other tables. Full-tree verification on lat3 and
the downloaded archive agreed: 4,053 entries checked, 1,178 protected originals
unchanged, and zero missing, extra or unexpected changes. Installed-state
archive SHA-256:
`ed611dad3e2216278dd4358f2bdf61ba26f9277187402a087ea8f5328121f0e3`.

After normal Core restart, installed SQLite 3.53.3 passed integrity and foreign
key checks. The canonical digest matched all 25,194 imported messages and
444 parent links. Native session reads/search and active profile hashes passed;
original Chat rooms/message keys were preserved. A fresh Chat confirmed source
persona/profile continuity and searched imported HRF/Bitcoin sessions.

A second normal stop produced the final consistent backup, including that
verification conversation. The empty restore on lat3 matched the original
stopped state hash. Final archive SHA-256:
`1030b2ef644f81b44bda9540bc065355f002da4c9087119bcea781d4f2df7fde`.
Its off-host copy matched that hash, and the local empty restore passed all
4,055 entries, bytes, modes and links. Both SQLite databases passed integrity
and foreign-key checks on scratch copies. All 25,194 imported messages matched
their canonical content/ancestry digest, and pre-backup Chat rooms and message
keys were preserved. Numeric ownership was verified in the remote restore.

The first local macOS tar extraction interpreted a literal AppleDouble `._`
file as metadata, so it failed the exact-file check. A second empty restore
used literal extraction after validating archive paths, types and symlink
ancestry; it passed. This was an operator-side extraction issue and required
no change to the source archive or production data.

The final Core restart succeeded. Winston answered the final Chat request with
“Winston migration complete.” The lifecycle probe is operable, and the final
canonical platform check reports 31/31 active lat3 and 22/22 active lat4 Runtimes
ready and on target. The separate smoke entry remains red. No shared host
repair, image rollout, or piped guest exec was needed.

At migration completion, source services, old device identities and external
integrations remained inactive on the new Runtime. Source retirement was not
performed. Telegram was subsequently enabled as described below.

### Subsequent Telegram activation

After migration completion, the owner explicitly requested enabling the original
Telegram bot credential without first stopping or invalidating the old connection.
The new Runtime's Telegram connection is enabled through the Connections UI with
restricted pairing policy. Gateway logs report a polling conflict, so competing
consumer access remains unresolved. No token rotation or old-source shutdown was
performed. Intended-user pairing and an end-to-end Telegram test are still
pending. The final migration backup predates this subsequent connection change.
The dashboard's Connected label confirms configuration, not reliable delivery
or a completed handoff. Google Workspace product access remains unconnected;
the new Google login alone does not grant Gmail, Calendar or Drive access.

## Resume from this checkpoint

- Keep the working Finite Chat migration and existing recovery copies intact.
- Have the intended Telegram user message the original bot. Approve its pairing
  request only after matching the intended account; no account is approved yet.
- Coordinate stopping the competing Telegram consumer when the owner is ready.
  Do not rotate the token or stop the old source under the current instruction.
- Verify inbound/outbound Telegram messages on the migrated Winston after the
  competing consumer is stopped; a reply alone cannot identify the answering
  Runtime while both consumers are active.
- Preserve updated connection/pairing state in the next recovery checkpoint.
  The completed migration backup must not be represented as covering it.

Private evidence anchors: `source-restore-report.json`,
`remote-recovery-verification.json`, `stopped-candidate-report.json`,
`installed-stopped-verification.json`, `running-import-check.json`,
`final-restore-report.json`, `final-lifecycle.json`, `cutover-final-status.json`,
`telegram-readiness.json`, `telegram-live-check.json`, and `migration-plan.md`.
These are retained locally, not published with the PR.
