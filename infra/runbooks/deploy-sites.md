# Deploying Finite Sites on Fly

Fly is the destination for the complete Sites switchover. The
[`fly.toml`](../fly/sites/fly.toml) configuration owns the deployment's domains,
ports and resource sizes: one Machine with one persistent volume.
The public service uses real mail. Enable dashboard account login only after
its route and shared credential are deployed and verified. The dedicated
rsync.net restore proof remains a migration gate; public validation alone does
not satisfy it.

Preserve the source data and archives until the Fly restore and access checks
pass. Never open the live source registry with the static-only daemon: its
startup migrations remove unsupported output kinds.

Run `scripts/finite-status` on the authenticated app-plane host before and after
rollouts; Fly health alone does not establish platform health. Build artifacts
in CI from the reviewed revision, never on a production host.

## Fly

The production Fly app retains its historical name `finite-sites-demo` in the
`finite` organization; set `APP=finite-sites-demo`. It serves `finite.site` and
already contains live writes. Do not create a replacement app or overwrite its
volume with a rehearsal database. The separate `finite-sites` app holds private
rehearsal state and must not be promoted as production. Inspect the production
Machine, attached
`sites_data` volume, IPs and issued apex/wildcard certificates before deploying.

Install `RESEND_API_KEY` from the
[secret inventory](../nixos/README.md#secrets-bootstrap-checklist-values-never-in-this-repo)
using `fly secrets import --stage --app "$APP"` with secret assignments on stdin.
Keep values out of logs and files. Configure any dashboard account exchange
separately, after verifying the intended registry and account route.

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
APP=finite-sites-demo
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
already be accessible to that user, including the Git trees and mail outbox.
Check ownership after all staging writes. After maintenance using a Machine entrypoint
override, explicitly restore `/usr/local/bin/sites-entrypoint`, the serving
arguments, image, services and mount; restoring the command alone is insufficient.

## Account bridge

Follow [ADR 0029](../../finite-sites/docs/adr/0029-account-session-viewer-bridge.md)
and the [dashboard configuration](../../finitecomputer-v2/apps/dashboard/README.md#sites-account-viewer-boundary).
Deploy and qualify the dashboard's `/site-auth` route before applying the
account login setting in `fly.toml`. The pre-cutover dashboard lacks this route;
setting an upstream alone is insufficient. Stage the reviewed dashboard image
and include its deployment in the authorized cutover.
Both services must receive the same `FINITE_SITES_VIEWER_SESSION_TOKEN` from the
secret inventory. The NixOS dashboard module owns the single non-secret origin:
`FC_SITES_UPSTREAM_URL=https://finite.site`, used for viewing and publishing
assertions. Remove stale Sites endpoint assignments from
`/etc/finite/dashboard.env`, preserving its other settings and root:root `0600`.
Deploy a dashboard image built from the reviewed single-origin implementation;
the previous image's separate-origin behavior is incompatible with this config.

Verify authorized and unshared accounts, email fallback, share revocation,
direct Site visits and dashboard iframes on the real domains. Missing Sites
availability must leave Chat usable. Previous content links navigate through
the edge redirect and then Sites-owned sign-in; no account credentials are sent
to the previous content host. Cookies and login links do not migrate across
hosts. Historical-state fixtures and local tests do not replace live checks.

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

The pre-cutover Runtime's `fsite/v0.5.3` calls `/api/v1` and cannot use the new
`/api/v2` service. Build the staged Runtime with this revision's CLI and qualify it before cutover;
promote that Runtime and the public `fsite-latest` alias only with the canonical
endpoint.
Saved Git remotes also require an explicit update to the server-returned URL
and host-scoped credential storage. Content redirects do not migrate Git or
API requests. Retire the previous publishing listener at cutover; old clients must fail
clearly rather than create a second history.

The Hosted Chat requester-assertion issuer must also move to the publishing
registry before qualifying an existing agent. Assertions are random tokens
stored in the issuing registry, not portable signed claims. Deploy the reviewed
dashboard issuer and Runtime together. Prove one existing agent can publish and
update through Hosted Chat, then verify Chat still works when Sites is unavailable.
A standalone CLI test does not cover this boundary.

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
attempts a bounded restart before cleanup. The full-data rehearsal paused
serving for about 108–113 seconds; this daily pause was accepted for the initial launch. Re-measure
after material data growth. Runs are serialized; Borg warnings fail the job. There
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

Prepare the cutover review package outside git because it contains customer data:

- Assign every output a retained static Site or archive-only disposition, with
  exact old/new URLs. Render documents ahead of time; copy only reviewed public
  HTML/assets from app bundles. Preserve browser JavaScript where useful, but
  do not copy server code, databases, dependency trees or credentials into a
  deploy path. Unsupported server behavior is retired. Check document deep
  links, relative assets and app pages with their backends absent.
- Preserve source history, owner, visibility, shares and grants for retained
  Sites. A fallback is not permission to make private content public. Mixed
  projects need an explicit retained Site and proof of a subsequent publish.
  Use the [offline reconciliation procedure](sites-static-output-reconciliation.md)
  for supported output IDs. Record proposed source commits and observed branch
  tips; drift requires review, never a force-push.
- Capture the destination's current projects, shares, sessions, Git credentials
  and cookie key as well as the frozen source. Rehearse their reconciliation on
  copies; never replace current Fly state with an older one-project snapshot or
  with the rehearsal database, which contains synthetic writes.

### Activate redirects and retire the app-host service

Only after authorized migration and access checks pass:

1. Under the final publishing freeze, capture and independently archive the
   complete source Recovery Set. Preserve the old data directory and complete
   archives; this rollout does not purge them. Record the final archive and
   credential custody before removing ongoing app-host Sites backups.
2. Prepare a private JSON array of `from_host` / `to_host` mappings for retained
   content. Generate the Caddy fragment with `infra/scripts/sites-redirects`.
   Install it atomically as `/etc/finite/sites-redirects.caddy`, root:caddy `0640`.
   The file is required, including when deploying this shared Caddy module on
   another host. Missing mappings must fail validation before activation.
3. Validate the candidate's complete Caddy configuration with that fragment.
   Deploy the reviewed app-host closure only after the new dashboard image and
   Runtime are qualified. This removes `finite-saas-sites`, its package,
   health checks and ongoing host snapshot dependency. Caddy redirects mapped
   GET/HEAD content requests with 302 and `no-store`; paths and queries survive.
   Old auth routes, API/Git hosts and unmapped content return 410. Neither old
   login tokens nor Git credentials are forwarded to another host.
4. Verify mapped URLs, document deep links, private/unshared access, current
   public content, saved Git remotes and an existing agent's next publish.
   Run `scripts/finite-status`. Keep the Identity mail credential at
   `/etc/finite-saas/sites.env`; Identity still reads it despite the old filename.
5. Create and verify a new hosted Recovery Snapshot and archive it. Format v4
   covers Chat/Core/Brain/Identity without Sites. Historical v3 snapshots remain
   readable by the restore tool; current Sites recovery follows the independent
   [Sites backup procedure](#backups-and-restore).

Rollback must preserve all destination writes. Do not restart an old writable
registry as an automatic closure rollback, or restore an old database over new
commits. Fence publishing, diagnose on copies, and choose a recovery image that
can read the current state. This runbook does not itself authorize production
mutation or a Runtime rollout.
