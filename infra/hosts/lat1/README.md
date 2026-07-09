# finite-lat-1 (lat1)

> **SUPERSEDED 2026-07-09 — lat1 is now NixOS.** Its live config is
> **`infra/nixos/`** (host `finite-lat-1`); reinstall/recovery procedure is
> `infra/runbooks/lat1-nixos-reinstall.md`. The k8s manifests, systemd units,
> and Caddyfile in **this** directory are **PRE-CUTOVER historical reference**:
> the single-node k3s control plane they describe was **wiped** in the 2026-07-09
> consolidation cutover. lat1 now runs everything natively — finite-saas-core,
> dashboard (podman), **native** Postgres 16, finitechat-server (:8788),
> finitesitesd (:8787), finite-search, and **one** Caddy edge — with **no k3s,
> no Traefik, and no on-host podman builds**. Nothing below is current; it is
> kept as provenance for how the box ran before the cutover. Deploy is now
> `nixos-rebuild --flake ...#finite-lat-1`, not `kubectl apply`.

---

**Historical (pre-cutover) map** of host `finite-lat-1` — 64.34.82.77,
Latitude.sh. Everything here is traceable to the read-only capture of
2026-07-08 or to the files in this directory. Secrets: **names only** (this repo
is public). **This describes the wiped k3s host, not the current NixOS box —
see `infra/nixos/`.**

## Hardware / OS

- Supermicro AS-3015MR-H10TNR bare metal, 188G RAM.
- md RAID: `md0` 439G root (18% used), `md1` 1.8T at `/data` — `/data` is
  essentially empty.
- Ubuntu 26.04 LTS, kernel 7.0.0-15-generic.
- k3s v1.35.5+k3s1, single node, started with `--disable=traefik
  --disable=servicelb --write-kubeconfig-mode=644` (no other flags, no
  `/etc/rancher/k3s/config.yaml`). Kubeconfig is world-readable by design:
  any local user is cluster-admin.
- No cron on the host (`crontab` binary absent). All scheduling is systemd
  timers (host) plus one k8s CronJob.

## Services

### In k3s (namespace `finite-system`, manifests in `k8s/`)

| Workload | Kind | Image (live 2026-07-08) | Notes |
|---|---|---|---|
| finite-saas-core | Deployment, 1 replica | `localhost/finite-saas-core:6138718` | :4200, ClusterIP Service 10.43.237.180. Relay state on PVC `finite-saas-core-relay-state` (5Gi local-path). Init container waits for Postgres. |
| finite-dashboard | Deployment, 1 replica | `localhost/finite-saas-dashboard:b4982e9` | :3000, NodePort Service 30080. Runtime mode `canary` (ConfigMap `FC_DASHBOARD_RUNTIME_MODE`). |
| finite-core-postgres | StatefulSet, 1 replica | `postgres:16-alpine` | db `finite_core`, user `finite`. PVC `postgres-data-finite-core-postgres-0` (20Gi local-path). |
| finite-core-postgres-backup | CronJob `17 */6 * * *` | `postgres:16-alpine` | `pg_dump --format=custom --file=/backups/finite_core_latest.dump` onto PVC `finite-core-postgres-backups` (20Gi local-path). Same filename every run — see appendix. |

Config: ConfigMap `finite-computer-config` (8 keys, no credentials — admin
emails, base URLs, a public Stripe price id). Secrets: k8s Secret
`finite-computer-secrets` (inventory below).

### On the host (systemd)

| Unit | What | Notes |
|---|---|---|
| `caddy.service` | Edge for `finite.computer` on :80/:443 | Stock Ubuntu package unit, no drop-ins. Config: `/etc/caddy/Caddyfile` (copy in `caddy/Caddyfile`). **There is no Traefik on this host** — k3s disables it; earlier inventories claiming Traefik are wrong. No Ingress/IngressRoute objects exist in the cluster. |
| `finite-saas-runner.service` + `.timer` | Finite agent-creation runner | Oneshot `run-once`, User=ubuntu, After=k3s. Timer: `OnBootSec=30s`, **`OnUnitInactiveSec=20s`**, `AccuracySec=1s` — a 20-second polling loop dressed as a timer, not a cron-style schedule. Env: `/etc/finite-computer/runner.env`. Backend is **Phala Cloud** (launches CVMs from `ghcr.io/finitecomputer/finite-agent-runtime`). Binary: `/opt/finite/finitecomputer-v2/target/release/finite-saas-runner` — see appendix for the provenance problem. Units in `systemd/`. |
| `k3s.service` | Single-node Kubernetes | Flags above. |

## Network / ports

From `ss -tlnp` (capture):

| Port | Bind | Process |
|---|---|---|
| 22 | 0.0.0.0 / :: | sshd |
| 80, 443 | * | caddy (edge) |
| 2019 | 127.0.0.1 | caddy admin API |
| 6443 | * | k3s apiserver |
| 10250 | * | kubelet |
| 30080 | (NodePort) | finite-dashboard Service |
| various 102xx, 6444, 10010 | 127.0.0.1 | k3s internals / containerd |

Nothing listens on :8787 (finitechat) — see appendix.

## How finite.computer routes

DNS A `finite.computer` → 64.34.82.77 (this host). Host Caddy terminates TLS:

1. `/internal/finite-private/*` → `reverse_proxy 10.43.237.180:4200` — the
   **hardcoded ClusterIP** of Service `finite-system/finite-saas-core`.
   FRAGILE: it works only because Caddy runs on the single k3s node and can
   reach the service CIDR, and it breaks silently if the Service is ever
   recreated (ClusterIPs are not stable). Fix candidates: NodePort for core,
   or resolve via kube DNS/stable VIP. The same IP is also baked into
   `systemd/runner.env.example` (`FC_CORE_URL`).
2. everything else → `127.0.0.1:30080` — the finite-dashboard NodePort.

`chat.finite.computer` is **not** served here: DNS points at 15.204.108.57
(clawland), and this host has no Caddy vhost or listener for it.

## Secrets inventory (names only — values live on the host/cluster)

### k8s Secret `finite-computer-secrets` (ns `finite-system`) — 10 keys, live

`FC_CORE_API_TOKEN`, `FC_FINITE_PRIVATE_USAGE_API_TOKEN`,
`GOOGLE_WORKSPACE_CLIENT_ID`, `GOOGLE_WORKSPACE_CLIENT_SECRET`,
`POSTGRES_PASSWORD`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`,
`WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `WORKOS_COOKIE_PASSWORD`

Consumed by core, dashboard, postgres, and the backup CronJob (see `k8s/`).
The dashboard manifest additionally references optional keys
`FC_RELAY_ADMIN_TOKEN` and `FC_RELAY_HOST_ENDPOINTS_JSON` that do not exist
in the live secret (marked `optional: true`, so pods start without them).
There is **no Secret manifest in this repo** — the secret was created/updated
imperatively with kubectl.

### `/etc/finite-computer/runner.env` (root:ubuntu 0640, mtime Jul 6) — runner service

`FC_CORE_URL`, `FC_CORE_API_TOKEN`, `FC_RUNNER_ID`,
`FC_RUNNER_SOURCE_HOST_ID`, `FC_RUNNER_BACKEND`,
`FC_RUNNER_RUNTIME_ARTIFACT_ID`, `FC_RUNNER_FINITE_PRIVATE_BASE_URL`,
`FC_RUNNER_FINITE_PRIVATE_MODEL`, `PHALA_CLOUD_API_KEY`,
`FC_RUNNER_PHALA_BIN`, `FC_RUNNER_PHALA_INSTANCE_TYPE`,
`FC_RUNNER_PHALA_DISK_SIZE`, `FC_RUNNER_PHALA_KMS`,
`FC_RUNNER_PHALA_PUBLIC_LOGS`, `FC_RUNNER_PHALA_PUBLIC_SYSINFO`,
`FC_RUNNER_WORK_ROOT`, `FC_RUNNER_DRAIN`, `FC_RUNNER_MAX_SANDBOXES`,
`FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS`, `FC_RUNNER_RUNTIME_READY_INTERVAL_MS`,
`FC_RUNNER_LAUNCH_TIMEOUT_SECS`, `FC_RUNNER_COMMAND_TIMEOUT_SECS`

Template with the same names: `systemd/runner.env.example`.

### `/etc/finite-computer/deploy.env` (root:ubuntu 0640) — 5 keys, STALE

`POSTGRES_PASSWORD`, `FC_CORE_API_TOKEN`, `WORKOS_API_KEY`,
`WORKOS_CLIENT_ID`, `WORKOS_COOKIE_PASSWORD`

Not referenced by any systemd unit; presumably the seed for
`finite-computer-secrets`. It has **drifted behind** the live secret — see
appendix. A sibling backup `deploy.env.pre-core-relay-routing-20260525`
(root-only, contains secret values) also sits there.

## Files in this directory

- `k8s/` — the finite-system manifests (moved from
  `finitecomputer-v2/deploy/finite-computer/k8s/`). Kept as-committed; live
  drift is recorded in the appendix, not silently rewritten.
- `caddy/Caddyfile` — verbatim copy of `/etc/caddy/Caddyfile`.
- `systemd/` — `finite-saas-runner.service` + `.timer` (moved from
  `finitecomputer-v2/deploy/finite-computer/systemd/`; line-identical to the
  units captured from `/etc/systemd/system/`) and `runner.env.example`.
- `scripts/deploy-finitechat-server.sh` — the finitechat server's FUTURE lat1
  deploy path (moved from
  `finitecomputer-v2/scripts/deploy_finitechat_server_lat1.sh`); the live
  server is on clawland today. See the script header for host mismatches.
- `deploy.md` — how deploys work today (deprecated on-host podman flow) and
  the target CI/GHCR flow.

## Captured-state appendix — divergences (live vs repo), 2026-07-08

1. **Image refs.** Repo manifests pin `localhost/finite-saas-core:dev` and
   `localhost/finite-dashboard:dev`; live runs
   `localhost/finite-saas-core:6138718` and
   `localhost/finite-saas-dashboard:b4982e9`. Deploys `kubectl apply` the
   manifest then retag/`kubectl set image` per build, so the manifests' `:dev`
   tags never match production. Manifests kept as-is; the target flow
   (deploy.md) replaces this with digest-pinned GHCR refs.
2. **Dashboard image rename.** The live deployment runs
   `localhost/finite-saas-dashboard:b4982e9` (new repo name, single tag) while
   ~40 historical tags sit under the old name `localhost/finite-dashboard`
   (plus ~20 of `finite-saas-core`) in both podman and k3s containerd —
   multi-GB of on-host build cruft, no pruning.
3. **Secret drift.** Live `finite-computer-secrets` has 10 keys; the on-disk
   seed `/etc/finite-computer/deploy.env` has only 5 (missing
   `FC_FINITE_PRIVATE_USAGE_API_TOKEN`, `GOOGLE_WORKSPACE_CLIENT_ID`,
   `GOOGLE_WORKSPACE_CLIENT_SECRET`, `STRIPE_SECRET_KEY`,
   `STRIPE_WEBHOOK_SECRET`). The secret was evidently updated via kubectl
   without updating the env file. Re-running any deploy.env-based secret
   creation would drop the 5 newer keys.
4. **Runner binary provenance.** `/opt/finite/finitecomputer-v2` is NOT a git
   repo: it is owned by uid/gid `501:staff` (macOS ids), contains a
   `.DS_Store`, and was rsync'd from someone's Mac. The production
   `finite-saas-runner` binary is executed from its `target/release/`. No
   repo/SHA provenance is recoverable from the host (Cargo.lock mtime Jul 6).
5. **Root-only old env backup.** `/opt/finite/remote-backups/runner.env-20260526T163029Z`
   — a root-only copy of an old runner.env containing secret values, sitting
   outside `/etc`. Flagged at capture, not read. Should be shredded or moved
   under `/etc/finite-computer/` with the same permissions discipline.
6. **Backup reality.** The pg_dump CronJob writes the SAME file
   (`finite_core_latest.dump`, ~1MB) every 6h. Retention = one snapshot, max
   6h old; no rotation, no timestamps, no off-host copy. The backups PVC, the
   Postgres data PVC, and the OS all live on the same `md0` filesystem
   (local-path provisioner). One ad-hoc manual dump also exists at
   `/opt/finite/backups/finitecomputer-v2/pre-cleanup-20260703T0029Z.dump`
   (Jul 3). A restore has never been drilled.
7. **Rolled-back finitechat deploy (2026-07-07).** A finitechat-server was
   deployed 18:06 UTC via a script piped over SSH stdin (repo
   `finitecomputer/finitechat.git` @ `6cc74b58d504…`), ran ~2 min listening on
   `10.42.0.1:8787`, then was stopped, runtime-disabled, and its transient
   unit deleted at 18:09. Leftovers on lat1: `finite-chat` user (UID 986,
   dangling NixOS nologin shell) + group, `/var/lib/finite-chat` with the
   release binary, a src clone at the pinned SHA, and a tiny SQLite DB whose
   197KB WAL was never checkpointed (checkpoint before archiving if that data
   matters). The k8s Service `fc-chat/finitechat-server` created during the
   deploy is gone; the namespace no longer exists. Live
   `chat.finite.computer` runs on **clawland (15.204.108.57)**. The exact
   piped script was not recoverable from the host; the in-repo copy
   (`scripts/deploy-finitechat-server.sh`) uses `nix shell` and Traefik
   IngressRoutes, neither of which exists on lat1, so the two diverge.
8. **Hardcoded ClusterIP, twice.** `/etc/caddy/Caddyfile` and
   `runner.env.example`'s `FC_CORE_URL` both bake in `10.43.237.180`.
9. **systemd units.** In-repo `finite-saas-runner.service`/`.timer` match the
   captured `/etc/systemd/system/` units line-for-line — no drift.
   `runner.env.example` covers all 22 live runner.env names; live sets
   `FC_RUNNER_MAX_SANDBOXES` (commented in the example) and does not set the
   optional `FC_RUNNER_PHALA_REGION`.
10. **ConfigMap.** `k8s/configmap.yaml` matches the live
    `finite-computer-config` key-for-key (8 keys, values identical).
11. **Older siblings under /opt/finite** (dead weight, not code):
    `finitechat/` (May 23 copy, no .git), `finitecomputer/` (May 29),
    `bin/finite-saas-core` (May 25 host binary, likely dead), `deps/`,
    `runtime-build/`, `runtime-template/`.
