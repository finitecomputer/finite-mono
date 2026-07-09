# clawland-ovh — 15.204.108.57

> **UPDATE 2026-07-09 — finitechat-server has MIGRATED to lat1 and is DISABLED
> here.** `chat.finite.computer` now resolves to lat1 (native
> `finitechat-server` on :8788). clawland is now **purely the legacy finite.vip
> fleet box** — it is out of mono's scope again except as the nix build host for
> smoke deploys. The finitechat section below is **historical** (its migration
> story is in [`finitechat-server.md`](finitechat-server.md)).

Legacy finite.vip fleet box, managed by the LEGACY `finitecomputer` repo
(deliberately outside finite-mono). It **formerly hosted the live finitechat
server** (migrated to lat1 in the 2026-07-09 cutover) — see
[`finitechat-server.md`](finitechat-server.md).

Older inventory mislabeled this IP "ovh-vps-smoke". Wrong: the real
ovh-vps-smoke is 15.204.56.61 (see [`../smoke/`](../smoke/README.md)). This
box is bare-metal (MSI, 1.8T md0 RAID at 35%, ~126G RAM), NixOS 26.05,
hostname `clawland-ovh`, ssh alias `ovh-rescue`. Captured 2026-07-08.

## What mono cares about

- **finitechat-server.service** — **DISABLED / migrated to lat1 (2026-07-09)**.
  Historically ran here as
  `/var/lib/finite-chat/bin/finitechat-server serve 10.42.0.1:8787 --sqlite
  /var/lib/finite-chat/data/server.sqlite3`, fronted by the legacy fleet's
  Traefik/oauth stack (workspace `ovh-fc-1`), with borg backups
  (`/root/box1_borg_backup.sh`, `fc-offsite-backup.service` + timer).
  `chat.finite.computer` DNS now points at **lat1**, where the server runs
  natively on :8788 (SQLite copied under single-writer doctrine — never two
  writers). Details and migration story in
  [`finitechat-server.md`](finitechat-server.md).
- It is the **nix build host for ovh-vps-smoke deploys** (`/root/result` →
  a built `nixos-system-ovh-vps-smoke` closure; `host_deploy.sh
  --build-host` path).

## Everything else (legacy fleet — one line each, see the legacy repo)

- k3s + Traefik with Let's Encrypt for `*.finite.vip`; socat 80/443 bridges
  (same pattern as smoke).
- `finited` control plane for workspace `ovh-fc-1`, with
  `fc-control-plane-reconcile.path`/`.service` units.
- oauth2-proxy for `.finite.vip` (Google).
- ~50 per-user agent namespaces (statefulset `<user>-0`) plus published
  `*.finite.vip` apps; finitec relay/gateway pollers per agent.
- matrix-synapse; finite-specialization-worker :18998.
- Namespace `smoke-finite` here is a finite.vip user workspace, unrelated to
  the `smoke.finite.computer` domain.
- Leftover `/var/lib/caddy` (nothing runs it).
