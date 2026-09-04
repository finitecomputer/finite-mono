# Technical Debt Ledger

Tolerated shortcuts. Each item has an observed source, a risk, the first
proof of the shortcut in code, and a delete condition. A shortcut without a
delete condition is unfinished design, not accepted debt.

## 1. RESOLVED — real mailer implemented

`HttpMailer` (Resend, via the shared `finite-mail` transport) ships behind
the `Mailer` trait, selected with `--mailer` + `--mail-from`, key via env
var. Remaining work is configuration: domain verification plus a real-inbox
validation gate in the current Sites deploy authority. Local and Devfinity
select the dev mailer with `--mailer dev`; omitting the flag is an error.

## 2. Login-link rate limiting only; no platform-wide limits

- **Source**: closed the login-link half (per-(site,email) and per-IP
  budgets in `crates/finitesitesd/src/limiter.rs`, applied in
  `request_link`); general request limiting deliberately deferred because
  Cloudflare's proxy fronts the serving plane in the planned deploy.
- **Risk**: API-plane brute force (NIP-98 makes this low-value) and
  origin-direct floods if Cloudflare is bypassed.
- **Proof**: only `request_link` consults `login_limiter`.
- **Delete condition**: per-IP budgets on the API plane (project init attempts and
  git deploys per pubkey per hour) before registration opens beyond the operator
  publish grant gate; Cloudflare rate-limiting rules on `/_finite/*` as
  belt-and-braces when the zone goes live.

## 3. RESOLVED for serving — one control-plane writer remains

- **Resolution**: site traffic uses a bounded pool of independent query-only
  SQLite connections. Registry reads and verified blob reads run on Tokio's
  blocking pool. Static site requests do not take `AppState.engine`.
- **Atomicity boundary**: the control-plane writer still serializes mutations.
  Publication writes and verifies immutable blobs before atomically activating
  the new version; readers retain the resolved version id.
- **Proof**: `ServingEnginePool` plus
  `serving_pool_does_not_head_of_line_block_independent_reads`; the Store
  regression proves readers observe committed writes and reject mutation.
- **Remaining debt**: low-volume API and auth mutations still use the one writer
  Engine. Revisit only if measured control-plane p95 exceeds 50 ms; do not add
  writable serving connections.

## 4. Filesystem blob store and unreplicated registry

- **Source**: local v1; no object storage running.
- **Risk**: single-disk durability for all site content and the registry.
- **Proof**: `crates/finitesites-blob/src/lib.rs` writes under `--data`.
- **Delete condition**: Garage/S3 `BlobStore` implementation and a
  Litestream replication unit for `registry.db` in the production deploy
  definition.

## 5. Global blob dedup leaks hash existence

- **Source**: ADR-0007 chose global dedup.
- **Risk**: low — a publisher can learn whether some exact file already
  exists on the platform by watching the missing list.
- **Proof**: `Store::missing_blobs` consults a global `blobs` table.
- **Delete condition**: revisit before opening registration beyond the
  operator/Core publish grant gate; either accept formally in the ADR or scope
  dedup per owner.

## 6. No name release / key rotation surface

- **Source**: disable/delete are now operator commands; name release and key
  rotation remain out of the v1 user contract.
- **Risk**: names cannot be intentionally returned to the pool without
  operator SQL, and compromised or lost owner keys can permanently block user
  access even though Sites still holds the repository and site. This is a
  first-slice Recoverability Contract blocker, not post-launch polish.
- **Proof**: `finitesitesd disable-site` and `finitesitesd delete-site`
  mutate site status with audit events; there is no release-name or key
  rotation command.
- **Delete condition**: an audited Publishing Ownership Recovery flow gated by
  verified Account Auth/email or another independent authorized Principal,
  plus key rotation and destructive recovery tests, before durable first-slice
  publishing. Direct operator SQL is not the product recovery flow.

## 7. RESOLVED — NIP-98 URL matching verified through the live proxy

`https://api.finite.chat` is pinned end to end and the signed-call gate
passed on 2026-06-09 and later updated to the Project Repository flow:
project init plus git push from a remote machine through Cloudflare
succeeded against finite-lat-2. The residual behavior (a
misconfigured `--api-url` fails closed with "url mismatch") remains the
expected signed-call behavior.

## 8. RESOLVED — app runner debt removed by static-only Sites

ADR 0028 cuts app/document output kinds, Kata app runners, app proxying, and
wake-on-request from Finite Sites. The previous tier-2 runtime debt entries
are closed by removal rather than by completing the app-hosting path:

- no `kind = "app"` public contract;
- no app bundle manifest exception;
- no app proxy or websocket/log surface;
- no Sites-specific Kata/containerd/sudo integration;
- no wake-on-request app supervisor.

Future dynamic compute belongs in a separate product boundary, not as another
Sites kind.

## 9. RESOLVED — Project Repository pushes use durable post-receive events

Project Repositories now install a `hooks/post-receive` helper that records
bounded durable git ref-change events before the Git client sees success.
`finitesitesd` reconciles pending events after receive-pack and at daemon
startup. Tests cover real `git clone`/`git push`, ignored non-deploy refs,
missing output failure, restart reconciliation after a ref update before
deploy, and idempotent replay after Version creation before event
acknowledgement.

## 10. RETIRED — `reconcile-identity` one-shot migration command

- **Source**: the mailbox-grant → native-Principal reconciliation was a
  completed one-shot migration. Its optional Core cross-check called
  `/api/core/v1/brain/agent-account`, which the auth-kernel stack deleted
  (its only consumers are gone).
- **Risk**: none from removal — the migration already ran; durable grants
  were rewritten additively and the command was never part of startup.
  The store-layer reconciliation helpers stay because the engine and store
  test fixtures still exercise their local invariants (e.g. automated
  evidence never resurrects a revoked key).
- **Proof**: `finitesitesd reconcile-identity`, its Directory/Core clients
  (`crates/finitesitesd/src/identity.rs`), and the devfinity smoke
  operator-boundary check are deleted; the daemon no longer reads
  `FINITE_IDENTITY_AUTHORITY` / `FC_CORE_API_*` anywhere.
- **Delete condition**: this entry is the permanent record; remove the
  store-layer helpers only with a dedicated store cleanup that rewrites the
  fixtures that use them.
