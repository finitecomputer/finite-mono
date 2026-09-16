# Deploying Finite Sites on Fly

Fly is the destination for the complete Sites switchover. The
[`fly.toml`](../fly/sites/fly.toml) configuration owns the deployment's domains,
ports and resource sizes: one Machine with one persistent volume.

Preserve the source data and archives until the Fly restore and access checks
pass. Never open the live source registry with the static-only daemon: its
startup migrations remove unsupported output kinds.

Run `scripts/finite-status` on the authenticated app-plane host before and after
rollouts; Fly health alone does not establish platform health. Build artifacts
in CI from the reviewed revision, never on a production host.

## Fly

The configured app is `finite-sites` in the `finite` organization. Set
`APP=finite-sites`. Inspect this app's resources and provision any missing app
or `sites_data` volume before deployment; use 10 GiB as the initial volume size.
Obtain the app's allocated IPs and certificate DNS records from Fly. Require
issued certificates for both the API apex and wildcard Site hosts; do not copy another app's DNS records. Keep records DNS-only when
qualifying Fly's TLS edge. [Fly provisioning documentation](https://fly.io/docs/launch/).

Install `RESEND_API_KEY` from the
[secret inventory](../nixos/README.md#secrets-bootstrap-checklist-values-never-in-this-repo)
using `fly secrets import --stage --app "$APP"` with secret assignments on stdin.
Keep values out of logs and files. Configure any dashboard account exchange
separately; do not repoint the legacy dashboard upstream at an empty registry.

Set `REVIEWED_REF` to a branch or tag at the reviewed revision and `VERSION` to
an image version label. From the repository root:

```sh
gh workflow run service-images.yml --ref "$REVIEWED_REF" \
  -f image=sites -f version="$VERSION" -f publish_production=false
```

Wait for the workflow's source-revision, anonymous-pull and image smoke checks.
Set `SITES_IMAGE` to the immutable `ghcr.io/finitecomputer/finite-sites@sha256:...`
printed in that run's summary. Select the image for this rollout explicitly;
do not assume the checked-in pin includes your source changes:

```sh
APP=finite-sites
fly config validate --strict --app "$APP" --config infra/fly/sites/fly.toml
fly deploy --app "$APP" --config infra/fly/sites/fly.toml \
  --image "$SITES_IMAGE" --ha=false
fly machine list --app "$APP"
fly volumes list --app "$APP"
fly checks list --app "$APP"
```

Require exactly one application Machine attached to the intended volume.
`--ha=false` suppresses automatic spare creation; it does not remove existing
Machines. A single volume has restart/deploy downtime and must not be scaled
into independent writable copies.

The [entrypoint](../images/sites-entrypoint) requires the data mount, adjusts
only its root ownership, and runs Sites as UID/GID 65532. Imported files must
already be accessible to that user. After maintenance using a Machine entrypoint
override, explicitly restore `/usr/local/bin/sites-entrypoint`, the serving
arguments, image, services and mount; restoring the command alone is insufficient.

## Account bridge

Follow [ADR 0029](../../finite-sites/docs/adr/0029-account-session-viewer-bridge.md)
and the [dashboard configuration](../../finitecomputer-v2/apps/dashboard/README.md#sites-account-viewer-boundary).
Deploy the dashboard's `/site-auth` route before enabling
`FINITE_SITES_ACCOUNT_LOGIN_URL=https://finite.computer/site-auth` on Sites.
Both services must receive the same `FINITE_SITES_VIEWER_SESSION_TOKEN` from the
secret inventory. Keep `FC_SITES_UPSTREAM_URL` on legacy and set
`FC_SITES_V2_UPSTREAM_URL` to the Fly service API. A failed v2 exchange must not
retry against legacy.

Before enabling traffic, verify authorized and unshared accounts, the guest
email fallback, share revocation, direct Site visits and dashboard iframes on
the real domains. Also prove legacy app/document previews and Hosted Chat
requester assertions still work when v2 is unavailable. The bridge grants no
new shares and does not migrate old URLs. Its synthetic legacy fixture and
development browser tests do not replace a restore of the actual source state
and live account verification.

## Verify

Use the matching reviewed CLI with an isolated `FINITE_HOME` and
`FINITE_SITES_API` set to the Fly service's public API. Follow the
[publishing workflow](../../finite-sites/README.md#publish-a-static-site), using
the server-returned Git URL. Standalone publishers must first verify their
mailbox with `fsite auth sites-key request` / `add`; pass that mailbox as
`--owner-email` for Project Init.

On a disposable project, verify Init, Git push, rendered HTML/assets, a second
publish at the same URL, private/public viewing, and rejection of unshared or
revoked viewers. Mutable HTML and assets must return `Cache-Control: no-store`.
After restart and artifact replacement, verify content, grants, existing viewer
sessions and Git credentials still work. Exercise real mail and any enabled
account exchange through the public domains.

The [container smoke test](../images/sites-smoke.sh) covers synthetic publishing,
visibility and restart/replacement. It does not qualify real mail, Fly TLS, the
dashboard account bridge or migration from a legacy database.

## Backups and restore

Backups are disabled by default. Before production migration, prove the complete
Sites Recovery Set (repositories, blobs, registry, permissions and cookie key)
restores from rsync.net onto an empty target. Image smoke tests and Fly volumes
alone do not prove remote recovery.

### Enable backups

Provision a dedicated Borg repository using `repokey-blake2`; the job never
initializes one. Keep its exported repokey, passphrase, SSH identity, pinned host
key and service/mail configuration independently of Fly. Verify the SSH key's
access restrictions: a dedicated repository path alone provides no isolation
or append-only protection.

Add these settings to the Fly configuration's existing `[env]` table:

```toml
FINITE_SITES_BACKUP_ENABLED = "1"
FINITE_SITES_BACKUP_REPOSITORY = "<approved dedicated Sites repository>"
FINITE_SITES_BACKUP_REMOTE_PATH = "borg12"
```

Provision base64-encoded Fly file secrets through the secret custody process,
keeping values out of git and command arguments. Add one `[[files]]` entry per
row, with `guest_path` under `/var/lib/finitecomputer/backups/rsync-net/`:

| `secret_name` | `guest_path` filename |
| --- | --- |
| `FINITE_SITES_BORG_SSH_KEY` | `id_ed25519` |
| `FINITE_SITES_BORG_KNOWN_HOSTS` | `known_hosts` |
| `FINITE_SITES_BORG_PASSPHRASE` | `borg-passphrase` |

Deploy a qualified image using the procedure above. The image protects the
credential directory as root-only and the files as `0600`. Allow space for a
full snapshot in `/var/backups/finite-sites`; this staging/status directory is
private, outside serving data, and ephemeral across Machine replacement.

Once enabled, backups run daily at 03:07 UTC. Sites stops only for the consistent
file/SQLite copy, then resumes before hashing, verification and upload. Stop and
capture share a five-minute deadline; on timeout the job aborts capture and
attempts a bounded restart before cleanup. Confirm this pause and recovery
interval are acceptable. Runs are serialized; Borg warnings fail the job. There
is no automatic retry, prune or compact.

### Check or retry

Inside the backup-enabled Machine as root:

```sh
supervisorctl -c /etc/sites-supervisor.conf start sites-backup
supervisorctl -c /etc/sites-supervisor.conf status sites-backup
finite-status --sites-backup-state /var/backups/finite-sites/status.json --json
```

`start` is asynchronous: check after completion for a fresh archive ID and both
source and upload timestamps. The freshness limit defaults to 36 hours; a
missing receipt is unknown. Set up external failure/freshness alerts; this
command sends no notifications. After host loss or forced termination, check
Sites health and restart through Supervisor or restart the Machine before retrying.

### Restore or drill

Use a matching Sites image on an isolated target with no Sites writer and an
empty volume mounted at `/var/lib/finite-sites`. Run as root to preserve UID/GID
65532. Keep production routing unchanged and control outbound mail during drills.

Using independently held credentials, configure Borg 1.x with `BORG_REPO`,
`BORG_REMOTE_PATH=borg12`, `BORG_PASSCOMMAND` reading the private passphrase file,
`BORG_RSH` using strict pinned-host checking and the SSH key, and a fresh private
`BORG_BASE_DIR`. Select `ARCHIVE` from `borg list`, independent of the lost
Machine's status file. Never copy a Borg repository while it has writers.

The restore tool requires a nonexistent target, so restore to scratch first:

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

The tool verifies inventory, checksums and SQLite integrity. Stop on any error;
never overwrite a nonempty volume or start a partial restore. Allow scratch
space for both copies, protect them as secrets, and remove them after verification.
Inspect snapshot SQLite only through `scripts/snapshot-sqlite` or a scratch copy.
Restore service configuration, start Sites, and run the [verification](#verify)
checks with pre-backup credentials before moving traffic.

## Recovery and cutover

Roll back to a previous image digest only when it can read the current state.
Preserve the data volume; binary rollback does not undo migrations or writes.
If a Git push was accepted but publication failed, reconcile it after service
recovery. Never restore an old database over newer accepted writes.

Only agreed published static Sites migrate. Preserve archives for retired
apps/documents and unpublished or missing-source projects. Rehearse on isolated
copies; use the [offline reconciliation procedure](sites-static-output-reconciliation.md)
for supported legacy output IDs. Mixed projects need an explicit retained Site
and proof of another publish.

Production cutover needs a separately reviewed site/URL mapping, final
service-consistent copy under a Publishing Write Freeze, access verification,
and a rollback boundary for destination writes. Preserve legacy API, Git and
auth routes until their consumers are retired; do not blanket-redirect them.
Redirect only explicitly mapped static content URLs, preserving path and query;
old-host cookies and tokens cannot establish a new-host session. Qualify saved
Git remotes and existing agents before changing CLI defaults or runtime pins.
This deployment procedure does not authorize cutover or a CLI/runtime rollout.
