# Deploying Finite Sites on Fly

Production is `finite.site`, hosted by the `finite-sites-demo` Fly app in the
`finite` organization. [`fly.toml`](../fly/sites/fly.toml) is the deployment
configuration: one Machine and one persistent volume. All publishing and live
validation use this service. Restore drills use disposable isolated targets,
removed after their recovery artifacts are retained.

Run `scripts/finite-status` on the app-plane host before and after rollouts.
Build images in CI. Never overwrite accepted writes with a test database or
open an unmigrated source registry with the static-only daemon.

## Fly

Set `APP=finite-sites-demo`.
Inspect its Machine, `sites_data` volume, IPs and issued certificates for
`finite.site` and `*.finite.site` before deployment. Preserve existing data.

Import `RESEND_API_KEY` from the
[secret inventory](../nixos/README.md#secrets-bootstrap-checklist-values-never-in-this-repo)
using `fly secrets import --stage --app "$APP"`, with assignments on stdin and
values excluded from logs. Account login and backups have separate gates below.

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

Require exactly one serving Machine on the intended volume. `--ha=false` does
not remove existing Machines; do not create independent writable copies.
Imported files must be accessible to UID/GID 65532. After maintenance overrides,
restore `/usr/local/bin/sites-entrypoint`, serving arguments, image, services
and mount—not just the command.

## Usage metrics

`GET /internal/v1/metrics` serves aggregate Prometheus usage metrics on the API
host. It is disabled (503) unless `FINITE_SITES_METRICS_TOKEN` is configured.
Use a dedicated 32-byte random value encoded as exactly 64 lowercase hex
characters; empty/malformed values fail startup. Never reuse
`FINITE_SITES_VIEWER_SESSION_TOKEN`: the server rejects matching credentials.
The metrics token grants only this read-only endpoint; viewer-session and
publishing credentials do not authorize it. Responses use `Cache-Control: no-store`.

1. Store the new credential in the team password manager. Provision the value
   through Fly's existing secret-import flow on stdin, without printing it or
   placing it in arguments. Its name is `FINITE_SITES_METRICS_TOKEN`.
2. Provision the same raw token at
   `/etc/finite/monitoring/sites-metrics-token` on the monitoring receiver,
   `root:finite-monitoring` mode `0640` on Ubuntu (`root:prometheus` on the
   declared NixOS receiver). This file must never enter Git or the Nix store.
3. Build the reviewed Sites image in CI and deploy the immutable digest using
   the Fly procedure above. Public uptime probes must stay healthy; verify the
   metrics request with authentication returns Prometheus text, while absent
   or incorrect credentials receive 401. Requests on wildcard site hosts must
   remain site traffic, not expose API metrics.
4. Back up/reconcile the receiver's Prometheus configuration, validate it with
   `promtool check config`, reload, and require `up{job="finite-sites-metrics"}`
   to be 1 with fresh aggregate samples. Redirect following is disabled for
   this authenticated scrape. Preserve unrelated live receiver configuration.
5. Deploy the merged overview through the dashboard workflow. Verify existing
   and published totals, the 90-day UTC bars, and the collection-health tile.
   Record canonical `scripts/finite-status` before and after rollout.

Do not change creation timestamps to repair a chart. The metrics read retained
registry rows directly; they cannot reconstruct purged history.
Rollback restores the previous image and scrape configuration. No schema or
data migration is involved; a disabled/older endpoint leaves usage panels
unavailable rather than reporting zero. The dashboard workflow alone does not
deploy the service image, provision this credential, or change scrape jobs.

### Diagnose missing usage data

Check the serving image, endpoint, and Prometheus target separately. A successful
dashboard workflow deploys the panels only; the API/wildcard uptime probes do
not collect usage metrics.

| Observation | Next action |
| --- | --- |
| `/internal/v1/metrics` returns 404 on `finite.site` | Compare the live image revision with the metrics-capable CI image; deploy the qualified digest. |
| Endpoint returns 503 | Check whether `FINITE_SITES_METRICS_TOKEN` is configured and inspect service logs for collection errors. |
| Authenticated scrape returns 401 | Reconcile the dedicated Fly token and receiver credential file. |
| `up{job="finite-sites-metrics"}` returns no series | Install/reload the Sites scrape job on the receiver after provisioning its token. |
| Target is up but panels remain unavailable | Check that `finite_sites_metrics_collected_at_seconds` is less than 180 seconds old and not in the future; use a dashboard end time after the first successful scrape. |

Accept the rollout only when the live Grafana queries return existing/published
totals, 90 daily buckets, and healthy collection. Preserve the receiver's other
scrape jobs when reconciling its configuration.

## Account bridge

Deploy the reviewed dashboard image and verify `/site-auth` before enabling
`FINITE_SITES_ACCOUNT_LOGIN_URL=https://finite.computer/site-auth` on Sites.
The dashboard and Sites share `FINITE_SITES_VIEWER_SESSION_TOKEN`.
Nix owns `FC_SITES_UPSTREAM_URL=https://finite.site` for both viewer sessions
and publishing assertions; remove stale endpoint assignments from
`/etc/finite/dashboard.env`, retaining its other settings and mode `0600`.

Verify shared/unshared accounts, revocation, email fallback, direct visits and
iframes. Previous content links redirect before Sites-owned sign-in; old-host
cookies and tokens do not transfer. Sites failure must not prevent Chat.
The protocol is defined in [ADR 0029](../../finite-sites/docs/adr/0029-account-session-viewer-bridge.md).

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

Qualify the Runtime with the matching CLI before promoting it or `fsite-latest`.
Older `/api/v1` clients cannot publish to `/api/v2`. Update saved Git remotes
and host-scoped credentials using the server-returned URL; content redirects
cannot migrate them. Deploy the dashboard assertion issuer with the Runtime
and prove an existing agent can publish and update through Hosted Chat.
Assertions belong to the publishing registry; sharing a service credential
does not make tokens portable between registries.

## Backups and restore

The production Fly configuration enables daily backups to
`fm2890@fm2890.rsync.net:finitecomputer/finite-sites`. Its upload SSH key is
restricted to that repository with `borg12 serve --append-only`; keep the
separate repository-restricted recovery key outside Fly. Append-only preserves
repository segments, but does not prevent logical archive deletion; recovery
may still require operator repair. No job prunes or compacts the repository.

The complete Sites Recovery Set (repositories, blobs, registry, permissions
and cookie key) has restored from rsync.net onto an empty target. Repeat that
proof when changing the recovery contract; image smoke tests and Fly volumes
alone do not prove remote recovery. Qualification evidence and the remaining
external backup-alert verification are tracked in
[FIN-54](https://linear.app/finitecomputer/issue/FIN-54).

### Enable backups

Provision a dedicated Borg repository using `repokey-blake2`; the job never
initializes one. Keep its exported repokey, passphrase, SSH identity, pinned host
key and service/mail configuration independently of Fly. Verify the SSH key's
access restrictions: a dedicated repository path alone provides no isolation
or append-only protection.

The Fly configuration declares these settings in its `[env]` table:

```toml
FINITE_SITES_BACKUP_ENABLED = "1"
FINITE_SITES_BACKUP_REPOSITORY = "fm2890@fm2890.rsync.net:finitecomputer/finite-sites"
FINITE_SITES_BACKUP_REMOTE_PATH = "borg12"
```

Provision base64-encoded Fly file secrets through the secret custody process,
keeping values out of git and command arguments. The configuration has one
`[[files]]` entry per row, with `guest_path` under
`/var/lib/finitecomputer/backups/rsync-net/`:

| `secret_name` | `guest_path` filename |
| --- | --- |
| `FINITE_SITES_BORG_SSH_KEY` | `id_ed25519` |
| `FINITE_SITES_BORG_KNOWN_HOSTS` | `known_hosts` |
| `FINITE_SITES_BORG_PASSPHRASE` | `borg-passphrase` |

Deploy a qualified image using the procedure above. The image protects the
credential directory as root-only and the files as `0600`. Allow space for a
full snapshot in `/var/backups/finite-sites`; this staging/status directory is
private, outside serving data, and ephemeral across Machine replacement.

Backups run daily at 03:07 UTC. Serving pauses for the consistent copy
(approximately two minutes, accepted for launch), then resumes before upload.
Capture has a five-minute deadline and attempts restart on failure. Runs are
serialized; warnings fail the job. There is no automatic retry, prune or compact.

### Check or retry

Inside the backup-enabled Machine as root:

```sh
supervisorctl -c /etc/sites-supervisor.conf start sites-backup
supervisorctl -c /etc/sites-supervisor.conf status sites-backup
finite-status --sites-backup-state /var/backups/finite-sites/status.json --json
```

`start` is asynchronous. Require a fresh archive ID and source/upload timestamps
after completion; freshness defaults to 36 hours and a missing receipt is
unknown. This command sends no alerts. After forced termination, check and
restore Sites health before retrying.

### Restore or drill

Use a matching Sites image on an isolated target with no Sites writer and an
empty volume mounted at `/var/lib/finite-sites`. Run as root to preserve UID/GID
65532. Keep production routing unchanged and control outbound mail during drills.

Using the independently held recovery SSH key and encryption credentials,
configure Borg 1.x with `BORG_REPO`,
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

## Recovery

Roll back to a previous image digest only when it can read the current state.
Preserve the data volume; binary rollback does not undo migrations or writes.
If a Git push was accepted but publication failed, reconcile it after service
recovery. Never restore an old database over newer accepted writes. Fence
publishing and diagnose on copies before choosing a recovery image.

Sites recovery uses the independent [Sites backup procedure](#backups-and-restore).
Hosted Recovery Snapshot format v4 covers Chat/Core/Brain/Identity separately.
Historical v3 snapshots remain readable by the restore tool. Retain the old
Sites data directory and complete recovery archives; service retirement does
not authorize purging user data.

## Content redirect maintenance

The app-plane Caddy serves reviewed redirects for previous content URLs. It
runs no Sites daemon. Keep `/etc/finite-saas/sites.env`: Identity and Brain
still read that mail credential despite its historical filename.

1. Prepare a private JSON array of `from_host` / `to_host` mappings for retained
   content. Sources must be one-label hosts under `finite.chat` or
   `docs.finite.chat`. Disposable `*.v2.finite.chat` validation URLs are excluded
   from both redirects and dashboard previews; no DNS/TLS route is retained for
   them. Generate the Caddy fragment with `infra/scripts/sites-redirects`.
   Install it atomically as `/etc/finite/sites-redirects.caddy`, root:caddy `0640`.
   The file is required, including when deploying the shared Caddy module on
   another host. Missing mappings must fail validation before activation.
2. Validate the complete candidate Caddy configuration with that fragment as
   the `caddy` user; root validation can create root-owned access logs that
   prevent reload. Require `ReloadResult=success` after activation.
3. Verify mapped GET/HEAD content requests return 302 and `no-store`, preserving
   paths and queries. Old auth routes, API/Git hosts and unmapped content must
   return 410. Old login tokens and Git credentials must never be forwarded.
   Check document deep links, public content and private/unshared access at the
   destination. Run `scripts/finite-status` before and after rollout.

Redirects do not move cookies, Git credentials or remotes. Maintain the exact
reviewed destination for each source host; a hostname is not authority to infer
ownership or rewrite durable state. This runbook does not itself authorize
production mutation or a Runtime rollout.
