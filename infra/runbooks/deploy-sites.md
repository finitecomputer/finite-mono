# Deploying finite-sites v2

ADR 0028 makes Finite Sites a static-only platform service. The validation
deployment is one dedicated NixOS VPS/VM running `finitesitesd` separately from
the rest of the server stack.

This runbook does not cut canonical `finite.chat` Sites traffic over. The
finite-lat-2 app-plane service stays in `legacy-canonical` mode, pinned to the
released `fsite/v0.5.2` daemon, so `api.finite.chat`, `*.finite.chat`, and
`*.docs.finite.chat` continue to serve the public v1 contract until a separate
cutover PR changes that boundary deliberately.

## Topology

- API and git smart HTTP: `https://v2.finite.chat`
- Served sites: `https://{site}.v2.finite.chat/`
- Daemon listener: `127.0.0.1:8787`
- State: `/var/lib/finite-sites`
- Healthcheck: `GET /api/v2/healthz`
- Systemd unit: `finite-saas-sites.service`
- NixOS config: `nixosConfigurations.finite-sites-v2`
- NixOS modules: `infra/nixos/modules/finitesitesd.nix`,
  `infra/nixos/modules/caddy-sites-v2.nix`, and
  `infra/nixos/modules/finite-sites-v2-backups.nix`

There is no app runner, document renderer, Kata/containerd dependency, or
wake-on-request path in v2 Sites.

`fsite/v*` releases still ship the `fsite` CLI + `finitesitesd` linux binary
([release-cli.md](release-cli.md)), but the validation daemon is deployed by a
reviewed NixOS closure from the pinned mono rev, not by copying a release
tarball onto the box.

## Preconditions

- The finitesitesd source change is merged to `main`.
- The deploy artifact is built from the exact reviewed monorepo revision; do
  not build on the production host.
- DNS has `v2.finite.chat` and `*.v2.finite.chat` pointed at the validation
  host or edge.
- The host has a Cloudflare Origin CA cert/key at
  `/etc/finite-saas/certs/finite-sites-v2-origin.{pem,key}` covering
  `v2.finite.chat` and `*.v2.finite.chat`.
- The validation host has the mail secrets documented by name in
  `infra/README.md`; do not write secret values into git.
- The local backup timers from `finite-sites-v2-backups.nix` are enabled, and
  an off-host copy plan exists before real production traffic moves.

## Deploy

1. Build/download the reviewed
   `nixosConfigurations.finite-sites-v2.config.system.build.toplevel` closure
   artifact using the host deployment procedure.

2. Activate the reviewed closure on the host. The service command should look
   like:

   ```sh
   finitesitesd serve \
     --data /var/lib/finite-sites \
     --listen 127.0.0.1:8787 \
     --base-domain v2.finite.chat \
     --api-url https://v2.finite.chat \
     --git-url https://v2.finite.chat \
     --site-scheme https \
     --site-port none \
     --mailer resend \
     --mail-from "Finite Sites <links@finite.chat>"
   ```

3. Keep config changes in `infra/nixos/hosts/finite-sites-v2/` and
   `infra/nixos/modules/`; do not edit production units by hand.

## Verify

1. `systemctl status finite-saas-sites` is active.
2. `curl -fsS https://v2.finite.chat/api/v2/healthz` succeeds.
3. `systemctl start finite-sites-v2-snapshot.service` succeeds, then
   `systemctl start finite-sites-v2-restore-check.service` succeeds.
4. `fsite auth register --output json` works with
   `FINITE_SITES_API=https://v2.finite.chat`.
5. `fsite project init --config finite.toml --dry-run --output json` returns a
   `site` object or `site: null`, never `outputs`.
6. Create an operator-owned disposable static project, push the configured
   Deploy Branch, and confirm the returned site URL serves the committed
   bytes.
7. Exercise viewer sharing:

   ```sh
   fsite project share PROJECT --public --yes-public --output json
   fsite project share PROJECT --private --output json
   ```

8. Probe one real HTML URL and one real asset URL through the public edge.
   Both must preserve `Cache-Control: no-store` while URLs remain mutable.

## Rollback

Rollback is the previous NixOS generation or the previous known-good reviewed
closure for the validation host. Re-run the healthcheck and one static-site
read after rollback.

Rollback must not delete `/var/lib/finite-sites`. If a git ref was accepted
but deployment failed before rollback, fix forward with a new commit after the
service is healthy.
