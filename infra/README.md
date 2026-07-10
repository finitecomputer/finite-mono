# infra/ — the single deploy root

Everything Finite runs in production is defined here. The north star:

> A release tag in finite-mono is sufficient, by itself, to reproduce any
> artifact we ship and to deploy any service we run. Nothing is built on a
> prod box. Nothing requires knowledge that lives only in someone's shell
> history.

## Post-cutover headline (2026-07-09)

**finite-lat-1 is now the consolidated NixOS app server, and it runs the whole
coupled cluster.** Its definition is `infra/nixos/` (host `finite-lat-1`); the
reinstall/recovery procedure is `infra/runbooks/lat1-nixos-reinstall.md`.

What the 2026-07-09 lat1 consolidation cutover changed:

- **One app server, one config tree.** finite-saas-core, dashboard,
  finitechat-server (migrated off clawland), finitesitesd (migrated off lat2),
  finite-search, and Postgres all run on lat1, defined in `infra/nixos/`.
- **Native Postgres.** Postgres 16 is a `services.postgresql` systemd service
  (db `finite_core`, 87 Finite Private keys) — **no more k3s StatefulSet**.
- **One Caddy edge.** A single Caddy on lat1 fronts `finite.computer`,
  `chat.finite.computer`, `*.finite.chat` (Cloudflare Origin CA), and
  `*.docs.finite.chat`. **No Traefik, no k3s, no socat bridges.**
- **`nixos-rebuild` is THE deploy.** Deploying a release pins the flake to the
  rev that tagged the binaries. The old *six distinct deploy mechanisms* — k3s
  `kubectl apply` + on-host `podman build` (lat1), systemd + Kata (lat2), Nix
  fleet `just host-deploy` (smoke/clawland), and the hand-run finitechat script
  — are **resolved for the coupled cluster**: one `nixos-rebuild --flake
  ...#finite-lat-1`. On-host `podman build` is gone; first-party images are
  CI-built and digest-pinned (`infra/images/`).

Deferred / still elsewhere: **finite-brain** stays on smoke (migrates later
with the auth-integration follow-up); **the CI runners** live on lat2;
**clawland** remains the legacy finite.vip fleet box; **Tinfoil** is unchanged.

## Layout

```
infra/
  nixos/       # finite-lat-1 AS CODE — the live definition of the app server
  hosts/
    lat1/      # finite-lat-1 (64.34.82.77) — PRE-CUTOVER k3s reference only (superseded by infra/nixos/)
    lat2/      # finite-lat-2 (64.34.80.19) — now the CI runner box
    smoke/     # ovh-vps-smoke (15.204.56.61, OVH) — still hosts finite-brain (deferred)
    clawland/  # clawland-ovh (15.204.108.57, OVH) — legacy finite.vip fleet box
  images/      # container image definitions; built ONLY by CI, pushed digest-pinned to GHCR
  tinfoil/     # pins + notes for the public Tinfoil satellite repos (measured enclaves)
  runbooks/    # per-service: deploy, rollback, backup/restore, break-glass
```

`infra/nixos/` is the source of truth for lat1. The `hosts/lat1/` directory is
now **pre-cutover historical reference** (the k3s control plane it documents
was wiped in the cutover) — see the banner in `hosts/lat1/README.md`. The other
`hosts/<name>/` dirs still map the unit files, Caddyfiles, and compose files
that ARE those boxes' config, plus a captured-state appendix.

## Hosts and services (current topology, post-2026-07-09)

| Host | Role | Services |
|---|---|---|
| **lat1** (64.34.82.77) | **Consolidated NixOS app server** (`infra/nixos/`) | finite-saas-core (:4200), dashboard (podman :3000), **native** Postgres 16 (`services.postgresql`, `finite_core`, 87 FP keys), finitechat-server (:8788), finitesitesd (:8787), finite-search (SearXNG :8080 + Firecrawl), finite-saas-runner (Kata timer defined and enabled in Nix; live canary readiness still to be verified), **one** Caddy edge for `finite.computer` + `chat.finite.computer` + `*.finite.chat` + `*.docs.finite.chat`. NO k3s, NO Traefik, NO on-host image builds. Deploy: `nixos-rebuild --flake ...#finite-lat-1`. |
| **lat2** (64.34.80.19) | **CI runner box** (still Ubuntu+nix) | GitHub Actions runners: `finite-lat-2-mono` (against finite-mono) plus the 3 legacy-repo runners until those repos are archived (`hosts/lat2/runners.md`). finite-saas-sites / finite-search / finite-core-tunnel are **DISABLED** (migrated to lat1). |
| **smoke** (15.204.56.61) | Legacy Nix-fleet box; finite-brain | finite-brain on :3015 (`brain.smoke.finite.computer`), NixOS-generated systemd unit via the legacy repo. **DEFERRED** from the cutover — migrates with the auth-integration follow-up. |
| **clawland** (15.204.108.57) | Legacy finite.vip fleet box | Legacy `*.finite.vip` fleet (k3s + Traefik + oauth2-proxy, `finited`, ~50 agent namespaces). finitechat-server here is **DISABLED** (migrated to lat1). |
| Tinfoil | Measured enclaves (unchanged) | glm-5-2 inference + finite-private-limiter enclave; searxng enclave. The limiter validates usage against **lat1** Core. Deployed from the public satellite repos (`tinfoil/`). |
| Phala | hosted-agent CVMs | Confidential Runner fast-follow; not the internal production-canary path. |

## DNS (current)

- `finite.computer`, `chat.finite.computer` → **lat1** (Namecheap).
- `*.finite.chat` → **Cloudflare** (Full strict) → lat1 origin (Cloudflare
  Origin CA cert); `*.docs.finite.chat` same edge.
- `brain.finite.computer` record exists → lat1, but **brain isn't there yet**
  (deferred); `brain.smoke.finite.computer` → **smoke** is the working URL.
- `brain.smoke.finite.computer` / `*.smoke.finite.computer` → smoke.

## Secrets policy

**No secret values in this repo, ever.** This repo is public. Secrets live
where they run: on lat1, root-owned `/etc/finite/*.env` and
`/etc/finite-saas/` files (bootstrap checklist in `infra/nixos/README.md`);
Tinfoil sealed secrets; Phala sealed env; the legacy fleet's k8s Secrets on
smoke/clawland. Each host README documents which secrets each service needs —
variable **names** and where the value lives, never the value. If you find a
secret value committed here, rotate it first, then delete it.

## Images

First-party images are **built by CI**, tagged with the git SHA, pushed to
GHCR, and deployed by digest (`infra/images/`). On-host `podman build` (the old
lat1 k3s pattern) is gone: the confidential-compute company's control plane
does not run binaries built from "whatever was on the box." The lat1 dashboard
runs a digest-pinned image under podman; core and the sites/chat/brain binaries
are built by Nix from `infra/nixos/packages.nix`.

## Tinfoil satellite

Tinfoil enclaves are deployed from the public satellite repos, not from here —
`infra/tinfoil/` holds the pins and notes. The Finite Private limiter enclave
validates usage against lat1 Core (`FINITE_USAGE_API_SERVICE_KEY` pairs with
lat1's `FC_FINITE_PRIVATE_USAGE_API_TOKEN` — do NOT rotate at cutover).

## Deploy principles

1. **lat1 = `nixos-rebuild` from a release rev.** The rev that tagged the
   binaries is the rev the host runs. Rollback: `nixos-rebuild --rollback` on
   the host, or pin the previous rev. Source of truth: `infra/nixos/`; recovery:
   `infra/runbooks/lat1-nixos-reinstall.md`.
2. **Images are built by CI**, tagged with the git SHA, pushed to GHCR, and
   deployed by digest. No on-host builds.
3. **Binaries ship from release tags** (component-scoped: `finitechat/v*`,
   `fsite/v*`, `fbrain/v*`, `runtime-image/*`, `core/v*`).
4. **Deploy scripts / runbooks live here**, are idempotent, take an explicit
   ref/digest, and verify what they deployed (health endpoint reporting
   `source_commit`, like the finitechat server contract gate).
5. **Backups are only real once restored.** Before first-slice user data, every
   stateful service must have a service-consistent backup, an off-host copy, a
   restore runbook, and an empty-target restore drill. The current deployment
   does not satisfy this rule: lat1 is single-disk, the Borg target in
   `modules/backups.nix` is a placeholder, live SQLite directories are not yet
   converted into service-owned consistent snapshots, Agent Runtime `/data` is
   not covered, and the complete restore drill has not run. Paid-cohort launch
   is blocked on closing those gaps. A disk mirror (2 spare NVMes) remains
   defense in depth, not a backup.
