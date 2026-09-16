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
  `request_link`); general request limiting remains deferred. The Fly
  deployment does not inherit the legacy Cloudflare proxy's protections.
- **Risk**: API-plane brute force and serving-plane floods.
- **Proof**: `login_limiter` covers login, access-request and viewer-session
  issuance; other control-plane operations and content serving have no
  general request budget.
- **Delete condition**: per-IP budgets on the API plane (project init attempts and
  git deploys per pubkey per hour), plus qualified serving-plane limits at
  the Fly edge or service.

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
- **Delete condition**: the
  [snapshot-and-Borg job](../../infra/runbooks/sites-borg-recovery.md) is deployed
  and the complete Recovery Set has restored from rsync.net onto an empty
  target. Local tests alone do not establish independent durability.

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

## 8. Static-only Sites boundary

Under [ADR 0028](adr/0028-static-only-sites-platform-service.md), the Sites
service and CLI support static Sites only. App/document output kinds, app
runners and proxies, and wake-on-request are outside that boundary.

Dynamic compute belongs in a separate product boundary. Retained legacy
app/document consumers stay on their existing service until retirement; they
are not converted into static Sites during cutover.

## 9. RESOLVED — Project Repository pushes use durable post-receive events

Project Repositories now install a `hooks/post-receive` helper that records
bounded durable git ref-change events before the Git client sees success.
`finitesitesd` reconciles pending events after receive-pack and at daemon
startup. Tests cover real `git clone`/`git push`, ignored non-deploy refs,
missing output failure, restart reconciliation after a ref update before
deploy, and idempotent replay after Version creation before event
acknowledgement.

## 10. Test-fixture reconciliation helpers

- **Source**: engine and store test fixtures use store-layer reconciliation
  helpers to exercise identity/grant invariants.
- **Risk**: removing the helpers without replacing their callers would lose
  coverage, including rejection of automated evidence for revoked keys.
- **Delete condition**: replace those fixture callers with equivalent invariant
  coverage before removing the helpers. They are not a daemon command or a
  startup migration.

## 11. Retained legacy viewer-session exchange

- **Boundary**: dashboard account previews select one of two fixed configured
  Sites origins using the existing allowed site hostname distinction. Until
  cutover, legacy outputs retain their registry; v2 static sites use theirs. No retry
  across registries, new roster, or grant copy is introduced.
- **Delete condition**: remove legacy selection and request spelling after
  legacy previews and Hosted Chat requester consumers are retired. Until then,
  paired exchange tests must prove requests and failures stay on their backend.
- **Contract**: [ADR 0029](adr/0029-account-session-viewer-bridge.md).
  Production cutover requires separate authorization.
