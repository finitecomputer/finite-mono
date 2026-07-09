# Deploying finite-saas-core (and dashboard) to lat1

Since the 2026-07-09 consolidation cutover, Core and the dashboard are NixOS
services on finite-lat-1 (64.34.82.77). Config lives in `infra/nixos/`
(host `finite-lat-1`, modules `modules/finite-saas-core.nix` +
`modules/dashboard.nix`); topology and secrets checklist:
`infra/nixos/README.md`. Rebuild/recovery of the box itself:
[lat1-nixos-reinstall.md](lat1-nixos-reinstall.md).

- **Core** = systemd unit `finite-saas-core.service`, binds 127.0.0.1:4200,
  DynamicUser, `EnvironmentFile=/etc/finite/core.env`. Talks to native
  Postgres at 127.0.0.1:5432 via `FC_CORE_DATABASE_URL`.
- **Dashboard** = podman container `finite-saas-dashboard` (host-net, binds
  127.0.0.1:3000), image **digest-pinned** in `modules/dashboard.nix`
  (`ghcr.io/finitecomputer/finite-saas-dashboard@sha256:...`),
  `EnvironmentFile=/etc/finite/dashboard.env`, `FC_CORE_BASE_URL=
  http://127.0.0.1:4200`.
- **Edge** = the single host Caddy: `finite.computer/internal/finite-private/*`
  → core:4200, everything else → dashboard:3000.

> History: this box previously ran Core/dashboard/Postgres as a single-node
> k3s cluster with on-host podman builds and `kubectl set image`. That cluster
> is GONE (wiped at the cutover). Do not resurrect the kubectl flow.

## Deploy flow — nixos-rebuild pinned to a mono rev

Deploying a release IS pinning the flake: the mono rev you build is the rev
the host runs (binaries + config together). The dashboard is the exception —
it deploys as a digest-pinned GHCR container, so bumping it is an edit to
`modules/dashboard.nix`.

### PRECONDITIONS

- The change (Core source and/or the dashboard digest bump) is merged to
  `main` — you deploy a committed rev, not a working tree.
- ssh access to lat1 (`ssh root@64.34.82.77`, key-only) or a driver host with
  nix that can reach it as root.
- For a dashboard bump: the new image is CI-built and pushed to GHCR, and you
  have its `name@sha256:...` digest (from the Service Images workflow summary).

### STEPS

1. **Core (and any config/module change):**

   ```sh
   nixos-rebuild switch --target-host root@finite-lat-1 \
     --flake github:finitecomputer/finite-mono/<tag-or-rev>#finite-lat-1
   ```

   The rev that tagged the Core binary is the rev the host runs.

2. **Dashboard image bump:** edit `image = "...@sha256:..."` in
   `infra/nixos/modules/dashboard.nix`, commit to `main` — the committed
   digest is the deploy record and the rollback target — then run the same
   `nixos-rebuild switch` against the new rev. podman pulls the pinned digest.

3. Config-only validation without a linux builder (see `infra/nixos/README.md`):
   `nix eval .#nixosConfigurations.finite-lat-1.config.system.build.toplevel.drvPath`.

### VERIFY

1. Core health directly on the box:

   ```sh
   ssh root@finite-lat-1 'curl -fsS http://127.0.0.1:4200/healthz'
   ```

2. Through the edge: `curl -fsS https://finite.computer/` (dashboard) and
   `curl -fsS https://finite.computer/internal/finite-private/` → 401
   (core alive + gated — the limiter path).
3. Units are up: `ssh root@finite-lat-1 'systemctl status finite-saas-core
   finite-saas-dashboard'` (`podman-finite-saas-dashboard.service` for the
   container unit name if querying journald).
4. TODO: Core still exposes no `source_commit` health payload; until it does,
   confirm the deployed rev by the NixOS generation
   (`nixos-rebuild list-generations` on the host) rather than a runtime probe.

### ROLLBACK

1. Fast path: `ssh root@finite-lat-1 nixos-rebuild switch --rollback` — boots
   the previous generation (both Core binary and dashboard digest revert
   together). Then reconcile git to match what is running within a day
   (break-glass rule).
2. Deliberate path: re-run `nixos-rebuild switch --flake ...#finite-lat-1`
   pinned to the previous known-good mono rev (and, for a dashboard-only
   regression, revert the digest in `modules/dashboard.nix`).
3. Re-run VERIFY.
