# infra/ — the single deploy root

Everything Finite runs in production is defined here. The north star:

> A release tag in finite-mono is sufficient, by itself, to reproduce any
> artifact we ship and to deploy any service we run. Nothing is built on a
> prod box. Nothing requires knowledge that lives only in someone's shell
> history.

## Post-cutover headline (2026-07-09)

**finite-lat-1 is now the consolidated NixOS app server, and it runs the whole
coupled cluster.** Its definition is `infra/nixos/` (host `finite-lat-1`); the
2026-07-09 reinstall transcript is
`infra/runbooks/lat1-nixos-reinstall.md`, but destructive reuse is paused while
the finite-lat-3 capacity/redundancy plan produces a recovery-proved
replacement.

What the 2026-07-09 lat1 consolidation cutover changed:

- **One app server, one config tree.** finite-saas-core, Finite Identity, dashboard,
  finitechat-server (migrated off clawland), finitesitesd (migrated off lat2),
  and Postgres all run on lat1, defined in `infra/nixos/`.
- **Native Postgres.** Postgres 16 is a `services.postgresql` systemd service
  (db `finite_core`, 87 Finite Private keys) — **no more k3s StatefulSet**.
- **One Caddy edge.** A single Caddy on lat1 fronts `finite.computer`,
  `brain.finite.computer`, `chat.finite.computer`, `*.finite.chat` (Cloudflare Origin CA), and
  `*.docs.finite.chat`; the exact `identity.finite.vip` record joins it when
  the Identity Authority deployment is accepted. **No Traefik, no k3s, no
  socat bridges.**
- **`nixos-rebuild` is THE deploy.** Deploying a release pins the flake to the
  rev that tagged the binaries. The old *six distinct deploy mechanisms* — k3s
  `kubectl apply` + on-host `podman build` (lat1), systemd + Kata (lat2), Nix
  fleet `just host-deploy` (smoke/clawland), and the hand-run finitechat script
  — are **resolved for the coupled cluster**: one NixOS closure for
  `finite-lat-1`, built by CI, published to the `finite` Cachix cache, and
  switched by `scripts/deploy-lat1-closure-cache` from an exact manifest-pinned
  system path. On-host `podman build` is gone;
  first-party images are CI-built and digest-pinned (`infra/images/`).

Still elsewhere: lat2 is becoming the replacement single app server (ADR
0007, 2026-08-28 — emergency cutover for lat1's thermal failure). Its stale
Ubuntu data is skipped-by-decision; the box is wiped via provider reinstall
and reinstalled from the `Lat2 NixOS Closure` artifact with lat1's service
stack minus any Agent Runner (`runbooks/lat2-replacement-cutover.md`). Until
the cutover completes it holds no production role.
**clawland** remains the legacy finite.vip fleet box; **Tinfoil** is unchanged.
The old FiniteBrain smoke service remains a rollback source, not the
production origin.

## First-cohort production baseline (2026-07-15)

The hosted-agent path is now in production with a proven fresh-Agent flow,
authenticated private Finite Sites Preview, Telegram pairing, and serial
digest-pinned upgrades of existing healthy Kata Agents. The exact deployed
checkpoint and future regression gates are recorded in
[`docs/runs/production-baseline-2026-07-15.md`](../docs/runs/production-baseline-2026-07-15.md).

`scripts/deploy-lat1-closure-cache ARTIFACT_DIR` switches infrastructure only.
An Agent Runtime image rollout is a separate two-command prepare/execute
operation: it names an exact promoted artifact and either explicit Project ids
or `--roll-all` plus an already-target canary, then requires the prepared plan
hash before mutation. Never infer a bot rollout from the word “deploy.”

Merged work not yet known to be released or deployed is tracked by surface in
[`deployment-queue.md`](deployment-queue.md). The queue is a handoff, not
production-mutation authority.

## Layout

```
infra/
  deployment-queue.md  # merged work awaiting a release/deploy/rollout
  nixos/       # finite-lat-1 AS CODE — the live definition of the app server
  hosts/
    lat1/      # finite-lat-1 (64.34.82.77) — PRE-CUTOVER k3s reference only (superseded by infra/nixos/)
    lat2/      # finite-lat-2 (64.34.80.19) — historical captures; box being repurposed as the replacement app server (ADR 0007)
    smoke/     # ovh-vps-smoke (15.204.56.61, OVH) — legacy Brain rollback source
    clawland/  # clawland-ovh (15.204.108.57, OVH) — legacy finite.vip fleet box
  images/      # container image definitions; built ONLY by CI, pushed digest-pinned to GHCR
  monitoring/  # external monitoring dashboards and NixOS receiver docs
  tinfoil/     # pins + notes for the public Tinfoil satellite repos (measured enclaves)
  runbooks/    # per-service: deploy, rollback, backup/restore, break-glass
```

`infra/nixos/` is the declared source of truth for lat1. Every
`infra/hosts/<name>/` directory is a dated capture or migration record unless
its own banner explicitly says otherwise; it is not permission to deploy its
old units. `hosts/lat1/` describes the wiped pre-cutover k3s control plane, and
the Sites/Search material under `hosts/lat2/` is historical. The runner
inventory in `hosts/lat2/runners.md` is removal inventory for
`runbooks/decommission-lat2.md`; mono Docker/image CI is no longer scheduled
there.

## Hosts and services (observed topology, 2026-07-20)

This table is current-state authority, not a desired topology. A provider
server that is provisioning or under qualification is not deployed Finite
capacity. The one accepted next candidate and its hard gates live in
[`docs/runs/finite-lat-capacity-and-redundancy.md`](../docs/runs/finite-lat-capacity-and-redundancy.md).

| Host | Role | Services |
|---|---|---|
| **finite-lat-1** (64.34.82.77) | **Consolidated NixOS app server and existing-Agent Kata Runner** (`infra/nixos/`). NixOS 25.11; single-disk root and `/data`; no swap at the 2026-07-18 inventory. New creation is drained; the Runner timer remains active for existing-Agent lifecycle work. The private lat3 WireGuard path, peer-scoped firewall, Core socket proxy, and multi-Runner Core are active declarative configuration from merged PR #134; no runtime bridge override remains. | finite-saas-core (:4200), dashboard (podman :3000), **native** Postgres 16 (`services.postgresql`, `finite_core`, 87 FP keys), finitechat-server (:8788), finitechat-hosted-device (loopback only, per-WorkOS-user identity and encrypted store), FiniteBrain (:3015), finitesitesd (:8787), finite-saas-runner (Kata), a separately fenced **dark/disabled** Phala API worker definition, and **one** Caddy edge. NO k3s, NO Traefik, NO on-host image builds. Deploy: CI-built closure published to Cachix, metadata artifact `lat1-nixos-closure-REV`, and exact-path activation by `scripts/deploy-lat1-closure-cache`. |
| **finite-lat-2** (64.34.80.19) | **Emergency replacement app server (ADR 0007, in progress).** Currently Ubuntu 26.04+nix; the stale-data archive is skipped by owner decision and the box is wiped via provider reinstall. It re-installs with lat1's service stack — Core, Postgres, chat, hosted-device, sites, Brain, Identity, dashboard, Caddy, backups, litestream — minus any Agent Runner, boots in declarative import mode, and inherits the Recovery Authority role and the `wg-finite` hub at `10.254.3.1`. | Until Gate E of `runbooks/lat2-replacement-cutover.md` completes: no production service, CI, build, deploy, capacity, recovery, or archive authority. The installed GitHub runners on the old Ubuntu box are removal inventory (`hosts/lat2/runners.md`). |
| **finite-lat-3** (207.188.7.157) | **NixOS 26.05 Agent Runner accepting new creation, hard limit 42.** Kernel 6.18.39; 187 GiB RAM; exact-size RAID1 root and `/data`; dual ESPs; 64-GiB swapfile plus zswap. | The Runner timer is enabled declaratively with `FC_RUNNER_DRAIN=false` and `FC_RUNNER_MAX_SANDBOXES=42`. This owner-authorized ceiling deliberately overcommits the declared 8-GiB guest maximum against physical RAM; swap is not counted as usable Agent capacity. No Recovery Authority exists here. |
| **finite-lat-4** (152.236.34.15) | **Third NixOS Agent Runner host (ADR 0007 model, in progress).** Same chassis class as lat3 (Supermicro AS-3015MR-H10TNR, EPYC 4564P, 187 GiB RAM); exact-size RAID1 root and `/data` (same geometry, fresh captured identities); dual ESPs; 64-GiB swapfile plus zswap. Currently an interim Ubuntu install pending the artifact-driven NixOS reinstall. | Host config drafted in `infra/nixos/hosts/finite-lat-4/`; install and drained bring-up run only through `runbooks/lat4-nixos-runner-install.md` (Gates A–E). Admits **drained**; the undrain decision is a separate owner approval. Takes the `10.254.3.4` WireGuard address after the PR #715 /29 widening merges. Deploy: CI-built `lat4-nixos-closure-REV` artifact switched by `scripts/deploy-lat4-closure-cache`. No Recovery Authority here unless decided otherwise. |
| **smoke** (15.204.56.61) | Legacy Nix-fleet box; Brain rollback source | Legacy finite-brain on :3015 (`brain.smoke.finite.computer`). It is not a replica and must not be selected implicitly. |
| **clawland** (15.204.108.57) | Legacy finite.vip fleet box | Legacy `*.finite.vip` fleet (k3s + Traefik + oauth2-proxy, `finited`, ~50 agent namespaces). finitechat-server here is **DISABLED** (migrated to lat1). |
| Tinfoil | Measured enclaves | GLM-5.3-Flash inference + finite-private-limiter enclave on container `finite-private` (`v2026-08-28-glm-5-3-flash-4`); searxng enclave. Canonical model `glm-5-3-flash`; `deepseek-v4-flash-0731` and `glm-5-2` remain mixed-version request aliases. The historical `kimi-k2-6` hostname is retired. Deployed from the public satellite repos (`tinfoil/`). |

## DNS (current)

- `finite.computer`, `brain.finite.computer`, `chat.finite.computer` → **lat1** (Namecheap).
- `*.finite.chat` → **Cloudflare** (Full strict) → lat1 origin (Cloudflare
  Origin CA cert); `*.docs.finite.chat` same edge.
- `brain.finite.computer` is the canonical production Brain signing/API
  origin. The WorkOS-protected embedded client remains under
  `finite.computer/client`; its capability names the canonical Brain origin.
- `identity.finite.vip` is the canonical Finite Identity signing/API origin.
  Only its exact record moves to lat1; the `finite.vip` apex and wildcard stay
  on the legacy fleet.
- `brain.smoke.finite.computer` / `*.smoke.finite.computer` → smoke, retained
  only as an explicit rollback target.

## Secrets policy

**No secret values in this repo, ever.** This repo is public. Secrets live
where they run: on lat1, root-owned `/etc/finite/*.env` and
`/etc/finite-saas/` files (bootstrap checklist in `infra/nixos/README.md`);
Tinfoil sealed secrets; Phala sealed env; the legacy fleet's k8s Secrets on
smoke/clawland. Each host README documents which secrets each service needs —
variable **names** and where the value lives, never the value. If you find a
secret value committed here, rotate it first, then delete it.

CI-only operational secrets live as GitHub Actions repository or organization
secrets. `CACHIX_AUTH_TOKEN` is the Cachix write token for the `finite` binary
cache used by CI Nix service package jobs and lat1 production closure
publication. The cache must remain readable without that token for forked pull
requests and production hosts to substitute from it.

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

1. **lat1 = exact NixOS closure activation from a release rev.** The rev that
   tagged the binaries is the rev the host runs. CI builds the closure, pushes
   it to the `finite` Cachix cache, and the host substitutes only trusted store
   paths before activating the recorded `SYSTEM` path. Rollback:
   `nixos-rebuild --rollback` on the host, or pin the previous rev. Source of
   truth: `infra/nixos/`. The old bare-metal transcript in
   `infra/runbooks/lat1-nixos-reinstall.md` is historical and not current wipe
   authority.
2. **Images are built by CI**, tagged with the git SHA, pushed to GHCR, and
   deployed by digest. No on-host builds.
3. **Binaries ship from release tags** (component-scoped: `finitechat/v*`,
   `fsite/v*`, `fbrain/v*`, `runtime-image/*`, `core/v*`).
4. **Deploy scripts / runbooks live here**, are idempotent, take an explicit
   ref/digest, and verify what they deployed (health endpoint reporting
   an automatically derived artifact fingerprint, like the finitechat server
   contract gate).
5. **Backups are only real once restored.** Before first-slice user data, every
   stateful service must have a service-consistent backup, an off-host copy, a
   restore runbook, and an empty-target restore drill. The current deployment
   does not yet satisfy this rule: lat1 is single-disk. The Hosted Web Chat
   module creates a service-consistent snapshot only when a deploy or operator
   triggers it; its disruptive 15-minute timer was removed after it broke live
   streams. Snapshot health currently tolerates seven days, and Borg ships the
   latest snapshot daily to the dedicated rsync.net repository. A verified
   first archive exists, but this is not a 15-minute RPO. Destination-side
   append-only restriction is recommended hardening; a non-disruptive cadence
   and complete empty-target restore remain known gaps. On 2026-07-20 Paul
   explicitly waived them as prerequisites for opening lat3 at a hard limit of
   32 Agents.
   Agent Runtime `/data` is not covered.
   The July 13 first-cohort Stripe exception remains history. No new Core/UI
   admission gate was deployed for the July 20 lat3 opening; the enforced
   bound is the Runner's 32-sandbox maximum. The matching lat1 disks contain
   stale metadata from the failed 2026-07-09 MD install; they are not clean spares
   and may be touched only by a serial-stable, separately authorized reinstall.
   A future mirror remains defense in depth, not a backup.
