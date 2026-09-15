# Static Sites Migration Exceptions

Scope: the September 14, 2026 source receipt, inspected on September 15 using
disposable copies. Apps and documents remain authoritative on legacy. This is
one-time offline preparation, not a migration framework or cutover approval.
No production mutations, routing changes, deploys or remote requests were made.
Customer identifiers, source/config contents and selections remain outside git.

## Findings and Disposition

| Exception | Evidence | Safe handling |
| --- | --- | --- |
| `mockup`, `web` | Each project has exactly one published static output, no runtime fields and no canonical ID collision. Both copied repositories pass `git fsck --full`. Each committed config selects one static output and matches the registry's name, branch, path and SPA setting. | On a new offline registry copy, change only the explicitly selected `output_id` to `site`. Keep the output row primary key, project/site IDs, attribution, names, versions, shares and Git references. Both conversions were rehearsed successfully. |
| `static`, the mixed project | One static output and one app output share a project and deploy branch, with different deploy paths. The branch's committed config declares both outputs. The repository passes `git fsck --full`. | Keep gated for explicit owner/operator disposition of Git publishing authority and config. Renaming alone does not fix the rejected two-output config. Preserve the app and its config on legacy; do not split ownership, rewrite its shared branch, or invent a new project. |
| Four missing repositories | All four projects have one unpublished static output, one active name claim, no active version, zero historical versions, zero publishes and zero Git events. Their source repository directories are absent. | Preserve records, ownership and name claims. Keep source recovery/owner disposition gated. Do not create empty repos, reconstruct rendered content, delete claims or claim successful source recovery. |

The three source `sites.kind='static'` rows removed by startup are separate from
the four missing repositories: all three are unpublished, associated with **app**
outputs, and have no active or historical versions. Their removal from the
disposable static candidate does not authorize deletion from legacy.

## Why a Rename Is Necessary

These code references describe the investigated `b5280f16` source revision;
the offline tool does not depend on that branch or its store implementation.

- `finite-sites/crates/finitesites-store/src/lib.rs`,
  `migrate_project_output_document_shape` (line 1517): copies surviving `output_id` values
  verbatim when rebuilding the table.
- The same file, `init_project_with_owner` (line 2636): accepts only canonical `site`, looks
  up `(project_id, output_id)`, and otherwise attempts allocation. A noncanonical
  existing row is missed and its already-active name causes a conflict.
- `finite-sites/crates/finitesitesd/src/git.rs`, `reconcile_ref_event` (line 433): requires
  the stored output ID to equal `site` before publishing the pushed version.
- `finite-sites/crates/finitesites-proto/src/project_config.rs`,
  `legacy_static_site_from_outputs` (line 220): normalizes one legacy static config, but
  rejects more than one output. Therefore `mockup`/`web` need no Git history
  rewrite; the mixed project's existing config remains incompatible.

## Patched Startup Rehearsal

The separately built patched daemon was run on a fresh full scratch copy with
loopback-only listening, sandbox-denied outbound networking, the development
mailer and Git auto-reconciliation disabled. Only local unauthenticated health
was requested; the daemon was stopped afterward. This was independent of the
output-ID conversion, so that conversion cannot conceal attribution loss.

Pinned binary SHA-256:
`268c3760d2777cb6950cc20d3f5b8f37bd0b7ad77687f581a7e1091e2b6088fd`.
The binary was supplied from the sibling migration-fixes worktree; its store
changes are not included in this branch. After that worktree's build output
changed, the rehearsal was repeated on another fresh copy using a privately
preserved executable. Its hash matched before and after the repeat. The counts
below are from that definitive pinned-binary receipt.

- 224 source static rows; 221 retained; the three removed rows are classified above.
- All 119 non-null retained publisher attributes preserved; zero changed or lost.
- Zero changes to retained static ownership, status, visibility, active version
  or originating publisher attribution.
- All project rows, collaborators, Git credentials, authorized keys and email
  principals preserved; retained email and native shares preserved.
- Healthy startup, successful SQLite integrity check and zero foreign-key violations.
- All 22,300 protected source files and 55 symlink records matched the source
  manifest before and after rehearsal. SQLite opened only scratch copies.

This proves snapshot preservation for the tested binary, not the separate
authorization behavior/restart regressions owned by the parent change. It also
does not prove post-cutover publishing, mixed-version behavior or source freshness.

## Bounded Offline Command

`scripts/sites-reconcile-static-output-ids.py` uses Python's standard SQLite
library to inspect a copied database and produce at most three explicit renames.
Python fits the existing private receipt scripts and root offline-command tests;
no dependency or runtime service is added. Delete this tool and document after
this cutover's exceptions have been resolved and their receipts retained privately.

The test boundary is the offline command's arguments, JSON result, exit status
and output artifact. Synthetic tests do not call private store methods.

```sh
scripts/with-dev-env python3 scripts/sites-reconcile-static-output-ids.py inspect \
  --registry /PRIVATE/SOURCE/registry.db \
  --expect-sha256 REVIEWED_SOURCE_SHA256 \
  --repositories /PRIVATE/SOURCE/git/projects

scripts/with-dev-env python3 scripts/sites-reconcile-static-output-ids.py convert \
  --registry /PRIVATE/SOURCE/registry.db \
  --expect-sha256 REVIEWED_SOURCE_SHA256 \
  --repositories /PRIVATE/SOURCE/git/projects \
  --selection /PRIVATE/explicit-selection.json \
  --output-dir /PRIVATE/NEW-SCRATCH-DIRECTORY
```

Selection format, using synthetic identifiers only:

```json
[{"project_id":"synthetic-project","project_output_id":"synthetic-output",
  "site_id":"synthetic-site","from_output_id":"mockup"}]
```

The source must be a hash-pinned, checkpointed legacy registry. The tool rejects
nonempty SQLite sidecars, symlink input paths, already-created output directories,
output directories inside either the source registry or repository tree,
post-startup schemas that have erased mixed-output evidence, unexpected triggers,
stale/duplicate selections, multiple-output projects, runtime fields and missing
selected repositories. It copies before opening SQLite. It compares every logical
row and schema object against the input, allowing only selected output-ID changes;
integrity and foreign keys must pass before it publishes the scratch artifact.
Reports contain only counts and fixed messages. Actual Git config compatibility
and integrity were checked separately; directory existence alone is not Git proof.

The real offline conversion changed exactly two IDs, preserving all other logical
registry state, including legacy apps/documents. One noncanonical mixed output and
four unavailable repositories remain. The result explicitly reports
`cutover_ready: false`.

**The output is a legacy registry copy, not a complete or deployable recovery
set.** It has no bundled Git repositories, blobs or cookie secret. Do not boot it
as production or splice it into a live data directory. Keep the original
manifested recovery set and archive as the rollback boundary. Roll back this
exercise by discarding only its new scratch artifacts. Any eventual production
repair needs a fresh reconciled snapshot, publishing-write boundary, complete
restore proof and explicit production authorization.

Validation:

```sh
scripts/with-dev-env python3 -m unittest scripts.tests.test_sites_reconcile_static_output_ids
```

Eight synthetic offline-command tests cover conversion, source preservation,
mixed-project refusal, missing source, replay, stale batches, wrong hash,
nonempty WAL, symlink refusal and separate-repository-tree protection. The positive
test independently reads a scratch copy of the emitted database and compares
literal IDs, ownership, provenance, shares and Git-event references. The parent owns startup and authorization
behavior tests; no Rust/store or backup modules are changed here.
