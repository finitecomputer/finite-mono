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

The configured demo is `finite-sites-demo` in the `finite` organization. Set
`APP=finite-sites-demo` and reuse its existing Machine and `sites_data` volume.
For a new Fly app, provision an app and volume
(the demo uses 10 GiB), then obtain its allocated IPs and certificate DNS records
from Fly. Require issued certificates for both the API apex and wildcard Site
hosts; do not copy another app's DNS records. Keep records DNS-only when
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
already be accessible to that user. After maintenance using a Machine entrypoint
override, explicitly restore `/usr/local/bin/sites-entrypoint`, the serving
arguments, image, services and mount; restoring the command alone is insufficient.

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

## Recovery and cutover

Roll back to a previous image digest only when it can read the current state. Preserve the data volume; binary rollback does not undo migrations or
writes. If a Git push was accepted but publication failed, reconcile it after
service recovery. Never restore an old database over newer accepted writes.

Before production migration, prove an independent backup of the complete Sites
Recovery Set restores onto an empty target, including repositories, blobs,
registry and cookie secret. Local snapshots and Fly volumes alone do not prove
this. Inspect snapshot SQLite through `scripts/snapshot-sqlite` or a scratch copy.

Only agreed published static Sites migrate. Preserve archives for retired
apps/documents and unpublished or missing-source projects. Rehearse on isolated
copies; use the [offline reconciliation procedure](sites-static-output-reconciliation.md)
for supported legacy output IDs. Mixed projects need an explicit retained Site
and proof of another publish.

Production cutover needs a separately reviewed site/URL mapping, final
service-consistent copy under a Publishing Write Freeze, access verification,
and a rollback boundary for destination writes. Preserve legacy API, Git and
auth routes until their consumers are retired; do not blanket-redirect them.
This deployment procedure does not authorize cutover or a CLI/runtime rollout.
