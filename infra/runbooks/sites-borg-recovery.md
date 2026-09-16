# Sites Backup and Recovery

## Automatic Operation

Once enabled, cron runs `sites-backup` daily at 03:07 UTC. No operator is needed
for each backup. The job checks Borg access, stops only Sites, copies its data
with rsync and a consistent SQLite backup, verifies the snapshot, restarts
Sites, and uploads to rsync.net. Git repositories, blobs, permissions, and the
cookie signing key are included. Failed capture never uploads an old snapshot
as fresh. The job serializes runs and treats Borg warnings as failures.

The serving pause lasts through capture and verification, not remote upload.
SIGINT/SIGTERM and ordinary errors attempt restart; host loss or SIGKILL cannot
run cleanup. There is no automatic prune, compact, or immediate retry. Existing
Latitude and Chat backup jobs are independent.

## One-Time Setup

The checked-in [Fly configuration](../fly/sites/fly.toml) leaves backups
disabled. Repository provisioning, recovery-key custody, external alerting, and
the real remote restore drill are not automated by this code.

1. Provision a dedicated Sites Borg repository on the existing rsync.net account
   using `repokey-blake2`. Retain its exported repokey, passphrase, SSH identity,
   pinned host key, and runtime/mail/service configuration independently of Fly.
   Another repository's exported key cannot recover Sites. The job refuses a
   missing repository or a different encryption mode; it never initializes one.
2. Provision the three file secrets below through the existing secret custody
   process. Never put values in git or command arguments.
3. Deploy a qualified immutable Sites image with the settings below. Allow
   local space for a full snapshot and confirm the daily recovery interval and
   measured serving pause are acceptable.
4. Trigger the job once, check its result, and restore from rsync.net onto an
   isolated empty target before relying on it. Wire an external freshness and
   failure check; the local status command does not send notifications.

Run `scripts/finite-status` before and after an authorized rollout. Add these
environment settings to the Fly deployment when enabling backups:

```text
FINITE_SITES_BACKUP_ENABLED=1
FINITE_SITES_BACKUP_REPOSITORY=<approved dedicated Sites repository>
FINITE_SITES_BACKUP_REMOTE_PATH=borg12
```

Use these Fly file mappings for the provisioned base64-encoded secrets:

```toml
[[files]]
guest_path = "/var/lib/finitecomputer/backups/rsync-net/id_ed25519"
secret_name = "FINITE_SITES_BORG_SSH_KEY"

[[files]]
guest_path = "/var/lib/finitecomputer/backups/rsync-net/known_hosts"
secret_name = "FINITE_SITES_BORG_KNOWN_HOSTS"

[[files]]
guest_path = "/var/lib/finitecomputer/backups/rsync-net/borg-passphrase"
secret_name = "FINITE_SITES_BORG_PASSPHRASE"
```

The image makes the credential directory root-only, tightens files to `0600`,
and strips their environment variables before starting the unprivileged daemon.
Staging and `status.json` default to `/var/backups/finite-sites`, outside serving
data, and are ephemeral across Machine replacement. Set
`FINITE_SITES_BACKUP_ROOT` only to another private directory outside that data.
A dedicated repository path is not access isolation or append-only protection;
verify the SSH credential's restrictions separately.

## Check or Retry a Backup

Inside the backup-enabled Machine as root:

```sh
supervisorctl -c /run/sites-supervisor.conf start sites-backup
supervisorctl -c /run/sites-supervisor.conf status sites-backup
finite-status --sites-backup-state /var/backups/finite-sites/status.json --json
```

`start` launches the one-shot; it does not wait for a completed backup. Check
status after it finishes. Success requires a fresh archive ID and both source
and upload timestamps. `finite-status` defaults to a 36-hour maximum age
(`--sites-backup-max-age SECONDS` overrides it). A missing receipt is unknown,
not healthy. After host loss or forced termination, check Sites health and
restart it through Supervisor or restart the Machine before retrying.

## Restore or Recovery Drill

This is an operator action, not part of the nightly job. Use a matching Sites
image on an isolated recovery Machine/container with an empty volume mounted at
`/var/lib/finite-sites` and no running Sites writer. Run as root to preserve
UID/GID 65532. Keep production routing unchanged.

From independently held credentials, configure Borg 1.x with `BORG_REPO`,
`BORG_REMOTE_PATH=borg12`, `BORG_PASSCOMMAND` reading the private passphrase file,
and `BORG_RSH` using the SSH key with strict pinned-host checking. Use a fresh
private `BORG_BASE_DIR`. Select `ARCHIVE` from `borg list`; do not depend on a
status file surviving the lost Machine. Retain the exported repokey for key
recovery. Never copy a Borg repository while it has writers.

The restore tool requires a nonexistent destination, so restore to scratch
first, then copy verified data into the already-created empty volume:

```sh
set -eu
umask 077
RESTORE_ROOT=$(mktemp -d)
cd "$RESTORE_ROOT"
borg check --verify-data "::$ARCHIVE"
borg extract "::$ARCHIVE"
sites-backup restore --snapshot ./snapshot --target "$RESTORE_ROOT/data"
mountpoint -q /var/lib/finite-sites
test -z "$(ls -A /var/lib/finite-sites)"
rsync -a "$RESTORE_ROOT/data/" /var/lib/finite-sites/
```

Stop on any error; never overwrite a nonempty volume or start an incomplete
restore. The tool verifies inventory, checksums, and SQLite integrity. Use
`scripts/snapshot-sqlite` or a scratch copy for any database inspection. Allow
scratch capacity for the extracted snapshot and verified copy; protect both as
secrets and remove them after qualification.

Restore the service configuration and start Sites on the isolated target.
Verify content, guest/account permissions and revocation, then clone/push/publish
using pre-backup credentials. Keep mail and other outbound effects controlled
during the drill. Do not move production traffic until recovery is verified.

## Local Tests

```sh
scripts/with-dev-env just sites-backup-contract
bash infra/images/sites-smoke.sh "$SITES_IMAGE"
SITES_BACKUP_SMOKE=1 bash infra/images/sites-smoke.sh "$SITES_IMAGE"
```

These cover local backup/restore and Supervisor behavior, not remote credential
access, external notifications, or a real rsync.net-to-Fly restore.
