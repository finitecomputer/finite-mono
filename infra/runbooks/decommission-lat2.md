# Decommission finite-lat-2

> **SUPERSEDED 2026-08-28 (ADR 0007, emergency):** lat1's thermal failure
> changed lat2's destination from "decommission and release" to "wipe and
> reinstall as the replacement app-plane host" — see
> [`lat2-replacement-cutover.md`](lat2-replacement-cutover.md). The
> archive/offload steps below are **skipped by owner decision** (lat1's data
> is the authority; lat2's local copies are stale); the wipe happens as Gate
> A of the cutover runbook. Still load-bearing after the cutover:
> unregister the four GitHub runners (`runners.md` inventory) and rotate the
> legacy credentials this file names (Core runner token, Resend key, tunnel
> key, finite-search env values).

PRECONDITIONS:

- Explicit production authorization names this runbook and the intended end
  state: archive what must be kept, unregister every lat2 runner, revoke or
  rotate credentials, then wipe or release the host.
- `scripts/finite-status` is green or the incident owner has accepted the
  reason it is not green. Lat2 is not allowed to become a rollback, deploy, or
  recovery target while being decommissioned.
- Current lat1 backups and the hosted recovery set are independently
  verified. Do not treat any lat2 copy as the only recovery authority.
- A private encrypted destination exists for any retained host archive. This
  public repo records paths, hashes, and decisions only; it never stores secret
  values or private tarballs.

## What to keep

The current tree keeps the lat2 host summary and runner-removal inventory under
`infra/hosts/lat2/`. The pre-cutover systemd/Caddy capture, old deploy note,
and proposed backup timer were removed from the active tree; use git history
only if forensic reconstruction needs them. Do not re-copy deleted host-capture
files from the machine unless a fresh read-only drift check finds a difference
worth reviewing.

Candidates for a private off-host archive:

| Path | Keep? | Handling |
|---|---|---|
| `/var/lib/finite-sites` | Yes, if present | Private encrypted archive. Contains historical registry/app/blob/git state. Validate SQLite through `scripts/snapshot-sqlite` from a scratch extract before accepting. |
| `/var/backups/finite-sites` and `/var/backups/finite-cleanup` | Yes, if present | Private encrypted archive. These are dated historical tarballs; do not trust them until listed, hashed, extracted, and checked. |
| `/home/ubuntu/finite-search` | Maybe | Archive only for historical search configuration/source context. Exclude env files from any non-secret archive. |
| `/home/ubuntu/finite-sites` | Maybe | Historical rsync source tree with no git provenance. Keep only if it is useful forensic evidence; it is not deploy authority. |
| `/opt/finite/finitecomputer` | Usually no | Legacy build-on-box checkout and dormant runner tooling. Record a value-free file manifest if needed; do not archive `secrets/` into ordinary storage. |
| `/etc/finite-saas` | Secret-bearing | Do not place in a general archive. Verify current custody on lat1 or an approved vault, then revoke/rotate or include only in an encrypted secret archive. |
| `/etc/finite-computer/runner.env` | Secret-bearing | Do not archive as plain text. It contains legacy Core runner credential names and values. Prefer revoke/rotate and delete. |
| `/home/ubuntu/finite-search/**/.env` | Secret-bearing | Do not archive as plain text. Verify lat1 has the required current values, then rotate if they may remain accepted. |
| `/srv/github-runner` | No | Unregister every runner and delete. Runner credentials are revoked, not preserved. |
| Docker caches, Nix store, build `target/`, `_work` directories | No | Reproducible or disposable cache. Delete after runner removal. |

## Steps

1. Freeze new use of lat2.

   Confirm no current workflow, runbook, or operator command will schedule work
   on lat2. GitHub runner removal in later steps should make accidental
   scheduling impossible.

2. Collect read-only evidence.

   Do not read secret files. Collect service status, runner inventory, and
   path sizes only:

   ```sh
   ssh finite-lat-2 '
     set -eu
     hostname -f
     date -u
     systemctl --no-pager --plain list-units "actions.runner.*" || true
     systemctl --no-pager --plain list-units "finite-*" "caddy" || true
     sudo du -sh \
       /var/lib/finite-sites \
       /var/backups/finite-sites \
       /var/backups/finite-cleanup \
       /home/ubuntu/finite-search \
       /home/ubuntu/finite-sites \
       /opt/finite/finitecomputer \
       /srv/github-runner \
       /etc/finite-saas \
       /etc/finite-computer 2>/dev/null || true
   '
   ```

3. Create the private archive.

   Use an encrypted off-host destination approved for secret-bearing evidence.
   Keep the archive outside this repository. A safe shape is one general
   history archive plus, only if explicitly needed, one separate encrypted
   secret archive with narrower access.

   Example source selection for a general private archive. This stages on a
   root-only path, then streams the tarball to an encrypted off-host file. If
   the approved archive backend is not `age`, replace only the encryption/copy
   command; keep the root-only staging and exclusions.

   ```sh
   LAT2_ARCHIVE_ID=finite-lat-2-decommission-$(date -u +%Y%m%dT%H%M%SZ)
   LAT2_ARCHIVE_DEST=/path/to/private/archive
   LAT2_ARCHIVE_AGE_RECIPIENT=<AGE_RECIPIENT>

   ssh finite-lat-2 '
     set -eu
     sudo install -d -m 0700 /root/lat2-decommission
     sudo sh -c '"'"'
       umask 077
       tar --one-file-system --xattrs --acls -czf /root/lat2-decommission/lat2-history.tgz \
         --exclude=/home/ubuntu/finite-search/searxng/.env \
         --exclude=/home/ubuntu/finite-search/firecrawl-upstream/.env \
         --exclude=/opt/finite/finitecomputer/secrets \
         --exclude=/srv/github-runner \
         --exclude='"'"'*/target'"'"' \
         --exclude='"'"'*/node_modules'"'"' \
         /var/lib/finite-sites \
         /var/backups/finite-sites \
         /var/backups/finite-cleanup \
         /home/ubuntu/finite-search \
         /home/ubuntu/finite-sites \
         /opt/finite/finitecomputer
     '"'"'
     sudo sha256sum /root/lat2-decommission/lat2-history.tgz
   '
   ssh finite-lat-2 'sudo cat /root/lat2-decommission/lat2-history.tgz' \
     | age -r "$LAT2_ARCHIVE_AGE_RECIPIENT" \
       > "$LAT2_ARCHIVE_DEST/$LAT2_ARCHIVE_ID-history.tgz.age"
   ssh finite-lat-2 \
     'sudo shred -u /root/lat2-decommission/lat2-history.tgz 2>/dev/null || sudo rm -f /root/lat2-decommission/lat2-history.tgz'
   ```

   Do not stage the tarball in `/tmp`. `/tmp` is tmpfs on this host, shared,
   and not a durable archive location.

4. Verify the archive before touching registrations or disks.

   From a scratch machine or directory, list the archive, extract it, record a
   manifest, and validate SQLite snapshots without opening live host files:

   ```sh
   mkdir -p scratch/lat2-restore
   age -d -o scratch/lat2-history.tgz \
     "$LAT2_ARCHIVE_DEST/$LAT2_ARCHIVE_ID-history.tgz.age"
   tar -tzf scratch/lat2-history.tgz > scratch/lat2-history.list
   tar -xzf scratch/lat2-history.tgz -C scratch/lat2-restore
   (
     cd scratch/lat2-restore
     find . -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum > manifest.sha256
   )
   scripts/snapshot-sqlite integrity-check \
     scratch/lat2-restore/var/lib/finite-sites/registry.db
   ```

   If a non-secret archive was expected, scan the file list before accepting it
   and fail closed on env files, private keys, runner credentials, or root-only
   secret directories.

5. Unregister GitHub runners.

   Follow `infra/hosts/lat2/runners.md`. Remove all runner registrations whose
   names start with `finite-lat-2` from:

   - `finitecomputer/finite-mono`
   - `finitecomputer/finitechat`
   - `finitecomputer/finitecomputer`
   - `finitecomputer/finitecomputer-v2`

   Do not preserve `.credentials`, `.credentials_rsaparams`, `.runner`, or
   runner work directories.

6. Revoke or rotate old credentials.

   At minimum, close these credential surfaces before the machine is
   repurposed or released:

   - GitHub runner tokens: revoked by runner unregister.
   - `/home/ubuntu/.ssh/finite-lat2-core-tunnel`: remove any corresponding
     authorized key on lat1 and delete the private key from lat2.
   - `/etc/finite-computer/runner.env`: revoke legacy Core runner/API
     credentials if still accepted.
   - `/etc/finite-saas/sites.env`: verify lat1 or vault custody for the
     current Resend credential; rotate if the old value may still be accepted.
   - `/etc/finite-saas/certs/finite-chat-origin.key`: rotate the Cloudflare
     Origin CA key if it was copied into a broader archive or if disk wipe is
     not immediately verified.
   - finite-search env values: verify lat1 custody and rotate any credential
     still accepted by current services.

7. Stop and remove old services.

   The selective pre-wipe service-disable list was removed with the historical
   lat2 unit captures. Under ADR 0007, the owner-approved path is the Gate A
   wipe/reinstall in `lat2-replacement-cutover.md`; do not preserve old Ubuntu
   services as fallback authority. Runner registrations still need explicit
   removal before any host reuse:

   ```sh
   ssh finite-lat-2 '
     set -eu
     sudo systemctl disable --now "actions.runner.*" || true
     sudo rm -rf /srv/github-runner
   '
   ```

8. Wipe or release the host.

   Prefer the provider reinstall/wipe path if the machine will be repurposed.
   If releasing the server, use the provider's disk wipe/reinstall control
   before cancellation when available. Do not repurpose the host with old
   runner credentials, service secrets, or private keys still present on disk.

## Verify

- `scripts/finite-status` still reports the fleet healthy or its accepted
  incident state is unchanged.
- GitHub shows no online or offline runner named `finite-lat-2*` in any Finite
  repository.
- DNS, Cloudflare, and service runbooks do not point any production route at
  `64.34.80.19`.
- The private archive has a recorded SHA-256, file manifest, and scratch
  extraction result. `registry.db` passes `scripts/snapshot-sqlite
  integrity-check` if `/var/lib/finite-sites` was archived.
- The credential rotation/revocation checklist above has an owner and final
  state recorded outside this public repo.
- The provider console shows the machine wiped, reinstalled, or released.

## Rollback

Before runner unregister or host wipe, rollback is simply to stop and keep
lat2 unused while the archive issue is fixed. After runner unregister, do not
re-register lat2 runners; CI now uses Depot. After wipe/release, there is no
host rollback. Restore only from the verified private archive or from current
lat1 recovery authorities, depending on which data is needed.
