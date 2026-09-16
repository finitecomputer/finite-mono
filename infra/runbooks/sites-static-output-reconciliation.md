# Sites Static Output Reconciliation

One-time offline preparation for the static-only Sites Cutover. Legacy remains
authoritative until cutover. Only agreed published static Sites migrate; retired
apps, documents, and unpublished or missing-source projects are excluded from the
cutover candidate. Preserve their source archive. Do not fabricate repositories
or reconstruct source from rendered content.

## Supported Selections

`scripts/sites-reconcile-static-output-ids.py` accepts one to three exact
selections with previous output IDs `mockup`, `web`, or `static`. Each selection
must identify the project, output row, Site, and previous output ID. The project
must have exactly one output: a published static Site with an active version,
no document or runtime fields, and an available source repository.

- **`mockup` and `web`:** change only the selected registry `output_id` to
  `site`. Preserve the output row primary key, project and Site IDs, names,
  attribution, versions, Shares, and Git references. A committed legacy config
  declaring exactly one static output needs no Git history rewrite; verify that
  its name, Deploy Branch, Deploy Path, and SPA setting match the registry.
- **Mixed projects, including the mixed `static` output:** the tool refuses any
  project with multiple outputs. Manually qualify the selected static Site on an
  isolated copy: explicitly identify the Site to retain, prepare a static-only
  config, and prove a subsequent publish succeeds before admitting it to the
  candidate. Retire the app while preserving its archive. Renaming alone is
  insufficient; do not rewrite the live source branch or remove registry rows
  merely to bypass the tool's refusal.

Repository-directory existence is the tool's only Git check. Separately run
`git fsck --full` on copied repositories and verify their committed config.
Keep customer identifiers, selections, and source/config contents outside git.

## Offline Commands

Use a checkpointed legacy registry from the preserved snapshot, pinned to its
reviewed SHA-256. Do not open the snapshot database directly in SQLite; this
tool copies it before opening SQLite. Any other inspection must use
`scripts/snapshot-sqlite` or a scratch copy. Run before target startup removes
legacy mixed-output evidence.

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

Selection format, using synthetic identifiers:

```json
[
  {
    "project_id": "synthetic-project",
    "project_output_id": "synthetic-output",
    "site_id": "synthetic-site",
    "from_output_id": "mockup"
  }
]
```

Use exactly these fields. Identifiers must contain only letters, digits, `_`, or
`-` and be between 1 and 128 characters. The selection file is limited to 8192
bytes; the registry is limited to 128 MiB.

## Safety and Result

The output directory must not exist, its parent must exist, and it must be
outside both the input registry directory and the repository tree. The tool
refuses unsafe symlink paths, nonempty SQLite sidecars, a mismatched source
digest, unexpected triggers, post-startup schemas, stale or duplicate
selections, multiple-output projects, runtime fields, and missing selected
repositories. It does not overwrite an existing output or modify the source.

Conversion compares every logical row and schema object, allowing only the
selected output-ID changes. Integrity and foreign-key checks must pass before
it writes `registry.db` into the new output directory. All other registry state,
including retired apps and documents, remains in this intermediate copy;
conversion does not perform their exclusion from the cutover candidate.

**The result is not a complete or deployable Sites Recovery Set.** It contains
no bundled Git repositories, blobs, or cookie secret and reports
`cutover_ready: false`. Do not boot it as production or splice it into a live
data directory. Preserve the original Recovery Set and source archive as the
rollback boundary; undo offline preparation by discarding only new scratch
artifacts. Production cutover requires a fresh reconciled snapshot, a Publishing
Write Freeze, complete restore proof, and explicit production authorization.

## Validation and Removal

```sh
scripts/with-dev-env python3 -m unittest scripts.tests.test_sites_reconcile_static_output_ids
```

After cutover resolves these exceptions, delete the tool, its focused tests,
and this runbook, and remove their remaining references.
