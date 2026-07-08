# finitechat-server on clawland-ovh (the live chat server)

The production finitechat server for `chat.finite.computer` runs on
**clawland-ovh (15.204.108.57)**, a legacy-fleet box. Captured read-only
2026-07-08.

## How it runs

- systemd unit: `finitechat-server.service` (running).
- Cmdline: `/var/lib/finite-chat/bin/finitechat-server serve 10.42.0.1:8787
  --sqlite /var/lib/finite-chat/data/server.sqlite3`
- Binary: `/var/lib/finite-chat/bin/finitechat-server` (on-disk, not a nix
  store path — installed outside the NixOS module system).
- Storage: SQLite at `/var/lib/finite-chat/data/server.sqlite3`.
- Bind: `10.42.0.1:8787` — the host's cni0 (k3s bridge) address, so it is
  reachable from cluster pods and the host, not on the public interface.

## Network path

```
chat.finite.computer (DNS → 15.204.108.57)
  → :443 host socat bridge → 127.0.0.1:30443 (same fc-agent-cluster pattern as smoke)
  → k3s Traefik (Let's Encrypt for the box's vhosts, workspace ovh-fc-1)
  → 10.42.0.1:8787 (host systemd service on the cni0 address)
```

Fronted by the legacy fleet's Traefik/oauth stack (workspace `ovh-fc-1` in
the legacy `finitecomputer` repo); the exact IngressRoute spec for
`chat.finite.computer` was not captured — it lives in that repo/cluster.

## Backups

Covered: this box runs active borg offsite backups —
`fc-offsite-backup.service` + `fc-offsite-backup.timer` (the legacy repo's
`host-offsite-backup.nix` module) and `/root/box1_borg_backup.sh`;
`/root/backup-dashboard/` exists. (Contrast: smoke has none.)

## Contract with the mono repo

Mono's `finitechat/docs/server-deployment-gate.md` gates app releases on this
server's `GET /health` reporting `server_contract_version`, `server_version`,
and `source_commit` matching the expected finite-chat commit
(`scripts/server-contract-gate.py --server https://chat.finite.computer`).
Server deploys to this box are done from the legacy repo ("box1"), but the
server source and route contracts live in mono — any server-behavior change
requires a deploy here to pass the gate.

## Migration story

Its future home is **lat1**, via
`infra/hosts/lat1/scripts/deploy-finitechat-server.sh`. A 2026-07-07 lat1
deploy was rolled back after ~2 minutes; leftovers from that attempt are
still on lat1 (see `infra/hosts/lat1/`).

Moving it is a **deliberate cutover with a data migration** — quiesced SQLite
copy of `/var/lib/finite-chat/data/server.sqlite3` plus a DNS flip of
`chat.finite.computer` — and is **NOT part of this migration PR**. Until that
cutover, clawland remains the server of record and this file is the map.
