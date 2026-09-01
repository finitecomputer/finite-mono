# Chat plane freeze, 2026-08-27 → 2026-08-29

Status: **CLOSED (recovered 2026-08-29)**. This is the incident record; the
repair procedure it forced into existence is now a runbook:
[`infra/runbooks/chat-quarantine-repair.md`](../../infra/runbooks/chat-quarantine-repair.md).
All times UTC.

## Summary

The Aug 27 platform wave (`b9254c81`, deployed ~02:37) froze the chat server's
durable room projections and its snapshot cadence at op 198825. The op log
kept growing — ~8,000 more ops by Aug 29 — while every derived table the
server validates against stayed pinned to the Aug 27 state. Chat limped for
two days on activity hints and op-log fetches, then lat1 died thermally, and
the emergency cutover to lat2 faithfully carried the already-frozen database
into the new box. Unfreezing the server (#770) fleet-recovered the agents and
immediately exposed a second, silent bug: quarantined rooms livelocked the
hint path at network speed (#776). Five agents needed per-agent state repair
via the sanctioned diagnose/repair tooling. Nobody noticed any of this for
~42 hours because every health signal we habitually read was green or
meaningless. That list is the most valuable output of the incident.

## Timeline

| Time (UTC) | Event |
|---|---|
| Aug 27 ~02:37 | Platform wave `b9254c81` deployed to both NixOS closures (incl. `ded6fe5e`, the semantic readiness probe). Chat server's room projections and snapshot cadence freeze at op 198825. |
| Aug 27–28 | Chat limps: activity hints and op-log fetches keep conversations superficially alive. Endpoints' local stores grow to seqs 10407/10460 while the server projection for the example room stays at 3413. |
| Aug 28 ~20:40 | finite-lat-1 dies thermally (ADR 0007 dates the failure onset 2026-08-27; the host was fully dead this evening). Emergency cutover to finite-lat-2 begins. |
| Aug 29 06:22 | lat2 emergency-cutover boot. The frozen DB is carried over byte-faithfully — the restore is NOT the cause of the freeze (early wrong theory, see below). |
| Aug 29 daytime | Symptom chase: the sidecar SSE reader bug (#765/#768) looks like the outage; Waffle Prime is the visible casualty. Real, but secondary. `2026-08-29.3` (PRs #765+#768) promoted 17:40Z. |
| Aug 29 19:56 | #770 merged: boot reconciliation + snapshot-cadence fix, crash-atomic per review. |
| Aug 29 ~20:39 | #770 deployed to lat2 (chat-server system closure; the post-deploy `/health` fingerprint changes). First serving boot replays the ~8,000-op tail and rewrites the lagging projection rows before serving. |
| Aug 29 21:28–22:09 | Chat-authz stack (#710/#711/#712) rolls as a platform wave: lat2 closure from `9788a9ad` (Core migration `0023`; `finitechat-server` restarted 22:08:20Z), Agent Runtime `2026-08-29.4` promoted 22:09Z, fleet 51/51 (30 lat3 + 21 lat4). |
| Aug 29 evening | Fleet recovery exposes the quarantine livelock (#776): agents refetch rejected pages at 13–25/s, silently. One agent measured 25.3 fetches/s (~160k fetches, ~50 GB egress); five agents loop at ~90 fetches/s aggregate (~11 MB/s) against the chat server. |
| Aug 29 evening | Five agents repaired via the sanctioned tooling (table below): 20 skips total, ~2,800 held messages released. |
| Aug 29 23:03–23:05 | #776 merged; Agent Runtime `2026-08-29.5` built 23:05Z carrying the livelock fix. |

## Root cause

Two independent defects, both introduced by the Aug 27 wave and both fixed in
#770:

1. **Runtime split-brain.** `from_sqlite_path` rebuilt the delivery service to
   the true op-log head via snapshot + tail replay, but loaded
   `room_memberships` from the frozen projection table. Typed publishes were
   then gated against that stale projection: `validate_event_room_membership`
   rejected on `envelope.epoch != projection.current_epoch` (400) and
   `device_active_at_head` rejected devices whose intervals start above the
   frozen `last_seq` (403). Rejection happened **before** the op was written —
   the fleet-wide signature.
2. **Structurally starved snapshot cadence.** `note_op_for_snapshot` was
   called only from the `/commits` and `/events` handlers. The ops that kept
   flowing after the freeze (key-package publish/claim, lease expiry, revoke)
   never incremented `ops_since_snapshot`, so the counter could never again
   reach `SNAPSHOT_INTERVAL_OPS`. The snapshot writer was not failing — it was
   starved by construction.

The fix (#770): boot reconciliation replays each room's entries above the
projection row's `last_seq` with live-path semantics and persists repaired
rows **before serving starts** (idempotent across restarts, derived from the
authoritative log, no hand-edited production data); every op-appending state
method now counts toward the snapshot interval, a `Drop` guard releases the
in-flight flag, and failed readiness-probe transactions ROLLBACK.

## Wrong theories (recorded so they cost nobody else a night)

- **"The cutover restore caused the freeze."** False. The freeze began Aug 27
  ~02:37 at op 198825, ~42 hours before lat1 died. The emergency cutover
  carried the frozen database to lat2 faithfully — the restore was correct.
- **"`readiness SQLite commit failed: database is locked` means a
  service→store lock inversion."** False. It was the `ded6fe5e` readiness
  probe's 450 ms budget colliding with long writers — a symptom of the same
  wave, verified not to be an ordering bug.
- **"The SSE reader bug is the outage."** #765/#768 (unbounded blocking HTTP
  call; SSE read budget vs heartbeat) were real bugs with a real casualty
  (Waffle Prime wedged), and `.3` was worth shipping — but fixing them did not
  move any projection. Secondary.

## What lied to us

The point of this section: every item below read as evidence of health or of
a cause during the incident, and every one is deletion or simplification
fuel.

| What we read | What it actually said | Follow-up fuel |
|---|---|---|
| Health probes green while the chat plane was fleet-dead | `/healthz` is static (contract version + fingerprint); it cannot observe serving truth. Even the semantic `/readyz` (#678) answered ready — the server was "ready" by its own frozen view. | Make liveness mean something or delete the probe surface; readiness must fail when projections lag the op log. |
| Status files with fresh mtimes | Status JSON files are written **on change**. A fresh mtime proves nothing about the process that would have written them. | Carry a generation/timestamp inside every status file and age it out. |
| `hermes-bridge-status.json` "connected" (misread repeatedly tonight) | It measures the LOCAL hermes↔sidecar bridge, not the server stream. A green bridge card sat on top of a dead upstream all night. | Rename/scope it, or fold it into one status surface that distinguishes local vs upstream. |
| `client_app_events` max-seq as "the cursor" | The events table's max-seq is not the durable sync cursor (they diverged by 10 on one agent during the incident). | One cursor, one source, exposed by the diagnostic. |
| "10407" at death | That was the AGENT-side store head, not a server copy. We briefly reasoned about server state from an agent-local number. | Label every seq in output with its owner (agent store vs server projection vs op log). |
| Runbook topology | `deploy-finitechat-server.md` still said chat runs on finite-lat-1 — dead since Aug 28. Operators were pointed at a dead host mid-incident. | Topology sweep landed with this PR; runbooks must carry no host facts that `scripts/finite-status` can contradict. |
| No admission UI for `chat.admission` | The authz stack's admission state was CLI-only; debugging "why is this agent rejected" required host-side spelunking. | Ship the minimal admission surface or delete the manual path's camouflage. |
| `http_state_snapshots` vs `http_state_snapshots_v2` | Dual snapshot tables made forensics slower: which table is authoritative had to be re-derived mid-incident. | Delete the v1 table on the next system closure that can migrate it. |

## The five per-agent repairs

All five were quarantined MLS application ciphertext entries (the device
could not decrypt another member's message, cursor froze, #776's hint loop
made it loud). Per the runbook: capture from the server above the cursor,
diagnose on a byte copy (every skip proven `kind=application`,
class `mls_application_ciphertext`, else STOP), repair with the container
stopped, verify catch-up and a fresh `hermes-inbox.json`.

| Agent | Skips |
|---|---|
| Waffle (Prime) | 4 (incident alias `waffle-prime-livelock-20260829`) |
| Agent M | 3 |
| Argus | 3 |
| BrainBot | 6 |
| Jack | 4 |
| **Total** | **20** — ~2,800 held messages released |

Every skipped entry classified `kind=application`,
`error_class=mls_application_ciphertext`. Audit JSONL and pre-repair backups
retained per the runbook. No payload was ever read; the tooling is
privacy-locked by construction.

## What shipped

- **#770** — boot reconciliation + snapshot cadence (chat-server closure,
  deployed ~20:39; included in the `9788a9ad` system closure whose fingerprint
  the contract gate now reports).
- **#765/#768** — sidecar transport bounds (Agent Runtime `2026-08-29.3`,
  promoted 17:40Z; folded into `.4`).
- **#710/#711/#712** — owner-scoped chat admission stack (NIP-98 on
  account-scoped routes, Welcome allowlist, store-backed admission + Core
  migration `0023`): lat2 closure from `9788a9ad`, Agent Runtime
  `2026-08-29.4` promoted 22:09Z, fleet 51/51.
- **#776** — quarantine backoff + the first real quarantine visibility
  (single-line stderr report, `/readyz runtime_status`): Agent Runtime
  `2026-08-29.5`.

Deployment facts and digests: `infra/deployment-changelog.md`, 2026-08-29
entries.
