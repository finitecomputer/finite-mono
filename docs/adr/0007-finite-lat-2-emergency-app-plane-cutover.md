# ADR 0007: finite-lat-2 is the emergency replacement app-plane host

Status: accepted, 2026-08-28. Supersedes the same-numbered ADR drafted on
2026-08-27 (lat2 as a second Runner host), which was never merged; that
scaffold's chassis is reused here. This is an emergency-decision record, not
a ceremony: finite-lat-1 is down with a thermal failure (2026-08-27) and no
user can reach any product.

## Context

- finite-lat-1 (64.34.82.77) — the single app server: Core, Postgres, chat,
  hosted-device, sites, Brain, Identity, dashboard, search, Caddy edge,
  backups, litestream, and the lifecycle Kata runner for existing Agents —
  overheats and can no longer boot NixOS. It sits in Latitude rescue mode
  with disks mounted read-only; a full state pull is banked off-box
  (~55G: coordinated snapshot, Postgres dumps, live chat/brain SQLite,
  sites/hosted-device/identity trees, runner state, secrets), with litestream
  (seconds RPO) and borg/rsync.net behind it.
- finite-lat-2 (64.34.80.19) is the same chassis class as lat1/lat3, was an
  Ubuntu decommission target, and its decommission prerequisites were
  re-scoped as "skip the stale archive; wipe via provider reinstall" by
  owner decision during this outage.
- finite-lat-3 (207.188.7.157) is up and unaffected (it peered with lat1 for
  the private Runner path, which died with lat1).

## Decision

- finite-lat-2 is wiped and reinstalled as **the replacement single app
  server**: finite-lat-1's exact service stack — Core, native Postgres 16,
  chat, hosted-device, sites, Brain, Identity, dashboard, search, Caddy
  edge, backups (borg), litestream — on the lat3-qualified storage chassis
  (mirrored root and data arrays, dual ESPs, fail-closed storage health,
  ESP-write guard). It runs **no Agent Runner**: the runner lane moves to a
  future host (lat4; separate work).
- The host boots in a declarative **import mode** (`finite.importMode`):
  product units do not start until lat1's state is imported and verified
  offline; the go-live closure flips the option.
- Restore authorities, by owner decision 2026-08-28: Postgres from the
  latest coordinated dump; chat + Brain from the **litestream** bucket
  (drilled restore path); sites/hosted-device/identity/core-relay trees from
  the banked file pulls. lat1's frozen copies remain the point-in-time
  record; nothing writes to lat1's buckets or repos again.
- New off-host destinations for the new authority host:
  `finite-lat-2-litestream` bucket and
  `fm2890@fm2890.rsync.net:finitecomputer/finite-lat-2` — lat1's archives
  are preserved untouched.
- The `wg-finite` overlay hub role moves with the app plane: lat2 takes
  `10.254.3.1` (Core socket proxy + Identity Authority proxy), lat3's peer
  entry is re-pointed to lat2's public endpoint as part of cutover, and the
  overlay widens to `10.254.3.0/29` for the future runner host.
- DNS: `finite.computer`, `chat.finite.computer`,
  `brain.finite.computer` (Namecheap) and `identity.finite.vip` move to
  64.34.80.19; Cloudflare's `*.finite.chat` origin moves to the same IP.
- Existing hosted Agents remain on lat1's runner state, which was pulled
  off-box; they come back when the runner lane host imports it. This
  cutover restores the control/app plane, not Agent compute.

## Consequences

- Every runner host's `FC_CORE_URL`/Identity Authority endpoint is unchanged
  (both keep pointing at the 10.254.3.1 overlay hub) — only the peer
  public key/endpoint flip on lat3 is needed.
- `finite-lat-1` must never resume writing: when repaired it is cold
  standby at best, and its litestream/borg destinations are frozen. A
  split-brain here means two Postgres and two chat writers.
- The monitoring/dashboards/finite-status contracts gain `finite-lat-2` as
  the second `app`-role host and, with it, the Recovery Authority role
  (its backups are now the fleet's live recovery set).
- The 2026-08-27 runner-twin scaffold (storage capture tooling, `Lat2 NixOS
  Closure` artifact workflow, install mechanics, /29 overlay widening) is
  reused for this cutover; only the services layer and contracts changed.

## Rejected alternatives

- Repairing lat1 in place before restoring service: no ETA on the thermal
  fault, and every hour is a full product outage.
- Making lat2 the Runner twin (the superseded plan): product availability
  now depends on the app plane, not Agent capacity.
- Restoring from lat1's raw Postgres datadir instead of dumps: the dumps are
  the drilled, version-clean path; the raw dir stays fallback only.
- Keeping lat1's litestream bucket/borg repo names for the new host: two
  machines appending to one archive corrupts the point-in-time record.
