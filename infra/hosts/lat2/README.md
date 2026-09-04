# finite-lat-2 — 64.34.80.19 (Latitude.sh)

> **ROLE CHANGE 2026-08-29 — lat2 is the live app-plane host.** ADR 0007 moved
> the production app-plane stack from thermally failed finite-lat-1 onto this
> host. Current deploy authority lives in
> [`infra/nixos/hosts/finite-lat-2`](../../nixos/hosts/finite-lat-2) and the
> ADR 0007 runbooks. The service files in this directory are a historical
> pre-NixOS capture from the old dedicated Sites/CI-runner box; do not use them
> to deploy or disable current production services.

This directory was captured read-only on 2026-07-08, before the old
sites/tunnel migration, Docker/Nix build migrations to Depot-backed CI, and the
ADR 0007 emergency cutover that made finite-lat-2 the replacement app server.

- Hardware: Supermicro AS-3015MR-H10TNR. Ubuntu 26.04 LTS, kernel
  7.0.0-15-generic, x86-64.
- Disks at the dated capture: /dev/md0 439G root (24% used); /dev/md1 1.8T at
  `/data` (essentially empty). `/data` is not an Agent placement or recovery
  target; the historical proposal in `backups.md` is superseded.
- Network: 64.34.80.19/31 + IPv6. Public exposure is exactly 22 (sshd) and
  80/443 (Caddy); everything else binds loopback. `/tmp` is a 94G tmpfs
  (matters for backups — see appendix).

## Services

| Service | What | Config in this tree |
|---|---|---|
| `finite-saas-sites.service` **(historical pre-NixOS unit)** | Current production Sites is NixOS-managed on finite-lat-2 and remains pinned to the released v1 API/docs/static daemon until a deliberate ADR 0028 cutover. The separate v2 validation host is `infra/nixos/hosts/finite-sites-v2/`. Data: `/var/lib/finite-sites`. | `systemd/finite-saas-sites.service` |
| `caddy.service` **(historical pre-NixOS unit)** | Current production Caddy is NixOS-managed on finite-lat-2. Canonical vhosts `api.finite.chat`, `*.finite.chat`, and `*.docs.finite.chat` still proxy to the canonical Sites daemon on 127.0.0.1:8787 until cutover. | `caddy/Caddyfile` |
| `finite-core-tunnel.service` **(historical, retired)** | Pre-NixOS SSH tunnel capture. Current Core is native on finite-lat-2; do not recreate the tunnel. | `systemd/finite-core-tunnel.service` |
| `finite-saas-runner.service` + `.timer` | **Previously undocumented, DORMANT.** "Finite agent creation runner": oneshot every 20s from the build-on-box checkout `/opt/finite/finitecomputer`. Timer is disabled and absent from `list-timers`. Stale `After=k3s.service` (no k3s here); depends on the core tunnel via drop-in. | `systemd/finite-saas-runner.service`, `.timer`, `systemd/finite-saas-runner-10-core-tunnel.conf` |
| GitHub Actions runners **(removal inventory)** | `finite-lat-2-mono` (registered against finite-mono) **plus** the 3 legacy-repo runners (v2.335.1, `User=ubuntu`, under `/srv/github-runner/`, registered to finitechat / finitecomputer / finitecomputer-v2). Current workflows use Depot and should not target these runners; they are to be unregistered during decommission. | `runners.md` |

## Ports

| Bind | Port | Process | Notes |
|---|---|---|---|
| 0.0.0.0 / [::] | 22 | sshd | public |
| * | 80, 443 | caddy | public; Cloudflare-proxied zone |
| 127.0.0.1 | 8787 | finitesitesd | all canonical Caddy vhosts proxy here |
| 127.0.0.1 | 14200 | ssh (finite-core-tunnel) | → lat1 ClusterIP 10.43.237.180:4200 |
| 127.0.0.1 | 2019 | caddy admin API | |
| 127.0.0.1 | 41943 | containerd | ephemeral |

## Secrets inventory (names and locations only — values live on the host)

| Location | Contents | Consumer |
|---|---|---|
| `/etc/finite-saas/sites.env` (0640) | exactly one var: `RESEND_API_KEY`. | finite-saas-sites.service |
| `/etc/finite-saas/certs/finite-chat-origin.pem` (0644 root:root) / `.key` (0640 root:caddy) | Cloudflare Origin CA cert pair for `finite.chat, *.finite.chat, docs.finite.chat, *.docs.finite.chat`; regenerated 2026-07-02 | Caddy |
| `/etc/finite-computer/runner.env` (0600 root) | 18 `FC_*` vars: `FC_CORE_URL`, `FC_CORE_API_TOKEN`, `FC_RUNNER_ID`, `FC_RUNNER_SOURCE_HOST_ID`, `FC_RUNNER_RELAY_URL`, `FC_RUNNER_RUNTIME_ARTIFACT_ID`, `FC_RUNNER_RUNTIME_ARTIFACT_KIND`, `FC_RUNNER_RUNTIME_ARTIFACT_REFERENCE`, `FC_RUNNER_RUNTIME_STATE_SCHEMA_VERSION`, `FC_RUNNER_WORK_ROOT`, `FC_RUNNER_MSB_BIN`, `FC_RUNNER_MSB_MEMORY`, `FC_RUNNER_MSB_CPUS`, `FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS`, `FC_RUNNER_RUNTIME_READY_INTERVAL_MS`, `FC_RUNNER_COMMAND_TIMEOUT_SECS`, `FC_RUNNER_RUNTIME_TEMPLATE_ROOT`, `FC_RUNNER_MAX_SANDBOXES` | finite-saas-runner.service (dormant) |
| `/var/lib/finite-sites/cookie-secret` (64 bytes) | finitesitesd session secret | finitesitesd |
| `/srv/github-runner/*/.credentials`, `.credentials_rsaparams` | runner registration credentials (never captured) | Actions runners |
| `/opt/finite/finitecomputer/secrets/` (0700 root) | unenumerated by design (root-only) | legacy runner tooling |
| `/home/ubuntu/smoke-identity.env` (0600, 90 bytes) | not read; contents unknown | unknown |

## Files here

- `systemd/` — unit files and env example. Deployed-vs-repo drift check on
  2026-07-08 was **byte-identical** for the pre-ADR Caddyfile,
  finite-saas-sites.service, kata.conf, finite-app@.service, and nerdctl
  sudoers. ADR 0028 removed the app-runner files from this tree; git history
  is the archive.
- `caddy/Caddyfile` — deployed at `/etc/caddy/Caddyfile`.
- `runners.md` — runner removal inventory for the lat2 decommission.
- `backups.md` — backup reality and the proposed timer.
- `deploy.md` — current (deprecated) manual sites deploy and the target flow.

## Captured-state appendix — on-host reality that is not (yet) code

1. **`/home/ubuntu/finite-sites` is an rsync'd source tree, not a git repo**
   ("fatal: not a git repository"). The v0.2.16 binaries at
   `/usr/local/bin/{finitesitesd,fsite}` (mtime 2026-07-03 15:01) were built
   from it on the box — **no commit provenance** for what is running.
   Previous binaries kept as `*.prev-20260619T155747Z`.
2. **`/opt/finite/finitecomputer`** — a second build-on-box Rust checkout
   (Cargo.lock, target/, finite-saas-runner/, msb-go-launcher/, deploy/,
   systemd/, tools/) plus a root-only `secrets/` dir (0700, unenumerated) and
   `/opt/finite/runtime-template`. Source of the dormant finite-saas-runner
   binary.
3. **Ad-hoc containers outside compose**, up 8–12 days at capture, leftover
   smoke/canary runs: 2× `finite-agent-remote-canary:run-2026063*/2026062*`
   and 3× `ghcr.io/finitecomputer/fc-tinfoil-agent-runtime` smoke11/12/13.
   Plus ~30 cached ghcr.io `finite-agent-runtime` /
   `finite-chat-hermes-runtime` image tags (CI artifacts).
4. **Microsandbox residue**: `~/.microsandbox`, `.bashrc`/`.profile`
   `.pre-microsandbox` backups, `FC_RUNNER_MSB_*` vars in runner.env, and a
   `finite_sites_pre_msb_cleanup_20260617T213015Z.tar.gz` in
   `/var/backups/finite-cleanup/` — partial cleanup of an earlier
   MicroSandbox experiment.
5. **BACKUP GAP**: no cron (crontab binary not installed), no backup timers,
   no backup scripts anywhere on the box. Newest **durable** backup of
   `/var/lib/finite-sites` is `/var/backups/finite-sites/finite-sites-20260617T215714Z.tar.gz`
   (2026-06-17). A newer tarball,
   `/tmp/finite-sites-20260702T145453Z.tar.gz` (2026-07-02), sits in `/tmp`
   — **a tmpfs; it evaporates on reboot**. `/data` (1.8T) is empty. See
   `backups.md`.
6. `/etc/sudoers.d/finite-sites` (systemctl start/stop/restart/is-active for
   `finite-app@*`) existed on the host as app-runner residue. ADR 0028 removes
   the Sites app-runner path; do not recreate the sudoers or polkit files.
7. Runner labels were recovered from config-time `_diag` logs; the
   authoritative label list lives on the GitHub side. Firecrawl compose
   reports one service `exited(1)` (identity not chased). No kata runtime
   section in `/etc/containerd` config (kata wired via the
   `containerd-shim-kata-v2` symlink).
