# Sites on Fly

Run one `finitesitesd` serving static sites on one Fly Machine and one volume.
This runbook does not authorize production mutations or establish a completed
cutover. Production migration and old-link compatibility belong in the
[Sites deployment runbook](../../runbooks/deploy-sites.md).

[Latitude deployment](../../nixos/hosts/finite-sites-v2) is also supported.

## Configuration

[`fly.toml`](fly.toml) is the deployment configuration. Its target is:

| Setting | Value |
| --- | --- |
| Organization / app / region | `finite` / `finite-sites-demo` / `iad` |
| API and Git URL | `https://finite.site` |
| Site URLs | `https://{site}.finite.site/` |
| Public listener | `0.0.0.0:8787`, forwarded by Fly with HTTPS enforced |
| Machine | One shared CPU, 1 GiB RAM |
| Volume | `sites_data`, 10 GiB, mounted at `/var/lib/finite-sites` |
| Lifecycle | Rolling deploy; auto-stop off; auto-start on; minimum one Machine |
| Shutdown | `SIGINT`, 30-second grace period |
| Health check | `GET /api/v2/healthz`, Host `finite.site`; 15s grace, 30s interval, 5s timeout |

Organization and volume size are provisioning inputs, not fields in
`fly.toml`. The image is pinned there; a deployment must select the reviewed
CI-built digest explicitly. Sizing is not a measured production capacity claim.
Fly forwards the daemon's public listener without route allowlists and preserves
Host; the daemon owns API, Git and site dispatch. NIP-98 signatures use the
configured public HTTPS URL.

## Build the Artifact

Run commands from the repository root. Set `REVIEWED_REF` to the reviewed
revision and `VERSION` to the build's version label, then dispatch
[Service Images](../../../.github/workflows/service-images.yml):

```sh
gh workflow run service-images.yml --ref "$REVIEWED_REF" \
  -f image=sites -f version="$VERSION" -f publish_production=false
```

The workflow uses the root Rust pin and locked workspace dependencies to build
the AMD64 daemon and matching operator `fsite`. Wait for its anonymous GHCR pull
and exact-image smoke gates to pass. Set `SITES_IMAGE` to the immutable
`ghcr.io/finitecomputer/finite-sites@sha256:...` from that run's summary and
confirm the source revision. Resolve package-access failures before deployment.
Do not deploy a mutable tag or build on the destination Machine.

To run the same smoke gate in a Docker-capable development environment:

```sh
bash infra/images/sites-smoke.sh "$SITES_IMAGE"
```

It uses disposable volumes to test registration, synthetic mailbox proof,
init, Git push, visibility, non-root serving, shutdown, restart and replacement
with retained credentials and cookie secret. It does not qualify real mail,
the Fly edge, the account bridge or cross-version migration.

## Provision and Configure DNS

Use scoped Fly credentials. For the configured target:

```sh
APP=finite-sites-demo
ORG=finite
REGION=iad
```

Inspect existing resources before provisioning. Run creation commands only for
an authorized new target after confirming organization, billing, region and DNS
ownership; skip them for existing resources:

```sh
fly apps create "$APP" --org "$ORG"
fly volumes create sites_data --app "$APP" --region "$REGION" --size 10
fly ips allocate-v4 --shared --app "$APP"
fly certs add finite.site --app "$APP"
fly certs add '*.finite.site' --app "$APP"
```

The configured app's DNS records are:

| Type | Host | Value |
| --- | --- | --- |
| A | `@` | `66.241.124.192` |
| A | `*` | `66.241.124.192` |
| CNAME | `_acme-challenge` | `finite.site.md0y25m.flydns.net` |

These records are specific to this app; use the allocated addresses and
certificate records for any new app. The allocated IPv6 address is not
published in DNS; AAAA records are optional. Review existing records and
explicit subdomains, which override the wildcard. Replace Namecheap URL
forwarding only with domain-owner approval; preserve unrelated mail and
verification records. Do not repoint domains serving production users.
Keep Cloudflare records DNS-only during Fly-edge qualification.

After propagation, check both names with `fly certs show` and require issued
certificates. Creating certificate requests or passing health on
`finite-sites-demo.fly.dev` does not prove apex/wildcard DNS and TLS. If changing
the namespace, update all three daemon URL/domain flags and the health-check
Host together.

## Mail and Account Bridge

The configuration uses `--mailer resend` and sender
`Finite Sites <links@finite.chat>`. Install `RESEND_API_KEY` in Fly secrets before
deployment. The approved send-only credential lives in lat2's root-owned
`/etc/finite-saas/sites.env`; transfer it without modifying that file or printing
or saving its value. `fly secrets import --stage --app "$APP"` accepts secret
assignments through stdin. See the [secret inventory](../../nixos/README.md#secrets-bootstrap-checklist-values-never-in-this-repo).

Project Init requires verified owner-mailbox authority. Complete a real
Sites-key or email-login flow for publisher enrollment; a synthetic dev outbox
or successful viewer login does not prove this flow.

The optional dashboard verified-email exchange requires matching reviewed
dashboard/daemon contracts and the same server-only
`FINITE_SITES_VIEWER_SESSION_TOKEN` (64 lowercase hex characters). Its host
location is `/etc/finite/sites-viewer-session.env`; provision the matching Fly
secret only when enabling the exchange. It does not create sharing grants.
Do not enable account redirects before that contract and upstream are ready.
Configure `FC_SITES_V2_UPSTREAM_URL` for this service while retaining
`FC_SITES_UPSTREAM_URL` for legacy consumers. Neither backend retries against
the other. Qualify both paths before enabling the account bridge; do not
repoint the legacy upstream at an empty registry.

## Deploy and Verify

Run `scripts/finite-status` on the authenticated app-plane host before and after
each rollout. A laptop without a host profile returns UNKNOWN; a Fly health
check does not establish platform health.

```sh
fly config validate --strict --app "$APP" --config infra/fly/sites/fly.toml
fly deploy --app "$APP" --config infra/fly/sites/fly.toml \
  --primary-region "$REGION" --image "$SITES_IMAGE" --ha=false
fly machine list --app "$APP"
fly volumes list --app "$APP"
fly checks list --app "$APP"
curl --fail --show-error https://finite.site/api/v2/healthz
```

Require exactly one application Machine attached to the expected volume.
`--ha=false` prevents spare creation but does not delete existing Machines;
investigate unexpected instances without automatically deleting them. Keep
autoscaling off.

Use the matching reviewed v2 CLI in an isolated Finite Home with
`FINITE_SITES_API=https://finite.site`. Follow the
[static publishing workflow](../../../finite-sites/README.md#publish-a-static-site)
using the Git URL returned by this server. Standalone publishers use
`fsite auth sites-key request MAILBOX`, then
`fsite auth sites-key add MAILBOX TOKEN`, and `--owner-email MAILBOX` for Init.
Keep tokens out of logs. Do not change the public rolling release, agents'
default endpoint or runtime CLI pin as part of this deployment.

On an authorized disposable project, check registration, Init, push, rendered
HTML/assets and an update at the same URL through the real Fly edge. Check
private/public viewing, unshared and revoked viewer rejection, and
`Cache-Control: no-store` on mutable HTML and assets. Restart and redeploy the
pinned image; verify content, grants, viewer sessions and original Git
credentials survive. Exercise real mail and the account bridge separately
when their prerequisites are present.

## Durability and Recovery

The volume contains the registry, repositories, blobs, cookie secret and
outbox. The [entrypoint](../../images/sites-entrypoint) refuses a missing mount,
initializes only its root ownership and drops to UID/GID 65532. It does not
recursively repair imported data.

One Machine and one volume mean restart/deploy downtime and no high
availability. Do not scale to independently writable volumes. Image rollback
does not roll back data: use a previous digest only when its data contract is
compatible, and never destroy the volume during deployment or rollback.

A Fly volume or its snapshots are not independent off-host backups. Before
production data arrives, require recurring service-consistent off-host backups,
freshness/failure reporting and restoration of the whole Recovery Set onto an
empty target. This runbook does not establish that backups are enabled. Protect
archives as secrets; inspect snapshot SQLite only through
`scripts/snapshot-sqlite` or a scratch copy. A restore check must verify content,
IDs, grants, existing viewer sessions and Git integrity, then exercise a push
only on the isolated restored copy.

**Entrypoint restoration:** `flyctl machine update` can retain a previous
entrypoint override when `init` fields are omitted. Restoring the serving
command alone after maintenance is insufficient. Explicitly restore
`/usr/local/bin/sites-entrypoint`, the serving arguments, and the original
services/mount/image configuration, then verify health and authenticated reads.
CLI command completion alone is not recovery evidence.

## Migration Boundary

Only the agreed published static sites migrate. Apps, documents and
unpublished/missing-source projects are retired at cutover; preserve the source
archive because retirement does not authorize durable data deletion. Never
open the authoritative legacy registry with the static-only daemon: its
migrations remove unsupported kinds.

Until a separately authorized cutover, preserve legacy routes/auth and
`git.finite.chat`. The [Sites deployment runbook](../../runbooks/deploy-sites.md)
owns the site mapping, account transition and old-link compatibility. This
runbook authorizes no wildcard redirect, legacy shutdown or production data
mutation.
