# ovh-vps-smoke — 15.204.56.61

**Host identity (correcting older docs):** ovh-vps-smoke is **15.204.56.61**
(OVH VPS, OpenStack KVM, NixOS 26.05 "Yarara", kernel 6.18.19, hostname
literally `ovh-vps-smoke`). The old inventory's claim that 15.204.108.57 is
this box is wrong — 15.204.108.57 is **clawland-ovh** (see
[`../clawland/`](../clawland/README.md)). DNS: `brain.smoke.finite.computer`
and the whole `*.smoke.finite.computer` wildcard resolve to 15.204.56.61.

**Who manages it:** the LEGACY `finitecomputer` repo's Nix fleet workspaces
(`workspaces/ovh-vps-smoke/`), deployed via `just host-deploy` (rsync repo →
host `/etc/nixos` → `nixos-rebuild switch --flake .#ovh-vps-smoke`). That repo
deliberately stays OUTSIDE finite-mono; this directory is **bridge
documentation** — it records what runs and how, and defines what mono will
eventually own. See [`deploy.md`](deploy.md).

All facts below captured read-only from the host on 2026-07-08.

> **Cutover status 2026-07-09 — finite-brain was DEFERRED from the lat1
> consolidation.** Brain still runs here, unchanged; it migrates to lat1 later,
> together with the oauth2-proxy / auth-integration follow-up. DNS note:
> `brain.finite.computer` now has a record pointing at **lat1**, but **brain is
> not there yet** — `brain.smoke.finite.computer` (→ this box) is the working
> URL. lat1's `infra/nixos/` already carries the brain + oauth2-proxy modules
> and port map (3015 / 4180) for when the migration happens.

## finite-brain (the service mono cares about)

finite-brain is **not** hand-run, and it listens on **3015 — not the 3025 some
runbooks claim** (SilverBullet on :3025 does not exist on this box or
clawland; no process, unit, pod, or config match — that cutover already
happened). It runs as a NixOS-generated systemd unit:

- Unit: `finite-brain-app.service` (enabled, `WantedBy=multi-user.target`),
  `Restart=always` / `RestartSec=3`, `DynamicUser=true`,
  `StateDirectory=finitebrain`, `NoNewPrivileges`, `PrivateTmp`,
  `ProtectSystem=full`. Captured verbatim in
  [`finite-brain-app.service`](finite-brain-app.service).
- ExecStart: `/nix/store/hsx3wyz3a00x48pfd1gpjqwhiawv5ijw-finite-brain-0.1.2-6466fcc/bin/finite-brain`
  (no args). Built by Nix (`rustPlatform.buildRustPackage`) from a vendored
  source tarball at rev `6466fcca389b1897771fa0a7c1cc5c6516e1d467`.
- Listens: `0.0.0.0:3015`.
- SQLite: `/var/lib/private/finitebrain/finite-brain.sqlite3` (2.3M at
  capture; `/var/lib/finitebrain` is the DynamicUser symlink to
  `private/finitebrain`).
- Config is entirely `Environment=` lines in the unit — **no
  EnvironmentFile, no secrets in the unit**. Names (values are plain config,
  visible in the unit copy):
  `FINITE_BRAIN_ADDR`, `FINITE_BRAIN_DB`, `FINITE_BRAIN_PUBLIC_BASE_URL`,
  `FINITE_BRAIN_SERVER_URL` (both `https://brain.smoke.finite.computer`),
  `FBRAIN_CONFIG_DIR` (`/var/lib/finitebrain/fbrain` — directory does not
  exist yet on the host).
- Generated from the legacy repo's
  `nix/modules/host-agent-cluster.nix` (`systemd.services.finite-brain-app`),
  enabled and parameterized by
  `workspaces/ovh-vps-smoke/agent-cluster/cluster.json` `.finite_brain`
  (`enabled: true`, `port: 3015`, `hostname: brain.smoke.finite.computer`,
  `require_oauth: true`, `oauth_email_domain: finite.vip`,
  `endpoint_ip: 15.204.56.61` — hardcoded).

Do not confuse with the k3s namespace `finite-brain-bot` (an agent workspace
pod with its own opencode ingressroute) — separate thing.

## Web edge

No Caddy, no nginx. The edge is:

```
Internet :80/:443
  → host socat bridges (fc-agent-cluster-http-bridge.service / -https-bridge.service,
    NixOS-generated, Restart=always): 0.0.0.0:80→127.0.0.1:30080, 0.0.0.0:443→127.0.0.1:30443
  → k3s Traefik NodePort svc kube-system/traefik (80:30080, 443:30443)
  → Traefik IngressRoute finite-brain/finite-brain-app
    (entryPoint websecure, tls.certResolver=letsencrypt — ACME HTTP-01,
     email paul@finite.vip, certs in traefik pod /data/acme.json)
  → Service finite-brain/finite-brain-app :3015 (ClusterIP, NO selector)
  → manual Endpoints object: 15.204.56.61:3015
  → back OUT of the cluster to the host systemd service
```

The IngressRoute (rendered by `host-agent-cluster.nix` as k3s Addon
`fc-finite-brain`) routes, in priority order:

| prio | match | backend | auth |
|---|---|---|---|
| 1000 | `Host(brain.smoke...) && Path(/health)` | finite-brain-app:3015 | none (expected) |
| 900 | `PathPrefix(/oauth2/)` | finite-brain-oauth2-proxy:4180 | — |
| 800 | `PathPrefix(/_admin)` | finite-brain-app:3015 | **NO oauth middleware** |
| 1 | everything else | finite-brain-app:3015 | oauth2-proxy forwardAuth |

oauth2-proxy (ns `fc-auth`, Google provider, `auth.smoke.finite.computer`,
allowed email domain `finite.vip`) gates everything EXCEPT `/health` and
`/_admin`. **Flag:** `/_admin` has no oauth middleware — presumably relies on
app-level auth; verify.

Other vhosts on this box (all `*.smoke.finite.computer`, same edge):
oauth2-proxy (auth.), dashboard, gitea (git.), matrix-synapse (matrix.),
finite-brain-bot opencode, and per-user agent/app namespaces (fire-finite,
paul-smoke, paul-with-key, skyler-smoke, smoke-studio).

## Deploy mechanism

Legacy repo: `nix/modules/host-agent-cluster.nix` + `nix/finite-brain.nix`
(vendored source tarballs in `nix/sources/`, ~27 of them Jun 30–Jul 8),
enabled via `workspaces/ovh-vps-smoke/agent-cluster/cluster.json`, deployed
with `just host-deploy`; **clawland-ovh (15.204.108.57) is the nix build
host**. On-host `/etc/nixos` is a git-init'd rsync mirror with only "sync"
commits and no remote; the operator-side rev pointer is
`/etc/nixos/.fc-source-rev` = `d954500eca4af6e197c1254058631dce4944b67f`.
Full flow and the mono target in [`deploy.md`](deploy.md).

## Risks (as captured 2026-07-08)

- **NO BACKUPS on this host.** No `fc-offsite-backup` timer, no borg, no cron
  (crontab not even installed). The legacy repo's offsite-borg module
  (`nix/modules/host-offsite-backup.nix`) exists but is gated on
  `workspaces/<ws>/host/backup.json` `{"enabled": true}` — no `backup.json`
  exists in `workspaces/ovh-vps-smoke/host/`. finite-brain's SQLite — the only
  copy — has no protected copy. Only manual pre-reset snapshots exist under
  `/var/lib/private/finitebrain/` (`*before-reset*`, `backups/`,
  `all-vault-reset-*`; ~9.3M total). Contrast: clawland has borg offsite
  backups active. The legacy repo's braindump lists this as known
  "Active Next Work #1".
- **`:3015` binds the public IP** (`0.0.0.0:3015` on 15.204.56.61). If the
  OVH network firewall does not block external :3015, direct
  `http://15.204.56.61:3015` skips oauth entirely. **Unverified** — only
  22/80/443 are the intended edge; also check k3s apiserver :6443/:10250.
- **Disk 82% full** (151G/196G on `/dev/sda2`); `nix/sources/` vendored
  tarballs keep growing on the operator side too.
- `/_admin` route bypasses oauth (see edge table above).
- Leftovers: `/root/result` → a manually `nix build`-built finite-brain 0.1.0
  (Jul 1); `/root/finitebrain-deploy-backups/finite-brain.nix.20260702T174828Z`;
  two Completed synapse pods in fc-matrix.
- No `~/.ssh/config` alias existed for 15.204.56.61 at capture time (the
  `ovh-rescue` alias points at clawland).

## Secrets (names/locations only — no values, ever)

- oauth2-proxy client-secret / cookie-secret: mounted in-pod from `/secrets`
  (k8s Secret in ns `fc-auth`). Google client_id
  `714116971392-1qk925pah8b7hhjr94magrtuh013bksn.apps.googleusercontent.com`
  is a public identifier, not a secret.
- finite-brain-app itself: no secret env vars — the unit's `Environment=`
  lines are plain config.
- Any app-level `/_admin` token lives in the app/DB, not in the unit
  (not captured).
