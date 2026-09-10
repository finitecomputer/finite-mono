# Sites Hosting Options

**Option A / Latitude** remains the dedicated NixOS deployment in
[`infra/nixos/hosts/finite-sites-v2`](../../nixos/hosts/finite-sites-v2). Keep that definition
and the current production service intact. Latitude capacity is currently an
external constraint, not a reason to delete the option.

**Option B / Fly** is the active implementation path for this session:
one Fly Machine, one volume, one Sites daemon serving all static sites.
This directory prepares an empty-state demo, not a production cutover.
No Sprites, per-site runtimes, replication, migration framework, or CLI fleet
rollout are involved. The final provider choice remains open.

Track work in [FIN-49](https://linear.app/finitecomputer/issue/FIN-49).
Completion receipts must say **Option A / Latitude**, **Option B / Fly**, or
**Shared**, and distinguish prepared, demo-deployed, and production-cutover.
An uncompleted alternative is not a blocker to completing the selected path.

## Demo Target

The approved Option B demo uses Fly organization `finite`, app
`finite-sites-demo`, region `iad`, and the 10 GiB `sites_data` volume. This does
not select Fly permanently or authorize moving production users. The checked-in
configuration records this target; use an immutable CI image digest at deploy.

The initial demo image is pinned in `fly.toml`. It was built from
`f2778edb36581e8cf634f61fc3af943a259396a0` by
[Service Images run 34401864605](https://github.com/finitecomputer/finite-mono/actions/runs/34401864605),
which passed the AMD64 exact-image smoke with production promotion disabled.
Fly resolves the pinned OCI index to its AMD64 manifest
`sha256:ca998ad3d9d334e5cfa85c108b28e773935f23c9e3722b82d60f7c1777470078`.
The health endpoint at `https://finite-sites-demo.fly.dev/api/v2/healthz`
does not prove `finite.site` DNS/TLS, real mail, or the account bridge.

The app's DNS records are below. Confirm ownership and review existing records
before changing them. In particular, replace Namecheap URL forwarding only when
the domain owner confirms it is no longer needed; do not remove unrelated mail
or verification records.

| Type | Host | Value |
| --- | --- | --- |
| A | `@` | `66.241.124.192` |
| A | `*` | `66.241.124.192` |
| CNAME | `_acme-challenge` | `finite.site.md0y25m.flydns.net` |

These are the agreed minimal IPv4-only records, verified on September 9.
The allocated IPv6 address is not published in DNS. AAAA records are optional,
not a prerequisite or an outstanding demo task. Both apex and wildcard
certificates were verified Ready, and real-domain API/site requests pass TLS.

Creating certificate requests does not establish that TLS is ready. Check both
`finite.site` and `*.finite.site` in Fly after DNS changes propagate. Any explicit
subdomain record takes precedence over the wildcard; inventory those before
claiming all site URLs work. The Machine is not a production recovery target.

## Option B: Prepare The Artifact

Run the **Service Images** workflow with `image=sites` and
`publish_production=false` on the reviewed revision. It uses the root Rust pin
and locked workspace dependencies, builds `finitesitesd` and the matching
operator `fsite`, and publishes a canary under `ghcr.io/finitecomputer/finite-sites`.
The runtime contains Git (including its HTTP backend), CA certificates and
the libraries needed by the binaries. Neither core nor an agent runs in it.

The image smoke gate tests the exact pulled canary, with disposable volumes:
registration, owner-mailbox proof via the dev outbox, project init, Git hooks/push, committed content, private/public
visibility, non-root serving, graceful shutdown, restart, container replacement,
retained credentials/cookie secret, and another push using the original identity.
For a local Docker-capable development environment:

```sh
bash infra/images/sites-smoke.sh "$SITES_IMAGE"
```

Record the Git SHA, successful workflow URL and immutable `name@sha256:...`
from its summary. Do not deploy a tag or build on the destination Machine.
The first GHCR publication must be anonymously pullable; resolve package
visibility/access issues if that gate fails rather than bypassing it.
Package preparation or this container test is not evidence of a live Fly demo,
Fly edge compatibility, or a cross-version migration.

## Option B: Provision And Deploy The Demo

For a new target, before running these commands, confirm the Fly organization, billing authority,
app name, region and DNS ownership in FIN-50. `APP`, `ORG`, `REGION`, and
`SITES_IMAGE` below are operator-supplied values, not reserved resources.
Use an approved Fly CLI and scoped credentials; never put secrets in git.
For the existing demo, use `APP=finite-sites-demo`, `ORG=finite`, `REGION=iad`;
inspect existing resources and skip the creation commands. Do not create a
second volume or app on each deploy.

The config targets `finite.site` and `*.finite.site`. Confirm those are the
team's intended, available domains before using them. **Do not repoint any
domain currently serving production users for this demo.** If these names are
occupied, first agree on an isolated demo namespace and change all three daemon
URL/domain flags and the health-check Host together. Leave `git.finite.chat`,
canonical v1 Sites and the agent/runtime CLI pin unchanged.

```sh
fly apps create "$APP" --org "$ORG"
fly volumes create sites_data --app "$APP" --region "$REGION" --size 10
fly ips allocate-v4 --shared --app "$APP"
fly ips allocate-v6 --app "$APP"
fly certs add finite.site --app "$APP"
fly certs add '*.finite.site' --app "$APP"
```

Follow the DNS records returned by `fly certs show` for each name, including
wildcard ACME validation, and verify certificate issuance. Keep Cloudflare
records DNS-only for the initial Fly-edge qualification. No route allowlists:
Fly forwards the daemon's public listener, preserving Host. The daemon owns
API/Git/site dispatch. NIP-98 signatures must use the configured public HTTPS
URL, not the Machine's private listener URL.

The checked-in config uses `--mailer resend` with the existing approved sender
`Finite Sites <links@finite.chat>`. `RESEND_API_KEY` must be installed through
Fly secrets before deployment. The September 10 setup reuses the documented
send-only credential from lat2's root-owned `/etc/finite-saas/sites.env` without
modifying that production file. Never print or save the value during transfer;
`fly secrets import --stage` accepts it through stdin.

The earlier demo and the exact-image container smoke use `--mailer dev` with
synthetic addresses. That outbox flow is never proof of a real person's mailbox.
Current Project Init requires verified owner-mailbox authority; complete a real
Sites-key or email-login flow before claiming self-service verification.
Enable the optional dashboard
viewer-session exchange only with the matching reviewed dashboard/daemon
contract and the shared `FINITE_SITES_VIEWER_SESSION_TOKEN` secret. No secret
values belong in this runbook, tickets, command logs or screenshots.

```sh
scripts/finite-status
fly config validate --strict --app "$APP" --config infra/fly/sites/fly.toml
fly deploy --app "$APP" --config infra/fly/sites/fly.toml \
  --primary-region "$REGION" --image "$SITES_IMAGE" --ha=false
fly machine list --app "$APP"
fly volumes list --app "$APP"
fly checks list --app "$APP"
curl --fail --show-error https://finite.site/api/v2/healthz
scripts/finite-status
```

Check that exactly one application Machine exists and is attached to the
expected volume. `--ha=false` prevents spare creation but does not delete
preexisting Machines; investigate unexpected instances, never automatically
delete them. Keep autoscaling off. One shared CPU, 1 GiB RAM and 10 GiB storage
are initial demo sizing, not measured production capacity.

The volume mounts at `/var/lib/finite-sites`, containing the complete registry,
repositories, blobs, cookie secret and outbox. Startup refuses a missing mount,
initializes only its root ownership, then drops to UID/GID 65532 before serving.
It does not recursively repair imported data. SIGINT matches the daemon's
graceful-shutdown handler; Fly allows 30 seconds before forced termination.

## Option B: Demonstrate And Record

Use the matching reviewed v2 CLI in an isolated Finite Home, with
`FINITE_SITES_API=https://finite.site`. Follow the existing
[publish workflow](../../../finite-sites/README.md#publish-a-static-site), using
the Git URL returned by this server. Standalone publishers first use
`fsite auth sites-key request MAILBOX` and `fsite auth sites-key add MAILBOX TOKEN`
to prove the owner mailbox, then pass `--owner-email MAILBOX` to Project Init.
Use actual mail delivery for real-user proof. Do not update the public rolling release
or existing agents' default endpoint.

In FIN-24 record the demo URL, provider, app/Machine/volume identifiers, revision,
image digest and results of registration, init, push, content/assets, an update
at the same URL, private/public viewing and unauthorized rejection **through
the real Fly edge**. Restart the Machine and redeploy the pinned image; verify
the same content, grants and viewer sessions survive. The image smoke is not
a substitute for those checks. Mail and account-bridge checks remain pending
until their credentials and matching reviewed components are deployed.

### Verified Demo Boundary

The September 9-10 controlled demo in FIN-24 uses
`https://fly-proof-0909-7b6e.finite.site/`. It is private and contains only
synthetic content. Its completed **Option B / Fly** checks are:

- Isolated CLI registration, synthetic owner proof, config dry-run, Project
  Init, scoped Git authentication, publishing and an update at the same URL.
- Browser-rendered HTML/CSS and authenticated asset reads; anonymous,
  unshared and revoked viewers are denied.
- Public HTML/CSS/assets through the real edge preserve `Cache-Control:
  no-store`; returning to private immediately denies anonymous reads.
- Machine restart and redeployment of the same CI-pinned image preserve
  Project/Site IDs, active Version, content, cookie secret, viewer grants and
  existing viewer sessions. The original Git credential still reads source.
- Exactly one Machine remains attached to the original volume. No customer
  state, legacy routing, dashboard upstream or agent CLI pin changed.

The CLI version string alone is not the artifact identity: the operator client
reports `fsite 0.5.2` but implements v2. FIN-24 records the exact client image
identity separately from the public v1 release and deployed AMD64 image.

Real email is configured separately from those synthetic checks; successful
provider submission is not proof of inbox delivery or mailbox ownership. Sharing
this private URL alone does not grant access. Do not enable account redirects until the reviewed #854 dashboard and
daemon are deployed with a matching service credential and upstream. The
production dashboard has one Sites upstream; do not repoint it at this empty
demo registry while legacy Sites remain authoritative.

`scripts/finite-status` returns UNKNOWN on a laptop without a host profile.
Use the installed command on the authenticated app-plane host for fleet
evidence; Fly's passing health check is evidence only for this demo service.
Keep FIN-54 open until recurring off-host recovery and failure visibility are
qualified, even when a one-time synthetic restore succeeds.

### Recovery Proof And Limits

On September 10, the entire demo data directory was archived with its writer
stopped, transferred off Fly over SFTP into a private local directory, and
restored onto an empty local Docker volume using the exact CI-built AMD64
image. The restored copy retained Site/Project IDs, Version 2, HTML/assets,
grants and cookie secret; an existing authorized session worked and anonymous
access was denied. The original publisher cloned the backed-up Git commit,
passed `git fsck`, and pushed a new commit that created Version 3 **only on the
isolated restore**. The live demo remains private at Version 2. FIN-54 records
the archive location and checksum; never commit or attach the archive, which
contains the cookie secret and registry.

This was a manual maintenance-window proof, not a scheduled backup mechanism,
production-state migration, or fresh-Fly-volume restore. Production still
needs recurring independent off-host backups, freshness/failure reporting,
and a restore of the actual source state without an hourly serving outage.

Maintenance revealed a `flyctl machine update` pitfall: omitted `init` fields
can retain the previous entrypoint override. Restoring the serving command
alone is insufficient after a maintenance entrypoint. Explicitly restore
`/usr/local/bin/sites-entrypoint`, the serving arguments, and the original
services/mount/image configuration, then verify health and authenticated reads.
Do not treat CLI command completion alone as recovery evidence.

## Availability And Later Cutover

A single-volume, single-Machine demo has restart/deployment downtime. It is
not a no-downtime or highly available production design. Do not clone or scale
it to a second independently writable volume to hide that limitation.

Redeploy a previously verified image digest only when its data contract is
compatible. Image rollback does not roll back the volume. Before production
data arrives, FIN-54 must prove off-host backups and restoration of the whole
Recovery Set onto an empty target. Fly volume snapshots alone are not that
proof. Do not destroy a volume during a deployment or rollback.

FIN-52 owns inventory/rehearsal, FIN-53 the account bridge, FIN-55 the actual
cutover and old-link compatibility, and FIN-56 the CLI/runtime rollout.
Nothing in this demo runbook authorizes those production mutations.

The September 10 live inventory found 25 published apps and eight published
documents, contrary to the original zero-app assumption. Alex's updated decision
is to retain them on the legacy service while static sites move. Never open the
authoritative legacy registry with the static-only daemon: its migrations remove
unsupported kinds. Preserve legacy routes/auth and qualify the mixed legacy/v2
dashboard preview path before changing the dashboard upstream. FIN-52/FIN-55
own the explicit site mapping; no blanket wildcard redirect or legacy shutdown.

Provider references:
[Fly configuration](https://fly.io/docs/reference/configuration/),
[deploy flags](https://fly.io/docs/flyctl/deploy/),
[custom domains](https://fly.io/docs/networking/custom-domain/),
[volume limitations](https://fly.io/docs/volumes/overview/).
