# Sites backup implementation status

The selected design is [revision backups with shared-state checkpoints](adr/0029-revision-backups-with-shared-state-checkpoints.md).
Tracking: [FIN-54](https://linear.app/finitecomputer/issue/FIN-54).

## Local foundation

The operator can capture and restore a **local** Recovery Point:

```sh
finitesitesd backup capture --data /path/to/sites --repository /path/to/backup
finitesitesd backup restore --repository /path/to/backup --point POINT_SHA256 --target /path/to/new-target
```

Capture prints a JSON receipt with the point ID, capture timestamp, and count
of newly stored objects. Both parent directories must exist. The repository
must be outside the source tree. Restore requires a destination that does not
exist; it never overwrites a live registry. Inspect protected snapshot databases
only through `scripts/snapshot-sqlite` or disposable scratch copies.

The source registry is opened read-only and is not initialized or migrated.
SQLite snapshots preserve shared metadata while published blobs are stored
once by content hash. Git bundles preserve all captured refs and recorded
historical object IDs, including source-only projects and non-deploy branches.
Historical commits use `refs/finite-recovery/` in the restored repository so
later Git maintenance cannot discard them. Live branch names are unchanged.
Each operator invocation still walks the full catalog and rebuilds Git bundles;
deduplication reduces stored bytes, not capture work. Per-revision execution is
not implemented yet.

Capture checks for pending Git reconciliation and for observed Git/catalog
changes across the checkpoint. A busy or inconsistent generation fails and can
be retried; earlier completed points and immutable objects remain available.
This is optimistic validation, not a global serving freeze. It is not yet a
qualified production capture scheduler, and sustained writes can prevent a
complete checkpoint.

The current event log deduplicates Git transitions and has no authoritative
per-ref cursor. Capture therefore accepts only an unambiguous acyclic chain of
recorded transitions for each ref, with the observed ref at its terminal SHA.
Unrecorded tips, cycles (including some branch delete/recreate and rollback
histories), and branching histories fail closed. Do not repair or rewrite user
history to make capture succeed. Writer-coordinated checkpointing is still a
production gate; this conservative operator command is not its replacement.
Missing repositories are refused, not synthesized as empty. An initialized,
empty bare repository can be captured and restored.

Completion manifests are content-addressed and written last. Restore validates
object hashes, registry integrity, required blob coverage, project inventory,
Git bundles, registry-required historical objects, and refs in private scratch
space before creating the destination.
It does not start the daemon or send email. Boot a test restore with dev mail
and production network access disabled. Never start a failed restore.

On Linux and macOS, the verified tree's files and directories are synced before
an atomic no-replace rename, followed by syncing the destination parent.
Concurrent restores cannot overwrite one another. Interruption before
publication leaves the destination absent; scratch directories left by a killed
process are private but require operator cleanup. A parent-sync error after
rename is reported as a failed restore even though the complete target exists.
Keep that target offline and investigate; do not automatically delete or retry
over it. Other operating systems are not supported for atomic publication.

Current ceilings are 100,000 manifest inventory items (files, projects, refs,
and retained Git objects combined), 64 MiB serialized manifest, and 1 GiB per
object. Objects are currently read into bounded memory; allow more than 1 GiB
of memory plus Git working space. Each SQLite capture and each Git command has
a 60-second deadline, and Git stderr is capped at 1 MiB. These are safety
ceilings, not production sizing or throughput qualification.
The restored tree is also bounded to one million entries and depth 128;
capture applies a conservative expansion budget before completing a point.

The local repository is **not encrypted**. It contains private source, registry
credentials/auth state, and the cookie signing secret. Directories are private
and object files are created with private permissions. Do not commit it, attach
it to a ticket, or upload it through an unqualified transport. Keep it on a
protected filesystem with enough space for the source checkpoint, bundles, and
retained objects. There is no automated deletion; unfinished capture objects
remain reusable but consume disk.

## Local verification

Run `scripts/with-dev-env cargo test -p finitesitesd --locked` for operator
capture/restore, crash/retry, concurrent no-replace publication, bounded Git
process failures, and application HTTP tests. The recovery application test
publishes a first version, captures/restores it, then uses the original editor
credential to clone and publish Version 2 on the restored server. The original
owner changes sharing through the signed API; the restored bytes become
public while the source remains private at Version 1. Another recovery test
checks that an original viewer cookie still works and revocation still applies.

The workspace gate is `scripts/with-dev-env just test`, which supplies the
isolated Postgres environment required by Core tests. Local macOS success is
not Linux, AWS, Fly, or production recovery qualification.

## Still required before production

- Revision-triggered durable work tracking and background execution, including
  source-only Git updates, retry/backoff, missed-work reconciliation, and an
  authoritative writer-coordinated Git checkpoint boundary.
- S3 transport with independently recoverable credentials, client-side
  encryption/key custody, exact-version receipts, and tested retention.
- Shared-state scheduling and dependency-aware retention. Do not configure an
  object-age lifecycle that deletes blobs needed by newer recovery points.
- Bounded source-host resource use, concurrency/crash qualification, freshness
  and failure reporting through `scripts/finite-status`, and an independent
  alert path.
- S3 contract tests and an actual remote restore onto an empty Fly volume,
  including the original publisher's clone/push/publish flow and preserved
  permissions. Local tests are not off-host recovery proof.
- Independently recoverable runtime configuration, mail/service credentials,
  encryption keys, and image access. These are not all files in the Sites data
  directory; the local capture includes the cookie key, not Fly secret values.

No production service, AWS resource, CLI fleet pin, DNS record, or migration
state is changed by implementing these commands. Option A / Latitude remains
intact. A local repository on the serving volume is not an independent backup.
