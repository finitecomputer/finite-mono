# finite-lat-2: emergency cutover to replacement app-plane host

Opened: 2026-08-28. Emergency: finite-lat-1 thermal failure, full product
outage since 2026-08-27. Authority: [ADR 0007](../adr/0007-finite-lat-2-emergency-app-plane-cutover.md).
Execution runbook: `infra/runbooks/lat2-replacement-cutover.md`.

## Situation

- lat1 (64.34.82.77): down, NixOS boots die pre-SSH, in rescue mode, disks
  RO. Full state pulled off-box (~55G, see session log): secrets banked,
  coordinated snapshot `20260827T023726Z`, Postgres dumps (18:35 UTC),
  identity backup (18:45 UTC), live chat/brain SQLite, sites (1.7G),
  hosted-device (1.2G), runner state (31G), raw pg dir (fallback).
- litestream bucket `finite-lat-1-litestream` holds seconds-RPO chat+brain;
  borg/rsync.net holds nightly history. Nothing has written anywhere since
  the crash — all sources agree to the outage point-in-time.
- lat3: up, its WG peer (to lat1) and Core/Identity routes are dead.
- lat4: future runner host (Austin, separate lane).

## Plan

1. **Wipe** lat2 via provider reinstall (skip stale archive by owner
   decision; unregister old GitHub runners + rotate legacy credentials
   post-cutover).
2. **Install** the replacement closure: lat1's service stack minus all
   Runner modules, on lat3's storage chassis (mirrored arrays, dual ESPs,
   storage health gates, ESP guard). Boots in **import mode** — product
   units down, Postgres up.
3. **Import**: secrets from the banked pull (minus `runner.env` /
   `phala-runner.env`, which belong to the runner lane) → pg_restore the
   latest dump into the fresh postgres_16 cluster → litestream-restore
   chat + brain from `finite-lat-1-litestream` → place
   sites/hosted-device/identity/core-relay trees at their exact paths.
4. **Verify offline**: 87 `finite_private_api_keys` rows, role password vs
   `FC_CORE_DATABASE_URL`, SQLite integrity + row-count checks, chat/brain
   health on loopback, sites registry, identity nostr.json, Core API auth.
5. **Go live**: go-live closure (`finite.importMode.enable = false`) via the
   `Lat2 NixOS Closure` artifact → full verify on lat2 → deploy lat3's peer
   flip (new overlay hub pubkey/endpoint) → owner flips DNS
   (Namecheap ×3 + `identity.finite.vip`, Cloudflare origin) → certs issue
   (ACME for the Namecheap names; the banked CF Origin pair for
   `*.finite.chat`) → `scripts/finite-status` green.

## Standing rules during the outage

- **lat1 never resumes writing.** Repaired = cold standby, and its
  litestream/borg destinations are frozen. Split-brain is the one
  unrecoverable outcome.
- Every mutating step: Paul's fresh approval, `scripts/finite-status`
  before/after, immutable evidence into the deployment changelog when done.
- The 31G runner state is lat4's inheritance, not lat2's.

## Checkpoints

| Date | Event | Result |
|---|---|---|
| 2026-08-27 | lat1 thermal failure; rescue mode; state pull started | — |
| 2026-08-28 | ADR 0007 (emergency cutover) + replacement scaffold PR | this PR |
