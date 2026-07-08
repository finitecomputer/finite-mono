# infra/ — the single deploy root

Everything Finite runs in production is defined here. The north star:

> A release tag in finite-mono is sufficient, by itself, to reproduce any
> artifact we ship and to deploy any service we run. Nothing is built on a
> prod box. Nothing requires knowledge that lives only in someone's shell
> history.

This tree records **what actually runs today** (captured read-only from the
hosts on 2026-07-08) and defines the deploy path forward. Rule one, from the
migration strategy: *defined as code where it already runs* — same boxes, same
mechanisms, now reproducible from one tree. No replatforming.

## Layout

```
infra/
  hosts/
    lat1/      # finite-lat-1 (64.34.82.77, Latitude.sh)
    lat2/      # finite-lat-2 (64.34.80.19, Latitude.sh)
    smoke/     # ovh-vps-smoke (15.204.56.61, OVH — NOT 15.204.108.57; that's clawland)
    clawland/  # clawland-ovh (15.204.108.57, OVH) — legacy fleet box, documented here ONLY for the finitechat server it currently hosts
  images/      # container image definitions; built ONLY by CI, pushed digest-pinned to GHCR
  tinfoil/     # pins + notes for the public Tinfoil satellite repos (measured enclaves)
  runbooks/    # per-service: deploy, rollback, backup/restore, break-glass
```

Each `hosts/<name>/` contains the unit files, Caddyfiles, k8s manifests, and
compose files that ARE the host's config, plus a `README.md` mapping every
service on the box and a captured-state appendix noting anything found on the
host that is not yet code.

## Hosts and services (as captured 2026-07-08)

| Host | Services |
|---|---|
| **lat1** | k3s (ns `finite-system`): finite-saas-core, dashboard, Postgres 16 + 6h pg_dump CronJob. Host: Caddy edge for `finite.computer` (no Traefik — k3s runs `--disable=traefik`), finite-saas-runner (systemd, 20s poll, Phala backend). |
| **lat2** | finite-sites / finitesitesd (systemd + Caddy + Kata/cloud-hypervisor microVMs) for `*.finite.chat`; finite-search (SearXNG + Firecrawl compose, loopback-only); finite-core-tunnel (SSH -L to lat1 core); **three** GitHub Actions runners (registered to finitechat, finitecomputer, finitecomputer-v2 — must be re-registered against finite-mono at cutover). |
| **smoke** | `*.smoke.finite.computer` incl. finite-brain — which is NOT hand-run: it's a NixOS-generated systemd unit deployed by the legacy finitecomputer repo's `just host-deploy` fleet flow (socat bridges → k3s Traefik → oauth2-proxy edge). Known gaps: no backups on this host; brain's :3015 binds the public IP. |
| **clawland** | Legacy finite.vip fleet box, managed by the legacy finitecomputer repo (deliberately outside mono). **Currently hosts the live finitechat server** (`chat.finite.computer` → 15.204.108.57, systemd `finitechat-server.service`, SQLite, borg-backed). Its target home is lat1 via the deploy script in `hosts/lat1/` — the 2026-07-07 lat1 deploy was rolled back after ~2 minutes. |
| Tinfoil | glm-5-2 inference + finite-private-limiter enclave; searxng enclave. Deployed from the public satellite repos (see `tinfoil/`). |
| Phala | hosted-agent CVMs, launched by the lat1 runner from `ghcr.io/finitecomputer/finite-agent-runtime`. |

## Secrets policy

**No secret values in this repo, ever.** This repo is public. Secrets live
where they run: k8s Secrets on lat1, root-owned env files on the hosts,
Tinfoil sealed secrets, Phala sealed env. Each host README documents which
secrets each service needs — variable **names** and where the value lives,
never the value. If you find a secret value committed here, rotate it first,
then delete it.

## Deploy principles

1. **Images are built by CI**, tagged with the git SHA, pushed to GHCR, and
   deployed by digest. On-host `podman build` (the old lat1 pattern) is
   deprecated: the confidential-compute company's control plane should not run
   binaries built from "whatever was on the box."
2. **Binaries ship from release tags** (component-scoped: `finitechat/v*`,
   `fsite/v*`, `fbrain/v*`, `runtime-image/*`, `core/v*`).
3. **Deploy scripts live here**, are idempotent, take an explicit
   ref/digest, and verify what they deployed (health endpoint reporting
   `source_commit`, like the finitechat server contract gate).
4. **Backups are only real once restored.** Every stateful service has a
   backup AND a restore runbook, and the restore has been drilled at least
   once. (Current reality is below that bar on every host — see the per-host
   READMEs and `runbooks/`.)
