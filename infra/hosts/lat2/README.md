# finite-lat-2 — 64.34.80.19 (Latitude.sh)

Dedicated sites + search + CI-runner box. Captured read-only 2026-07-08.

- Hardware: Supermicro AS-3015MR-H10TNR. Ubuntu 26.04 LTS, kernel
  7.0.0-15-generic, x86-64.
- Disks: /dev/md0 439G root (24% used); /dev/md1 1.8T at `/data`
  (essentially empty — earmarked as the backup target, see `backups.md`).
- Network: 64.34.80.19/31 + IPv6. Public exposure is exactly 22 (sshd) and
  80/443 (Caddy); everything else binds loopback. `/tmp` is a 94G tmpfs
  (matters for backups — see appendix).

## Services

| Service | What | Config in this tree |
|---|---|---|
| `finite-saas-sites.service` | finitesitesd **v0.2.16** (`/usr/local/bin/finitesitesd`, fsite v0.2.16 alongside). Registry, publishing API, Git smart HTTP, and site serving for `*.finite.chat` on 127.0.0.1:8787. `--app-runner kata`: tier-2 apps run as **Kata Containers 3.31.0 microVMs on cloud-hypervisor** (`[hypervisor.clh]`, `/etc/kata-containers/configuration.toml`), driven via `sudo nerdctl` (2.3.1) + containerd. Data: `/var/lib/finite-sites`. | `systemd/finite-saas-sites.service` + `systemd/finite-saas-sites-kata.conf` drop-in, `systemd/finite-app@.service` (template for the non-Kata systemd runner; no instances running), `systemd/50-finite-sites.rules` (polkit), `systemd/finite-sites-nerdctl-sudoers` |
| `caddy.service` | Caddy **2.6.2** (distro unit at `/usr/lib/systemd/system/caddy.service`). Three vhosts — `api.finite.chat`, `*.finite.chat`, `*.docs.finite.chat` — all reverse_proxy → 127.0.0.1:8787. TLS via **Cloudflare Origin CA cert** (no ACME, no API token on box); Cloudflare proxies the zone in Full (strict). | `caddy/Caddyfile` |
| finite-search | Two Docker Compose projects under `/home/ubuntu/finite-search/`: SearXNG on 127.0.0.1:8080, Firecrawl (upstream checkout + Finite override) on 127.0.0.1:3002. Loopback-only. | `search.md` (compose sources live in `finite-search/compose/`) |
| `finite-core-tunnel.service` | **Previously undocumented.** Persistent SSH `-L 127.0.0.1:14200 → 10.43.237.180:4200` (finite-saas-core ClusterIP inside lat1's k3s) via `ubuntu@64.34.82.77`, key `/home/ubuntu/.ssh/finite-lat2-core-tunnel`. Enabled, running. | `systemd/finite-core-tunnel.service` |
| `finite-saas-runner.service` + `.timer` | **Previously undocumented, DORMANT.** "Finite agent creation runner": oneshot every 20s from the build-on-box checkout `/opt/finite/finitecomputer`. Timer is disabled and absent from `list-timers`. Stale `After=k3s.service` (no k3s here); depends on the core tunnel via drop-in. | `systemd/finite-saas-runner.service`, `.timer`, `systemd/finite-saas-runner-10-core-tunnel.conf` |
| GitHub Actions runners ×3 | **Three** runners (inventory said one), all v2.335.1, `User=ubuntu`, under `/srv/github-runner/`. Registered to finitechat, finitecomputer, and finitecomputer-v2 — all must be re-registered against finite-mono at cutover. | `runners.md` |

## Ports

| Bind | Port | Process | Notes |
|---|---|---|---|
| 0.0.0.0 / [::] | 22 | sshd | public |
| * | 80, 443 | caddy | public; Cloudflare-proxied zone |
| 127.0.0.1 | 8787 | finitesitesd | all three Caddy vhosts proxy here |
| 127.0.0.1 | 8080 | docker-proxy → SearXNG | |
| 127.0.0.1 | 3002 | docker-proxy → Firecrawl api | |
| 127.0.0.1 | 14200 | ssh (finite-core-tunnel) | → lat1 ClusterIP 10.43.237.180:4200 |
| 127.0.0.1 | 2019 | caddy admin API | |
| 127.0.0.1 | 41943 | containerd | ephemeral |

## Secrets inventory (names and locations only — values live on the host)

| Location | Contents | Consumer |
|---|---|---|
| `/etc/finite-saas/sites.env` (0640) | exactly one var: `RESEND_API_KEY`. (`systemd/sites.env.example` also documents optional `FINITE_IDENTITY_AUTHORITY`, not set live.) | finite-saas-sites.service |
| `/etc/finite-saas/certs/finite-chat-origin.pem` (0644 root:root) / `.key` (0640 root:caddy) | Cloudflare Origin CA cert pair for `finite.chat, *.finite.chat, docs.finite.chat, *.docs.finite.chat`; regenerated 2026-07-02 | Caddy |
| `/etc/finite-computer/runner.env` (0600 root) | 18 `FC_*` vars: `FC_CORE_URL`, `FC_CORE_API_TOKEN`, `FC_RUNNER_ID`, `FC_RUNNER_SOURCE_HOST_ID`, `FC_RUNNER_RELAY_URL`, `FC_RUNNER_RUNTIME_ARTIFACT_ID`, `FC_RUNNER_RUNTIME_ARTIFACT_KIND`, `FC_RUNNER_RUNTIME_ARTIFACT_REFERENCE`, `FC_RUNNER_RUNTIME_STATE_SCHEMA_VERSION`, `FC_RUNNER_WORK_ROOT`, `FC_RUNNER_MSB_BIN`, `FC_RUNNER_MSB_MEMORY`, `FC_RUNNER_MSB_CPUS`, `FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS`, `FC_RUNNER_RUNTIME_READY_INTERVAL_MS`, `FC_RUNNER_COMMAND_TIMEOUT_SECS`, `FC_RUNNER_RUNTIME_TEMPLATE_ROOT`, `FC_RUNNER_MAX_SANDBOXES` | finite-saas-runner.service (dormant) |
| `/home/ubuntu/finite-search/searxng/.env` | `SEARXNG_BIND`, `SEARXNG_PORT`, `SEARXNG_BASE_URL`, `SEARXNG_LIMITER`, `SEARXNG_SECRET` | SearXNG compose |
| `/home/ubuntu/finite-search/firecrawl-upstream/.env` | `PORT`, `HOST`, `USE_DB_AUTHENTICATION`, `BULL_AUTH_KEY`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB`, `MAX_CPU`, `MAX_RAM` | Firecrawl compose |
| `/var/lib/finite-sites/cookie-secret` (64 bytes) | finitesitesd session secret | finitesitesd |
| `/srv/github-runner/*/.credentials`, `.credentials_rsaparams` | runner registration credentials (never captured) | Actions runners |
| `/opt/finite/finitecomputer/secrets/` (0700 root) | unenumerated by design (root-only) | legacy runner tooling |
| `/home/ubuntu/smoke-identity.env` (0600, 90 bytes) | not read; contents unknown | unknown |

## Files here

- `systemd/` — unit files, drop-ins, polkit rule, sudoers, env example.
  Deployed-vs-repo drift check on 2026-07-08: **byte-identical** for
  Caddyfile, finite-saas-sites.service, kata.conf, finite-app@.service,
  nerdctl sudoers (these were moved here from
  `finite-sites/deploy/finite-lat-2/`). Files headed "Captured from host"
  or "PROPOSED" are new to any repo.
- `caddy/Caddyfile` — deployed at `/etc/caddy/Caddyfile`.
- `runners.md` — the three Actions runners + finite-mono cutover checklist.
- `backups.md` — backup reality and the proposed timer.
- `search.md` — how finite-search runs; points at `finite-search/compose/`.
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
   `finite-app@*`) exists on the host but in no repo — likely superseded by
   the polkit rule (`systemd/50-finite-sites.rules`), which the deploy doc
   says exists precisely because sudo cannot work under NoNewPrivileges.
7. Runner labels were recovered from config-time `_diag` logs; the
   authoritative label list lives on the GitHub side. Firecrawl compose
   reports one service `exited(1)` (identity not chased). No kata runtime
   section in `/etc/containerd` config (kata wired via the
   `containerd-shim-kata-v2` symlink).
