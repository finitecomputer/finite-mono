# Sites backup adaptation and recovery

Tracking: [FIN-54](https://linear.app/finitecomputer/issue/FIN-54).
Decision: [Sites ADR 0031](../../finite-sites/docs/adr/0031-borg-on-existing-rsync-net.md).

## Reuse the existing system

Legacy Sites is already included in the shared
[snapshot and Borg job](../nixos/modules/backups.nix), enabled on
[LAT2](../nixos/hosts/finite-lat-2/default.nix). The snapshot stops Sites, copies
its data, and creates a consistent SQLite registry backup. Borg archives the
snapshot to rsync.net. The current shared snapshot is deploy/manual-triggered;
its daily Borg upload does not prove that source state was captured that day.

FIN-54 adapts this coverage to the independent Sites host. It does not introduce
a new backup platform, per-revision queues, revision hooks, a dependency graph,
a metadata scheduler, or a bespoke retry service.

## Fly adaptation

Option B / Fly is active. Option A / Latitude remains available with the same
backup approach, not a second implementation. Reuse the Sites snapshot and
restore logic where applicable, adapting paths and lifecycle control to Fly:

1. Serialize backup runs with the ordinary job lock. Record whether Sites was
   running, stop only Sites, and ensure all Sites writers have stopped.
2. Copy its data directory into private staging and use SQLite's backup API
   for the registry, excluding raw registry/WAL/SHM copies. Include Git
   repositories, blobs, permissions/auth state, and the cookie signing key.
3. Verify and finalize the snapshot. Resume the previously running Sites
   service before uploading. Capture failure must also attempt restart and
   report any restart failure.
4. Archive that completed snapshot using the existing native Borg conventions:
   encrypted `repokey-blake2`, SSH with a pinned host key, compression/dedup,
   and nonzero exit codes (including warnings) treated as failure.
5. Record the source snapshot timestamp, archive ID, and successful upload
   time. Report failures and stale snapshots through `scripts/finite-status`
   and existing alerting.

Schedule these steps as one job so each upload follows a fresh snapshot.
If capture fails, fail the run; do not silently upload an older snapshot and
call it fresh. Use normal scheduler/operator retries, not a database-backed
work queue. Daily is the starting cadence proposal matching existing archival,
not a promise of an agreed data-loss window. Confirm cadence and measure the
Sites pause before enabling it. No zero-downtime claim is made.

The implementation is `infra/scripts/sites-backup`. The opt-in image uses stock
Supervisor and cron for Sites-only lifecycle control; it never stops Chat,
Core, Identity or runners. Existing LAT2 jobs and archives are unchanged.

## Enable only after provisioning

The checked-in Fly digest still points to the earlier image. Build and qualify
a new immutable image before enabling backups. The default entrypoint remains
the direct non-root daemon. Supervision requires:

```text
FINITE_SITES_BACKUP_ENABLED=1
FINITE_SITES_BACKUP_REPOSITORY=<approved dedicated Sites repository>
FINITE_SITES_BACKUP_REMOTE_PATH=borg12
```

Optional `FINITE_SITES_BACKUP_ROOT` defaults to `/var/backups/finite-sites`,
outside the mounted serving data. This private local staging and status receipt
are ephemeral across Machine replacement; Borg is the durable off-host copy.
Provision enough local disk for one full snapshot. Repository initialization
and credential provisioning are separate authorized operator actions. The job
refuses an absent repository or encryption other than `repokey-blake2`.

Cron starts the supervised job daily at 03:07 UTC. For a manual run inside the
Machine, use the same supervised job rather than launching a detached process:

```sh
supervisorctl -c /run/sites-supervisor.conf start sites-backup
supervisorctl -c /run/sites-supervisor.conf status
finite-status --sites-backup-state /var/backups/finite-sites/status.json --json
```

Starting the job is not proof of completion. The receipt must report success,
a new archive ID, and fresh source and upload timestamps. `finite-status`
checks both timestamps (36-hour default, overridable with
`--sites-backup-max-age SECONDS`). Wire an external check before promotion;
logs and a local receipt alone do not notify anyone. A missing receipt after
Machine replacement is unknown, not healthy.

Normal failures and SIGINT/SIGTERM attempt restart of a previously running
Sites process. The image stops cron, then the job, then Sites during shutdown.
SIGKILL or host loss cannot execute cleanup; inspect the run and restart Sites
through Supervisor or restart the Machine before retrying. Do not claim
zero downtime or automatic recovery from every failure.

## Access and custody

Use a dedicated Sites repository on the existing rsync.net account. Verify its
path and access before promotion; a dedicated path alone is not access
isolation. Reuse the existing credential layout and independent custody from
[the NixOS inventory](../nixos/README.md):

```text
/var/lib/finitecomputer/backups/rsync-net/id_ed25519
/var/lib/finitecomputer/backups/rsync-net/known_hosts
/var/lib/finitecomputer/backups/rsync-net/borg-passphrase
```

These files remain root-only beneath a private directory. Only the backup job
needs access; do not bake credentials into a serving image. Borg connects
directly to rsync.net over SSH, so a separate rsync of live databases is not
needed. Never copy an active Borg repository while it has writers.

For Fly, provision the files using the existing secret custody process and
[Fly file secrets](https://fly.io/docs/reference/configuration/#the-files-section).
Add the following mappings to the deployment configuration only when the
corresponding base64-encoded secrets are provisioned (import through stdin,
never place their values in shell arguments, logs or git):

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

The image precreates the root-only directory, tightens these files to `0600`
and removes their corresponding environment variables before starting Sites.
Never place them beneath the UID 65532-owned serving data directory.

The existing job selects remote executable `borg12`; the local test client is
Borg 1.4.x. Verify actual client/server compatibility at the destination.
A new dedicated repository needs its own exported Borg repokey, retained with
the passphrase independently of Fly. Also retain SSH access, the pinned host
identity, runtime/mail/service configuration and deploy artifact access.
Another repository's exported repokey is not a substitute.

Reuse the current no-prune policy. No automatic prune/compact is introduced.
The existing SSH credential has been documented as overprivileged; do not
claim server-enforced append-only protection. No secret transfers, remote
initialization, production service stops, or retention changes are authorized
by this document.

## Restore gate

From an independent recovery environment, configure native Borg with the
escrowed SSH identity, pinned host key, repository, passphrase and repokey,
without relying on the source Fly Machine. Use a fresh Borg client directory.
Check and extract the recorded archive into a new private scratch directory:

```sh
umask 077
borg check --verify-data
borg extract "::$ARCHIVE"
sites-backup restore --snapshot ./snapshot --target "$NEW_DATA_DIR"
```

`NEW_DATA_DIR` must not exist; its parent must be private and on the empty
recovery volume. Run as root to preserve UID/GID 65532 ownership. The script
checks all file hashes, symlink inventory and registry integrity, copies into
private staging, then installs without overwriting any existing target.
Never overwrite live data or start an incomplete/failed restore. Inspect
protected SQLite only through `scripts/snapshot-sqlite` or a scratch copy.

Restore the actual production-shaped snapshot onto an empty isolated Fly volume,
with dev mail and production egress blocked. Prove content/assets, sharing and
revocation, guest/account access, and clone/push/publish using pre-backup
credentials. Confirm the source is unchanged and record recovery time.
A successful upload or `borg check` alone is not enough.

## Local qualification

```sh
scripts/with-dev-env just sites-backup-contract
bash infra/images/sites-smoke.sh "$SITES_IMAGE"
SITES_BACKUP_SMOKE=1 bash infra/images/sites-smoke.sh "$SITES_IMAGE"
```

These tests exercise stopped-Sites capture, real native encrypted Borg,
fresh-client extraction, wrong-passphrase and corrupt-snapshot rejection,
no-overwrite restore, capture/upload/restart failures, interruption cleanup,
and restore followed by publishing with the original editor credential. The
exact-image workflow also runs real Supervisor/cron isolation and shutdown
tests. The obsolete content-addressed backup subsystem and its test contracts
are removed. Synthetic local evidence does not establish remote access,
production pause duration, external alerts or recovery from the real archive.

FIN-54 remains open until the adapted job runs, freshness/failure checks work,
and the real restore drill passes. No production rollout has occurred.
Run `scripts/finite-status` before and after any separately authorized rollout.
