# infra/ — the single deploy root

Everything Finite runs in production is defined here. The north star:

> A release tag in finite-mono is sufficient, by itself, to reproduce any
> artifact we ship and to deploy any service we run. Nothing is built on a
> prod box. Nothing requires knowledge that lives only in someone's shell
> history.

## Current headline (2026-08-29)

**finite-lat-2 is the live app-plane host and runs the coupled cluster** (Core,
Postgres, chat, hosted-device, Sites, Brain, Identity, dashboard, Caddy,
backups — defined in `infra/nixos/`, per ADR 0007 and the 2026-08-29 emergency
cutover from lat1's thermal failure). **lat3 and lat4 are the active Agent
Runner (Kata) hosts.** lat1 is retired: it holds no production role, its
runner is leftover/inactive, and it is not deployment authority. DNS for
`finite.computer` and `*.finite.chat` points at lat2.

## Historical: the 2026-07-09 lat1 consolidation

The section below is a dated record of the earlier consolidation, kept for
history. It described the fleet before ADR 0007 moved the app plane to lat2;
do not read its present-tense statements as current topology.

**finite-lat-1 was the consolidated NixOS app server, and it ran the whole
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
  `finite-lat-1`, built by CI, and switched from an exact manifest-pinned
  system path. On-host `podman build` is gone;
  first-party images are CI-built and digest-pinned (`infra/images/`).

Still elsewhere: lat2 became the replacement single app server (ADR
0007, 2026-08-28 — emergency cutover for lat1's thermal failure). Its stale
Ubuntu data was skipped-by-decision; the box was wiped via provider reinstall
and reinstalled from the `Lat2 NixOS Closure` artifact with the service
stack minus any Agent Runner (`runbooks/lat2-replacement-cutover.md`). The
cutover completed 2026-08-29; lat2 now holds the production app plane.
**clawland** remains the legacy finite.vip fleet box; **Tinfoil** is unchanged.
The old FiniteBrain smoke service remains a rollback source, not the
production origin.

## First-cohort production baseline (2026-07-15)

The hosted-agent path is now in production with a proven fresh-Agent flow,
authenticated private Finite Sites Preview, Telegram pairing, and serial
digest-pinned upgrades of existing healthy Kata Agents. The exact deployed
checkpoint and future regression gates are recorded in
[`docs/runs/production-baseline-2026-07-15.md`](../docs/runs/production-baseline-2026-07-15.md).

`just deploy-lat2-closure ARTIFACT_DIR --activate` switches app-plane
infrastructure only. An Agent Runtime image rollout is a separate two-command
prepare/execute operation: it names an exact promoted artifact and either
explicit Project ids or `--roll-all` plus an already-target canary, then
requires the prepared plan hash before mutation. Never infer a bot rollout from
the word “deploy.”

Merged work not yet known to be released or deployed is tracked by surface in
[`deployment-queue.md`](deployment-queue.md). The queue is a handoff, not
production-mutation authority.

## Layout

```
infra/
  deployment-queue.md  # merged work awaiting a release/deploy/rollout
  nixos/       # THE NixOS fleet as code (lat2, lat3, lat4) — live definitions
  hosts/
    lat1/      # finite-lat-1 (64.34.82.77) — RETIRED; pre-cutover k3s reference only (superseded by infra/nixos/)
    lat2/      # finite-lat-2 (64.34.80.19) — historical captures from the pre-cutover repurposing; lat2 is now defined in infra/nixos/
    smoke/     # ovh-vps-smoke (15.204.56.61, OVH) — legacy Brain rollback source
    clawland/  # clawland-ovh (15.204.108.57, OVH) — legacy finite.vip fleet box
  images/      # container image definitions; built ONLY by CI, pushed digest-pinned to GHCR
  monitoring/  # external monitoring dashboards and NixOS receiver docs
  tinfoil/     # pins + notes for the public Tinfoil satellite repos (measured enclaves)
  runbooks/    # per-service: deploy, rollback, backup/restore, break-glass
```

`infra/nixos/` is the declared source of truth for the NixOS fleet (lat2,
lat3, lat4; lat1 is retired). Every
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
| **finite-lat-1** (64.34.82.77) | **RETIRED 2026-08-29 (thermal failure, ADR 0007) — leftover/inactive only.** Former consolidated app server and Kata Runner (`infra/nixos/` history); NixOS 25.11; single-disk root and `/data`. Its runner cannot lease relocations or creation work; do not address it for Core CLI, rollout, or closure activation. | Historical service stack (Core, Postgres, chat, hosted-device, Brain, Sites, dashboard, runner, Caddy) is dark. The physical disks hold stale 2026-07 metadata and may be touched only by a separately authorized reinstall. |
| **finite-lat-2** (64.34.80.19) | **Live app-plane host (ADR 0007).** Core, Postgres, chat, hosted-device, sites, Brain, Identity, dashboard, Caddy, backups, litestream. No Agent Runner. Recovery Authority and the `wg-finite` hub at `10.254.3.1`. | `finite.computer` DNS points here. Post-cutover cleanup (old runners, leftover credentials, stale smoke row) is still open. |
| **finite-lat-3** (207.188.7.157) | **NixOS 26.05 Agent Runner accepting new creation, hard limit 42.** Kernel 6.18.39; 187 GiB RAM; exact-size RAID1 root and `/data`; dual ESPs; 64-GiB swapfile plus zswap. | The Runner timer is enabled declaratively with `FC_RUNNER_DRAIN=false` and `FC_RUNNER_MAX_SANDBOXES=42`. This owner-authorized ceiling deliberately overcommits the declared 8-GiB guest maximum against physical RAM; swap is not counted as usable Agent capacity. No Recovery Authority exists here. |
| **finite-lat-4** (152.236.34.15) | **Second live NixOS Agent Runner** (same chassis class as lat3). RAID1 root and `/data`; `10.254.3.4` on `wg-finite`. | `FC_RUNNER_DRAIN=false`. Holds the relocated lat1 cohort (21 active Runtimes as of 2026-08-29). No Recovery Authority. |
| **smoke** (15.204.56.61) | Legacy Nix-fleet box; Brain rollback source | Legacy finite-brain on :3015 (`brain.smoke.finite.computer`). It is not a replica and must not be selected implicitly. |
| **clawland** (15.204.108.57) | Legacy finite.vip fleet box | Legacy `*.finite.vip` fleet (k3s + Traefik + oauth2-proxy, `finited`, ~50 agent namespaces). finitechat-server here is **DISABLED** (chat lives on the app-plane host, lat2). |
| Tinfoil | Measured enclaves | GLM-5.3-Flash inference + finite-private-limiter enclave on container `finite-private` (`v2026-08-28-glm-5-3-flash-5`, `acc651a6…`); searxng enclave. Admission is usage-api. Canonical model `glm-5-3-flash`; `deepseek-v4-flash-0731` and `glm-5-2` remain mixed-version request aliases. The historical `kimi-k2-6` hostname is retired. Deployed from the public satellite repos (`tinfoil/`). |

## DNS (current)

- `finite.computer`, `brain.finite.computer`, `chat.finite.computer` → **lat2** (`64.34.80.19`, Namecheap).
- `*.finite.chat` → **Cloudflare** (Full strict) → lat2 origin (Cloudflare
  Origin CA cert, served by lat2's Caddy since the 2026-08-29 cutover);
  `*.docs.finite.chat` same edge.
- `brain.finite.computer` is the canonical production Brain signing/API
  origin. The WorkOS-protected embedded client remains under
  `finite.computer/client`; its capability names the canonical Brain origin.
- `identity.finite.vip` is the canonical Finite Identity signing/API origin.
  Its exact record points at lat2 (`64.34.80.19`); the `finite.vip` apex and
  wildcard stay on the legacy fleet.
- `brain.smoke.finite.computer` / `*.smoke.finite.computer` → smoke, retained
  only as an explicit rollback target.

## Secrets policy

**No secret values in this repo, ever.** This repo is public. Secrets live
where they run: on each production host, root-owned `/etc/finite/*.env` and
`/etc/finite-saas/` files (bootstrap checklist in `infra/nixos/README.md`);
Tinfoil sealed secrets; Phala sealed env; the legacy fleet's k8s Secrets on
smoke/clawland. Each host README documents which secrets each service needs —
variable **names** and where the value lives, never the value. If you find a
secret value committed here, rotate it first, then delete it.

CI-only operational secrets live in Depot CI with the narrowest repository and
workflow scope that supports the lane. `CACHIX_AUTH_TOKEN` is the Cachix write
token for the `finite` binary cache used by CI Nix service package jobs and
production closure publication. The cache must remain readable without that
token for forked pull requests and production hosts to substitute from it.

## Images

First-party images are **built by CI**, tagged with the git SHA, pushed to
GHCR, and deployed by digest (`infra/images/`). On-host builds (the old k3s
pattern) are gone: the confidential-compute company's control plane
does not run binaries built from "whatever was on the box." The dashboard
runs a digest-pinned image under podman; core and the sites/chat/brain binaries
are built by Nix from `infra/nixos/packages.nix`.

## Tinfoil satellite

Tinfoil enclaves are deployed from the public satellite repos, not from here —
`infra/tinfoil/` holds the pins and notes. The Finite Private limiter enclave
validates usage against Core on the app-plane host (lat2;
`FINITE_USAGE_API_SERVICE_KEY` pairs with the host's
`FC_FINITE_PRIVATE_USAGE_API_TOKEN`).

## Deploy principles

1. **Each NixOS host = exact NixOS closure activation from a release rev.**
   The rev that
   tagged the binaries is the rev the host runs. CI builds the closure and
   packages or publishes trusted store paths before activating the recorded
   `SYSTEM` path. The active fleet uses host-specific helpers for lat2, lat3,
   and lat4; source of truth: `infra/nixos/`. Rollback:
   `nixos-rebuild --rollback` on the host, or pin the previous rev. The old
   bare-metal transcript in
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
    restore runbook, and an empty-target restore drill. The July deployment on
    lat1 did not satisfy this rule (it was
    single-disk); lat2 is now the app-plane host and its recovery boundary is
    tracked by the lat2 cutover records and `scripts/finite-status`. The
    Hosted Web Chat
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
