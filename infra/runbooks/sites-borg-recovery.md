# Sites Borg backup and empty-target recovery

Tracking: [FIN-54](https://linear.app/finitecomputer/issue/FIN-54).
Decision: [Sites ADR 0031](../../finite-sites/docs/adr/0031-borg-on-existing-rsync-net.md).

## Status and boundaries

This is the operator procedure for a **completed local Sites Recovery Point**.
The automated test uses a real, encrypted local Borg repository and a fresh
Borg client to restore, serve, and publish through Sites. It does not prove
remote access, recurring backup coverage, or a production Fly restore.

Option B / Fly is active. Option A / Latitude remains available; it uses the
same Recovery Point and Borg formats. This document does not change either
deployment or authorize remote initialization, credential copies, production
capture, retention deletion, or cutover. Keep existing Chat/host Borg jobs and
repositories unchanged. Run `scripts/finite-status` before and after an
authorized rollout.

Borg connects directly to **rsync.net over SSH**; a separate rsync upload is
not needed. Never rsync live SQLite/Git directories and call them a backup.
Do not copy or rsync an active Borg repository while it has writers.

## Provisioning prerequisites

Select a **dedicated Sites repository** on the existing rsync.net account;
do not append Sites archives to the Chat/host repositories. Its actual path
and access restrictions must be verified before configuration is promoted.
Reuse the credential layout in [the NixOS inventory](../nixos/README.md):

```text
/var/lib/finitecomputer/backups/rsync-net/id_ed25519
/var/lib/finitecomputer/backups/rsync-net/known_hosts
/var/lib/finitecomputer/backups/rsync-net/borg-passphrase
```

Files are root-owned `0600`, beneath a `0700` directory. Reuse independent
custody; never paste values into commands, tickets, or this repository. The
Sites process does not need access to the credentials: only the authorized
backup operator/job does. Do not distribute the broad existing SSH credential
into a serving image by default. The Fly job placement and secret mounting are
still to be qualified.

The existing [Borg module](../nixos/modules/backups.nix) selects remote
executable `borg12`; the repository-pinned development client is Borg 1.4.x.
Verify the actual endpoint's supported Borg 1.x executable and client/server
compatibility during the remote drill. Do not silently select Borg 2 or an
unencrypted repository.

For a separately authorized **new, empty** repository, native
`borg init --encryption=repokey-blake2` creates its own repokey. Export that key
using `borg key export` into protected independent custody. Reuse the existing
passphrase/SSH custody convention, not another repository's exported repokey.
Independently retain endpoint, SSH key, pinned host identity, passphrase, Sites
repokey export, runtime configuration, service/mail credentials, and access to
the exact deploy artifact. The Sites data capture includes its cookie key,
not all of its external runtime secrets.

The established SSH credential has previously been documented as
overprivileged. A dedicated path and a no-prune job are **not** server-enforced
append-only protection. Verify restrictions; do not change credentials or
permissions used by existing backups as part of this procedure.

## Archive a completed point

Use a pinned build containing the local `finitesitesd backup` commands and Borg
1.x. Set the following non-secret environment variables deliberately:

- `SITES_DATA`: absolute path to the source Sites data directory.
- `SITES_POINTS`: absolute protected local Recovery Point store, outside the
  source tree, with an existing parent directory.
- `BORG_REPO`: the verified dedicated Sites repository location.
- `BORG_REMOTE_PATH`: the verified Borg 1.x executable on rsync.net.

Configure native Borg's environment without loading or printing secret values:

```sh
set -euo pipefail
umask 077
: "${SITES_DATA:?}" "${SITES_POINTS:?}" "${BORG_REPO:?}" "${BORG_REMOTE_PATH:?}"
export BORG_REPO BORG_REMOTE_PATH
export BORG_PASSCOMMAND='cat /var/lib/finitecomputer/backups/rsync-net/borg-passphrase'
export BORG_RSH='ssh -i /var/lib/finitecomputer/backups/rsync-net/id_ed25519 -o BatchMode=yes -o UserKnownHostsFile=/var/lib/finitecomputer/backups/rsync-net/known_hosts -o StrictHostKeyChecking=yes'
evidence=$(mktemp -d)
borg info --json > "$evidence/repository.json"
jq -e '.encryption.mode == "repokey-blake2"' "$evidence/repository.json"
finitesitesd backup capture --data "$SITES_DATA" --repository "$SITES_POINTS" > "$evidence/capture.json"
point=$(jq -er '.id | select(test("^[0-9a-f]{64}$"))' "$evidence/capture.json")
finitesitesd backup restore --repository "$SITES_POINTS" --point "$point" --target "$evidence/verified"
(
  cd "$SITES_POINTS"
  borg create --compression=auto,zstd --files-cache=disabled --json \
    "::sites-$point" objects "points/$point"
) > "$evidence/archive.json"
borg info --json "::sites-$point" > "$evidence/archive-info.json"
```

All commands must exit **zero**, including Borg: warnings are not success.
Do not run multiple capture/archive invocations concurrently against this
staging store, edit immutable files, or remove objects while archival runs.
Record the Sites point ID, capture timestamp, Borg archive ID, repository
identity, source build, command outcomes, and completion timestamp in private
operator evidence. Do not attach registry content or secrets. The verification
target contains plaintext private state; leave it offline and clean up private
scratch only after evidence is recorded.

A retry with the same archive name fails rather than overwrites. After an
interrupted or uncertain upload, inspect that exact archive and restore it to
an empty scratch target before accepting it as complete. Never delete/recreate
the archive merely to make a retry green. Do not select a `.checkpoint` archive
or a mutable "latest" name as the recovery authority.

This first operator procedure includes the local store's `objects` directory
and the selected completion manifest. Borg deduplicates unchanged bytes, but
each archive still enumerates the stored objects and may retain objects from
older points. **This is not revision-triggered scheduling or a storage
reclamation policy.** Dependency-selected archival, writer coordination,
durable work/retries, metadata cadence, resource bounds, and retention remain
gates before enabling recurring production backups. Do not install a cron job
around this full-catalog command as a substitute.

## Restore without the serving host

From a separate recovery environment, recover the credential bundle and runtime
configuration from independent custody. Use a fresh Borg client cache. Select
the exact recorded repository, archive ID/name, and Sites point ID. Confirm
`borg info --json "::sites-$point"` matches the receipt, not merely its name.
Exported keys can be imported with Borg's native recovery procedure if needed;
do not replace a key in a live repository speculatively.

Set `SITES_RESTORE_TARGET` to an absolute **nonexistent** path on the empty
target with an existing parent. After setting the same native Borg environment:

```sh
set -euo pipefail
umask 077
: "${point:?}" "${SITES_RESTORE_TARGET:?}"
recovery=$(mktemp -d)
borg check --verify-data "::sites-$point"
(
  cd "$recovery"
  borg extract "::sites-$point"
)
finitesitesd backup restore --repository "$recovery" --point "$point" --target "$SITES_RESTORE_TARGET"
```

Borg validates encrypted chunks; Sites validates its manifest, blobs, SQLite,
Git history, and inventory before atomic no-replace installation. Wrong
passphrases, missing/corrupt dependencies, or an existing target fail closed.
Never start a failed restore. A successful extraction or `borg check` alone is
not an application restore. Inspect protected SQLite only with
`scripts/snapshot-sqlite` or a disposable scratch copy.

For the real Option B drill, boot the restored data on an **empty isolated Fly
volume**, using dev mail and blocked production egress. Prove original content
and assets, ownership and sharing, account/guest viewing and revocation, and
clone/push/publish using pre-backup credentials. Confirm the original source
has not changed. Record elapsed recovery time and the source-state exposure
window. This remains a production cutover gate.

## Recurring operation gate

The production archival job must not prune or compact. Retention/deletion is a
separately authorized administrative operation after restore proof; Borg's
chunk reference tracking replaces custom S3 object lifecycle handling.
Do not apply Chat's retention/cadence values to Sites implicitly.

Before enabling backups, qualify a non-disruptive consistent checkpoint,
revision work tracking, metadata-only checkpoints, retries and reconciliation,
capture/upload resource limits, a measured backup lag target, reporting through
`scripts/finite-status`, and independent failure/staleness alerts. Refreshing
an old archive does not refresh the source-state timestamp. A publish response
does not promise that asynchronous off-host backup has finished.

## Local regression

```sh
scripts/with-dev-env cargo test -p finitesitesd --locked --test e2e borg_restore_
```

The test uses synthetic credentials, local encrypted Borg storage, and a fresh
client after deleting the capture store and writer cache. It proves archive
replay rejection, wrong-passphrase rejection, damaged-point refusal and
no-overwrite restore, followed by application serving and continued
publishing. It does not access rsync.net, production secrets, or Fly.
