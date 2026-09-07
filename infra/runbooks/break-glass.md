# Break-glass: getting on the boxes

For incidents. Host facts (services, ports, secrets locations) live in
`infra/hosts/<name>/README.md` — read the host README before touching a box.

> **The rule:** any manual change made on a host must land back in `infra/`
> (or be reverted) **within a day**. The whole value of this tree is that it
> matches reality; an undocumented hotfix is drift, and drift is how the
> pre-mono mess happened. Note the change in your PR even if it is embarrassing.

## lat1 — finite-lat-1 (64.34.82.77) — retired app server

Since the 2026-08-29 ADR 0007 emergency cutover, lat1 is a frozen
point-in-time record after thermal failure. It previously ran the app-plane
stack as NixOS, but it is no longer production deploy authority.

- **Get on:** `ssh root@64.34.82.77` — **key-only, NO console password.** If
  the box will not boot and SSH is dead, use Latitude Rescue Mode and IPMI only
  for read-only diagnosis. The historical
  [2026-07-09 transcript](lat1-nixos-reinstall.md) may supply console/network
  facts, but none of its wipe/install commands is current authority. Preserve
  every disk and state source, fence writers if needed, and escalate to the
  [finite-lat capacity/redundancy recovery plan](../../docs/runs/finite-lat-capacity-and-redundancy.md)
  before mutation.
- **Read-only logs only:** use `journalctl` for incident forensics if an
  incident plan explicitly names lat1. Do not start services, restart units, or
  point DNS back at this host without a separate production-repair plan.
- **Do NOT edit or restart units on the box.** Treat lat1 as historical
  evidence unless an incident plan explicitly names a read-only recovery step.

## lat2 — finite-lat-2 (64.34.80.19) — emergency replacement app server (ADR 0007)

lat2 is the live single app server (lat1's stack minus any Agent Runner) and
the `wg-finite` overlay hub at `10.254.3.1`.

- **Get on:** `ssh root@64.34.80.19`. Declarative NixOS:
  fix forward in `infra/nixos/hosts/finite-lat-2/` and redeploy via
  `just deploy-lat2-closure`; roll back with `nixos-rebuild switch
  --rollback`.
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
- **`finitechat-server` here is DISABLED** (migrated off clawland at the
  2026-07-09 cutover, then to lat2 by ADR 0007). Do NOT re-enable it: chat is
  single-writer, and lat2 is the live writer.
- **Logs:** `journalctl -u fc-offsite-backup` (legacy borg, still relevant to
  the fleet); edge is the fleet's socat → k3s Traefik, same pattern as smoke.
- Otherwise unchanged legacy — nothing mono actively runs here now.
