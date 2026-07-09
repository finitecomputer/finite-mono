# Break-glass: getting on the boxes

For incidents. Host facts (services, ports, secrets locations) live in
`infra/hosts/<name>/README.md` — read the host README before touching a box.

> **The rule:** any manual change made on a host must land back in `infra/`
> (or be reverted) **within a day**. The whole value of this tree is that it
> matches reality; an undocumented hotfix is drift, and drift is how the
> pre-mono mess happened. Note the change in your PR even if it is embarrassing.

## lat1 — finite-lat-1 (64.34.82.77)

- **Get on:** `ssh finite-lat-1` (user `ubuntu`). kubectl works for any
  local user (kubeconfig is world-readable by design — lat1 README).
- **Logs:**
  - `journalctl -u caddy` — edge for finite.computer
  - `journalctl -u k3s` — cluster
  - `journalctl -u finite-saas-runner` — agent-creation runner (fires every
    20s via the timer; each run is a fresh oneshot)
  - `kubectl -n finite-system logs deploy/finite-saas-core` (also
    `deploy/finite-dashboard`, `sts/finite-core-postgres`, and
    `kubectl -n finite-system get jobs` for the backup CronJob)
- **Restart:**
  - `sudo systemctl restart caddy` (config test first:
    `caddy validate --config /etc/caddy/Caddyfile`)
  - `kubectl -n finite-system rollout restart deploy/finite-saas-core`
    (same for dashboard)
  - `sudo systemctl restart k3s` — last resort; takes the whole control
    plane with it
  - Runner: usually nothing to restart — the timer re-invokes it. To pause:
    set `FC_RUNNER_DRAIN=true` in `/etc/finite-computer/runner.env`; to
    stop the loop entirely: `sudo systemctl stop finite-saas-runner.timer`.
- **Trap:** the Caddyfile and runner env hardcode core's ClusterIP
  `10.43.237.180` — if core mysteriously 502s after Service changes, that
  IP changed (lat1 README, "How finite.computer routes").

## lat2 — finite-lat-2 (64.34.80.19)

- **Get on:** `ssh finite-lat-2` (user `ubuntu`).
- **Logs:**
  - `journalctl -u finite-saas-sites` — finitesitesd (`*.finite.chat`)
  - `journalctl -u caddy`
  - `journalctl -u finite-core-tunnel` — SSH tunnel to lat1 core
  - `journalctl -u 'actions.runner.*'` — the three GitHub Actions runners
  - search: `docker compose logs` in the two projects under
    `/home/ubuntu/finite-search/`
- **Restart:**
  - `sudo systemctl restart finite-saas-sites` — TODO: effect on running
    Kata microVMs unverified (see [deploy-sites.md](deploy-sites.md)
    cautions)
  - `sudo systemctl reload caddy` — prefer reload; TLS is a Cloudflare
    Origin CA cert, do not switch to ACME while firefighting
  - `sudo systemctl restart finite-core-tunnel`
  - runners: `sudo ./svc.sh stop|start` in the runner dir under
    `/srv/github-runner/`, or systemctl on the `actions.runner.*` unit
- **Trap:** `/tmp` is a 94G tmpfs — never park a backup or artifact there
  (`infra/hosts/lat2/backups.md`).

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

## clawland — clawland-ovh (15.204.108.57)

- **Get on:** `ssh ovh-rescue` (= `root@15.204.108.57`).
- **Legacy fleet box** — mono's scope here is ONLY the finitechat server
  (`infra/hosts/clawland/README.md`); coordinate anything else with the
  legacy `finitecomputer` repo (workspace `ovh-fc-1`). Also NixOS: same
  fix-forward caveat as smoke.
- **Logs:**
  - `journalctl -u finitechat-server` — the live chat.finite.computer server
  - `journalctl -u fc-offsite-backup` (+ `systemctl list-timers` to confirm
    the borg timer is live)
  - edge is the fleet's socat → k3s Traefik, same pattern as smoke
- **Restart:** `systemctl restart finitechat-server`, then confirm
  `curl -fsS https://chat.finite.computer/health` reports the expected
  `source_commit` ([deploy-finitechat-server.md](deploy-finitechat-server.md)).
- **Trap:** it is also the nix build host for smoke deploys — an outage here
  blocks smoke's deploy path too.
