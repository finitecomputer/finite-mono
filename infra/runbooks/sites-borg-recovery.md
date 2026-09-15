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

Do not copy the entire shared host job and its Chat/Core/Identity/runner stops
to Fly. Adapting Sites lifecycle control and volume access is still unfinished.
Keep existing LAT2 backup jobs and archives unchanged.

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

From an independent recovery environment, use native Borg to check and extract
the recorded archive into private scratch. Validate the snapshot using the
adapted existing restore checks before installing it onto an empty target.
Never overwrite live data or start an incomplete/failed restore. Inspect
protected SQLite only through `scripts/snapshot-sqlite` or a scratch copy.

Restore the actual production-shaped snapshot onto an empty isolated Fly volume,
with dev mail and production egress blocked. Prove content/assets, sharing and
revocation, guest/account access, and clone/push/publish using pre-backup
credentials. Confirm the source is unchanged and record recovery time.
A successful upload or `borg check` alone is not enough.

## Existing local proof

```sh
scripts/with-dev-env cargo test -p finitesitesd --locked --test e2e borg_restore_
```

This synthetic local test already proves native encrypted Borg archival,
fresh-client extraction, wrong-passphrase/replay rejection, and application
restore followed by publishing with the original editor credential. It uses
the previously built local capture/restore commands; that content-addressed
format is not mandatory for the simpler production adaptation. Test the chosen
stopped-Sites snapshot flow as it is wired up rather than adding new machinery
to satisfy restrictions of the optional local command.

FIN-54 remains open until the adapted job runs, freshness/failure checks work,
and the real restore drill passes. No production rollout has occurred.
Run `scripts/finite-status` before and after any separately authorized rollout.
