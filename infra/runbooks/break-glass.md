# Break-glass: getting on the boxes

For incidents. Host facts (services, ports, secrets locations) live in
`infra/hosts/<name>/README.md` — read the host README before touching a box.

> **The rule:** any manual change made on a host must land back in `infra/`
> (or be reverted) **within a day**. The whole value of this tree is that it
> matches reality; an undocumented hotfix is drift, and drift is how the
> pre-mono mess happened. Note the change in your PR even if it is embarrassing.

## lat1 — finite-lat-1 (64.34.82.77) — THE app server, NixOS

Since the 2026-07-09 cutover lat1 runs EVERYTHING (Core, dashboard, native
Postgres, chat, sites) as NixOS. No k3s, no kubectl. Config is
`infra/nixos/`; routine deploy uses the service runbooks and the exact prebuilt
closure. There is currently no accepted bare-metal rebuild procedure.

- **Get on:** `ssh root@64.34.82.77` — **key-only, NO console password.** If
  the box will not boot and SSH is dead, use Latitude Rescue Mode and IPMI only
  for read-only diagnosis. The historical
  [2026-07-09 transcript](lat1-nixos-reinstall.md) may supply console/network
  facts, but none of its wipe/install commands is current authority. Preserve
  every disk and state source, fence writers if needed, and escalate to the
  [finite-lat capacity/redundancy recovery plan](../../docs/runs/finite-lat-capacity-and-redundancy.md)
  before mutation.
- **Logs** (all native systemd/journald — no kubectl):
  - `journalctl -u caddy` — the single edge (finite.computer,
    chat.finite.computer, *.finite.chat)
  - `journalctl -u finite-saas-core` — Core (127.0.0.1:4200)
  - `journalctl -u podman-finite-saas-dashboard` — dashboard container
    (127.0.0.1:3000)
  - `journalctl -u postgresql` — native Postgres 16 (`finite_core`)
  - `journalctl -u finitechat-server` — chat (127.0.0.1:8788)
  - `journalctl -u finite-saas-sites` — finitesitesd (127.0.0.1:8787)
  - `journalctl -u finite-postgres-backup` — the 6-hourly dump timer
  - `journalctl -u finite-saas-runner` — Kata agent-creation runner; its Nix
    timer is enabled, but live canary readiness must be verified. Phala remains
    a separate fast-follow adapter.
- **Restart:**
  - `sudo systemctl restart caddy`
  - `sudo systemctl restart finite-saas-core` / `finite-saas-sites` /
    `finitechat-server` / `postgresql` / `podman-finite-saas-dashboard`
  - Runner: the timer re-invokes it; to pause new work,
    set `FC_RUNNER_DRAIN=true` in `/etc/finite/runner.env` or
    `sudo systemctl stop finite-saas-runner.timer`.
- **Do NOT edit units on the box.** lat1 is declarative — a hotfix survives
  only until the next `nixos-rebuild switch` and then reverts. Fix forward in
  `infra/nixos/` and re-deploy; land any emergency change back within a day
  (rule above). To roll config back fast: `nixos-rebuild switch --rollback`.

## lat2 — finite-lat-2 (64.34.80.19) — emergency replacement app server (ADR 0007)

lat2 is being cut over to replace lat1 (thermal failure). Until Gate C of
[`lat2-replacement-cutover.md`](lat2-replacement-cutover.md) it is the old
Ubuntu box; after install it is THE single app server (lat1's stack minus
any Agent Runner) and the `wg-finite` overlay hub at `10.254.3.1`.

- **While Ubuntu (pre-Gate C):** `ssh finite-lat-2` (user `ubuntu`). Do not
  build or deploy from here; there is no supported lat2 fallback for CI,
  Nix builds, or deploys. `/tmp` is a 94G tmpfs — never park a backup there
  and use the current service backup runbooks instead of the deleted
  historical lat2 backup proposal.
- **While NixOS (post-Gate C):** `ssh root@64.34.80.19` (the SSH host key
  changes at reinstall; expect a known_hosts break). Declarative like lat1:
  fix forward in `infra/nixos/hosts/finite-lat-2/` and redeploy via
  `just deploy-lat2-closure`; roll back with `nixos-rebuild switch
  --rollback`. **Import mode**: while `finite.importMode.enable` is true the
  product units stay down by design — bringing the product up means
  deploying the go-live closure, never `systemctl start` by hand.
- **lat1 is the frozen point-in-time record.** Do not boot it back into
  service; a second writer on Postgres/chat/sites is split-brain.

## smoke — ovh-vps-smoke (15.204.56.61)

- **Get on:** `ssh root@15.204.56.61` — **NOTE: no ssh alias exists yet**
  (the `ovh-rescue` alias points at clawland, not here — smoke README).
  TODO: add a `finite-smoke` (or similar) alias to operator ssh configs.
- **This is NixOS, managed by the legacy `finitecomputer` repo.** Any manual
  change to units/config will be silently reverted by the next
  `just host-deploy` switch — fix forward in the legacy repo (see
  `infra/hosts/smoke/deploy.md`), and mirror the fact into `infra/` per the
  rule above.
- **Logs:**
  - `journalctl -u finite-brain-app` — the brain (:3015)
  - `journalctl -u fc-agent-cluster-http-bridge -u fc-agent-cluster-https-bridge`
    — the socat edge (80/443 → k3s Traefik NodePorts)
  - `journalctl -u k3s`; oauth2-proxy lives in-cluster:
    `kubectl -n fc-auth logs deploy/...` (name per cluster)
- **Restart:** `systemctl restart finite-brain-app` (unit is
  `Restart=always`, so crashes self-heal); socat bridges likewise;
  `systemctl restart k3s` last resort.
- **Traps:** NO backups on this host — the brain's SQLite at
  `/var/lib/private/finitebrain/` is the only copy; take a manual copy
  before anything risky. Disk was 82% full at capture. `/_admin` bypasses
  oauth at the edge (smoke README, risks).

## clawland — clawland-ovh (15.204.108.57) — legacy fleet box

- **Get on:** `ssh ovh-rescue` (= `root@15.204.108.57`).
- **Legacy fleet box** (finite.vip fleet) — coordinate anything here with the
  legacy `finitecomputer` repo (workspace `ovh-fc-1`). NixOS: same fix-forward
  caveat as smoke.
- **`finitechat-server` here is DISABLED** (migrated to lat1 at the
  2026-07-09 cutover, per the single-writer doctrine —
  [deploy-finitechat-server.md](deploy-finitechat-server.md)). Do NOT
  re-enable it: chat is single-writer, and lat1 is the live writer.
  `chat.finite.computer` now resolves to lat1.
- **Logs:** `journalctl -u fc-offsite-backup` (legacy borg, still relevant to
  the fleet); edge is the fleet's socat → k3s Traefik, same pattern as smoke.
- Otherwise unchanged legacy — nothing mono actively runs here now.
