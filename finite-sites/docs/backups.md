# Sites backups

Decision: [snapshot and Borg backups](adr/0031-borg-on-existing-rsync-net.md).
Operations: [Sites Borg recovery](../../infra/runbooks/sites-borg-recovery.md).

## Implementation

`infra/scripts/sites-backup` stops Sites, copies its full data tree with rsync,
backs up SQLite, verifies the snapshot, resumes Sites, then archives it with
native Borg over SSH. Git repositories are copied intact. The operator script
is independent of publishing APIs and workflows.

## Operator commands

Only after all Sites writers have been stopped:

```sh
infra/scripts/sites-backup snapshot --offline --data DATA_DIR --target NEW_SNAPSHOT_DIR
infra/scripts/sites-backup restore --snapshot SNAPSHOT_DIR --target NEW_DATA_DIR
```

Targets must not exist and must be outside the source tree. Capture uses a
read-only source database connection and SQLite's backup API. It never migrates
the source. Snapshots contain `finite-sites/`, a format marker, source capture
timestamp, `manifest.sha256`, and the existing NUL-delimited symlink inventory
convention. Restore verifies the complete inventory and SQLite integrity through
`scripts/snapshot-sqlite`, copies to private scratch and installs without
overwriting an existing target. No daemon starts during restore.

Whole snapshots include source-only repositories, non-deploy refs, historical
Git objects, blobs, sharing/auth state and the cookie key. Runtime configuration,
mail/service credentials and the Borg key/passphrase still need independent
custody. The local snapshot is plaintext: keep its parent private.

The job command controls the stop/copy/start sequence:

```sh
sites-backup run --config /run/sites-backup.json
finite-status --sites-backup-state /var/backups/finite-sites/status.json --json
```

The job validates encrypted Borg access before pausing Sites, serializes runs
with a file lock, resumes a previously running service in its cleanup path,
and archives only the snapshot from this run. Failed capture never re-uploads
an old snapshot as fresh. Local staging is removed after the attempt; successful
archives remain in Borg. No automatic prune/compact or remote initialization.

## Service Lifecycle

The Fly image uses stock Supervisor and cron only when
`FINITE_SITES_BACKUP_ENABLED=1`. Default serving remains the existing direct
non-root daemon. Opt-in requires provisioned root-only credentials and a
repository; the daemon cannot access the supervisor socket or backup keys.
Cron starts a supervised one-shot daily at 03:07 UTC. Confirm the cadence and
measured pause before enabling it. Image shutdown stops cron, then the backup
worker, then Sites.

Normal errors and SIGINT/SIGTERM attempt service recovery and report failure.
SIGKILL or host loss cannot run cleanup: restore service through the supervisor
or restart the Machine and inspect the failed/incomplete run before retrying.
The next image boot starts Sites normally. This is a maintenance-window backup,
not a zero-downtime design.

Before enabling backups, follow the operating runbook to provision credentials,
qualify the image, wire external freshness alerts, and restore from rsync.net
onto an empty Fly volume. Existing host and Chat backup jobs are independent.

## Verification

```sh
scripts/with-dev-env just sites-backup-contract
bash infra/images/sites-smoke.sh IMAGE
SITES_BACKUP_SMOKE=1 bash infra/images/sites-smoke.sh IMAGE
```

The contract tests cover the actual script and native encrypted Borg, corruption,
wrong credentials, overlapping jobs, no-overwrite restore, capture/upload/restart
failures, supervisor isolation, and separate snapshot/upload freshness. The image
smoke covers the real daemon, restart/replacement, a Borg restore into an empty
Docker volume, original-credential clone/push and preserved sharing. A separate
root Linux test exercises real cron scheduling and ordered supervisor shutdown.
CI runs both the script contracts and exact-image checks. Local tests do not
prove rsync.net access or a completed production Fly restore.
