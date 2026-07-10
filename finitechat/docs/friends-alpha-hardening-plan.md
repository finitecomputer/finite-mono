# Friends Alpha Hardening Plan

## Problem Statement

Finite Chat should become usable day to day by Paul and a small group of
friends for testing Finite Computer agents, Finite Sites, and future
Finite Brain workflows. The milestone is not a broad public launch. It is a
production-shaped hardening pass that makes the first real usage coherent:
agents can be provisioned, chat through Hermes reliably, view shared Finite
Sites without email friction, receive wake-only push notifications, use a
shared blob substrate, and survive normal restart and multi-device scenarios.

## Acceptance Criteria

- A new agent can be provisioned with one stable Agent Principal Key under one
  Agent Home, and `fsite`, `fchat`, and later `finite-brain` can converge on
  that identity model.
- `fchat` is the distribution and runtime boundary for the Finite Chat Hermes
  adapter.
- The day-to-day Hermes path uses a supervised Rust service, not Python-owned
  protocol state.
- Multi-user rooms with one directly invited agent are tested end to end.
- Hermes home channel is treated as a Hermes routing preference, not a Finite
  Chat membership concept.
- People/profile/avatar loading is instant from cache and stale-while-
  revalidate, following the Pika shape with Rust-owned state.
- Wake-only APNs push works on a physical iPhone: locked phone receives a wake,
  syncs, and shows the message after opening.
- Finite Blob has a provider-neutral API with scoped capabilities and a path to
  S3-compatible hosted storage, with Latitude as the first canary candidate.
- The in-app Finite Sites browser opens shared sites invisibly using direct
  local NIP-98 signing over HTTPS, with no relay dependency and no page access
  to private keys.

## Constraints

- Product provisioning and test shortcuts must stay visibly separate.
- The agent never receives the user's personal Nostr secret.
- Directly invited agents are P0. Human-to-agent delegation is designed for but
  not required before Friends Alpha.
- Hosted web is Friends Beta, not Friends Alpha. Browser MLS remains out of
  scope, and trusted hosted-device specifics should be revisited before that
  run of work starts.
- In-app Finite Sites auth must not publish to or query Nostr relays.
- Swift/native UI should render Rust-projected state and dispatch actions; it
  should not own profile relay fetching or cache semantics.
- Blob storage should hide provider details from product callers and agents.
- Any shortcut relied on for Friends Alpha must be recorded in
  `docs/technical-debt-ledger.md` with source, risk, first proof, and delete
  condition.

## Working Terms

- Agent Home: the filesystem root for one agent/runtime identity and local
  product state.
- Agent Principal Key: the agent-controlled npub/nsec shared by agent-side
  tools such as `fsite`, `fchat`, and future `finite-brain`.
- User Key: the human's personal key. It may live in the native app, but it is
  not shared with the agent.
- Delegation: a Principal-approved authorization letting an Agent Principal Key
  act on behalf of a human/contact principal with bounded capabilities.
- Direct Agent Principal: an agent invited or shared with as its own visible
  principal, similar to a human participant.
- Finite Blob: provider-neutral blob service contract used by chat, sites, and
  brain through scoped capabilities.

These terms are provisional until we consolidate terminology across
`finite-sites`, `finite-chat`, and `finite-brain`.

## Commit Discipline

Work should happen on `codex/friends-alpha-hardening`. Each phase below should
land as one or more focused commits with tests or docs matching the phase gate.
Do not batch unrelated phases into one commit. If a phase discovers a domain
rename or irreversible product decision, capture it in `CONTEXT.md` and, when
appropriate, an ADR before implementing broad code changes.

## Phase 0 - Planning, Terms, And Cross-Repo RFCs

Goal: make the milestone self-contained enough that implementation work does
not invent product semantics ad hoc.

Work:

- Capture Friends Alpha goals and non-goals in this plan.
- Reconcile identity terminology with `../finite-sites/CONTEXT.md`:
  Principal, Native Principal, Agent Key, Agent Delegation, Key Challenge.
- Draft a short Finite Sites RFC for native viewer auth:
  direct local NIP-98 signing over HTTPS, no relays, host-scoped viewer cookie.
- Draft a short Finite Blob RFC:
  provider-neutral API, scoped capabilities, central allowlist, S3/Latitude
  backend canary.
- Update `CONTEXT.md` or ADRs only after terms are settled.

Acceptance:

- The plan names P0/P1 scope and non-goals.
- Follow-up RFCs list endpoint shapes, trust boundaries, and test matrices.
- No implementation code changes are hidden in the planning commit.

Commit checkpoint:

- Commit the plan and any accepted RFC/doc updates before Phase 1.

## Phase 1 - Agent Home, Identity, And fchat Distribution

Goal: make `fchat` the agent-side installation and runtime boundary.

Work:

- Define Agent Home path resolution and precedence.
- Add explicit identity commands:
  `fchat identity init`, `fchat identity show`.
- Add `fchat hermes install` as the supported adapter installation path.
- Keep product provisioning explicit; reserve dev/test shortcuts under clearly
  named `dev` or fixture commands.
- Align with `fsite` identity resolution without silently treating the agent as
  the human owner.
- Decide migration or aliasing from the current `finite-sites` identity path.

Acceptance:

- A fresh agent home can be initialized and reports one stable agent npub.
- The Hermes adapter can be installed by `fchat` without manual plugin copying.
- Missing identity causes clear failure; `serve` does not silently mint a new
  principal key.
- Multiple Agent Homes produce isolated principal keys and state.

Evaluation:

- CLI unit tests for path resolution, env overrides, missing identity, corrupt
  identity, and multi-home isolation.
- Integration test installing the Hermes plugin into a temp Hermes home.
- Negative tests proving test bootstrap commands are not invoked by product
  commands.

Commit checkpoint:

- Commit identity/path semantics first, then Hermes install packaging.

## Phase 2 - Supervised Hermes Service

Goal: replace the day-to-day CLI-per-call path with a supervised Rust service.

Work:

- Implement `fchat hermes serve` as the main bridge process.
- Keep Python Hermes plugin thin: start/check the service, stream inbound
  messages, send outbound actions, and report health.
- Add authenticated loopback API or stream boundary.
- Add `/healthz` or equivalent process health check.
- Let `fchat` do best-effort keepalive.
- Expose enough control for `finitec` to health-check and request restart.
- Preserve CLI-per-call adapter only as a fallback/test harness if useful.

Acceptance:

- Hermes can send and receive through the Rust service.
- Restart does not duplicate acknowledged messages or lose pending turns.
- Hangs and process death are detected and surfaced as agent-runtime
  degradation.
- Python owns no MLS state, identity state, blob state, or protocol decisions.

Evaluation:

- Service lifecycle tests: start, health, stop, parent death, orphan cleanup,
  restart after crash.
- Message tests: inbound stream, ack ordering, dedup, backpressure, bounded
  queues, invalid payloads.
- Media tests: encrypted blob refs human-to-agent and agent-to-human.
- Fault tests: partial writes, slow poll, corrupt state, unavailable server,
  duplicate delivery after restart.
- Clippy with `cargo clippy --all-targets -- -D warnings`.

Commit checkpoint:

- Commit service skeleton and health first.
- Commit message/ack path next.
- Commit lifecycle and fault tests before promoting it as default.

## Phase 3 - Hermes Product Semantics

Goal: make Hermes behavior match real agent usage.

Work:

- Support direct agent principal rooms.
- Test two humans plus one agent in one room.
- Preserve sender identity so Hermes does not collapse humans into one actor.
- Support Hermes home channel as a Hermes routing preference.
- Expose API for Hermes to set/show home channel when it asks in chat.
- Persist home channel in Agent Home.

Acceptance:

- One agent can participate in a group room with two users.
- The agent receives correct sender identity and replies into the same
  room/thread.
- Acks remain per message/cursor.
- Home channel can be unset, set to a 1:1 room, and set to a group room if
  Hermes permits it.
- Restart preserves the home-channel setting.
- Deleted/unavailable home channel fails clearly and does not leak messages to
  a fallback room.

Evaluation:

- Hermes adapter tests for multi-sender mapping.
- Live Hermes gateway demo with real model behavior, not only echo handler.
- End-to-end simulator test for human/group/agent message flow.
- Regression test for home-channel persistence and unavailable room handling.

Commit checkpoint:

- Commit group-room support tests separately from home-channel API.

## Phase 4 - Pika-Style People, Profiles, And Avatars

Goal: make the People tab and profile UI instant and reliable.

Work:

- Move follow/contact/profile state fully into Rust runtime projection.
- Remove Swift-owned relay profile fetching from the product path.
- Persist followed contacts and profile metadata locally.
- Hydrate People UI from cache immediately on launch.
- Refresh in the background, stale-while-revalidate.
- Cache avatars with Pika-style dedicated profile image storage:
  bounded download size, timeout, max concurrency, tmp cleanup, atomic writes,
  resize to display-safe JPEG, and `file://...?v=<mtime>` projection.
- Keep profile avatar cache separate from encrypted chat/blob storage policy.
- Add basic in-app profile editing where product semantics are already clear.

Acceptance:

- Already-fetched contacts render instantly while offline.
- Cached avatars render instantly when available.
- Empty or failed refresh does not wipe good cached contacts.
- Updated names/photos appear after refresh without UI cache staleness.
- Swift renders projected rows and does not fetch Nostr relays directly.

Evaluation:

- Rust restart tests for profile/contact/avatar cache.
- Negative tests for empty refresh, unavailable network, stale profile, corrupt
  avatar tmp file, oversized image, download timeout, and URL change.
- iOS UI test proving People tab loads cached rows before refresh completes.
- Manual comparison against Pika cache behavior where useful.

Commit checkpoint:

- Commit Rust cache model first.
- Commit Swift projection migration second.
- Commit avatar materialization and UI tests third.

## Phase 5 - Wake-Only Push Notifications

Goal: make physical phones wake and sync without exposing plaintext to push
services.

Work:

- Add iOS APNs token registration to home server.
- Implement or stand up pusher daemon draining `push_outbox`.
- Send wake-only payload `{room_id, seq}`.
- Keep badge/preview client-side or defer to NSE later.
- Write Apple setup walkthrough for certificates, entitlements, bundle IDs,
  environments, and server secrets.

Acceptance:

- Device registers and removes push token.
- Server stores tokens per account/device and drops them on revocation.
- Locked physical phone receives wake-only push for a new message.
- Opening app syncs and shows the message.
- Payload contains no plaintext message body, sender name, or attachment
  metadata.

Evaluation:

- Server tests for token registration/removal/revocation.
- Pusher tests for outbox drain, APNs error handling, retry bounds, and stale
  token cleanup.
- Simulator-first tests where possible; physical phone gate before friends.
- Manual Apple walkthrough executed once from clean state.

Commit checkpoint:

- Commit server/pusher plumbing before iOS entitlement work.
- Commit Apple walkthrough with the first passing physical-device proof.

## Phase 6 - Finite Blob Shared Substrate

Goal: give chat, sites, and brain one useful blob story without leaking bucket
credentials or product policy.

Work:

- Define Finite Blob provider-neutral refs and operations.
- Centralize initial allowlist in Finite Computer core:
  principal, enabled products, optional limits, expiry, environment.
- Mint scoped upload/download capabilities or signed URLs.
- Keep direct S3 credentials inside trusted blob service/infra only.
- Preserve separate product policies:
  encrypted chat attachments, site assets, brain artifacts, profile avatars.
- Add S3-compatible backend seam.
- Run Latitude as first hosted storage canary if performance and operations
  look good.

Acceptance:

- finite-brain can request a scoped upload/download path without knowing bucket
  details.
- Chat attachments still use encrypted blob refs and do not expose plaintext
  metadata.
- fsite assets can use the shared backend without inheriting chat attachment
  semantics.
- Usage can be attributed to principal/product for future billing.

Evaluation:

- Unit tests for capability scope, expiry, wrong principal, wrong product,
  quota/limit rejection, and replay.
- Integration tests for local disk and S3-compatible backends.
- Latency/cost canary for Latitude across iPhone, agent runtime, sites server,
  and brain workload.

Commit checkpoint:

- Commit API/ref model first, backend seam second, Latitude canary third.

## Phase 7 - In-App Finite Sites Browser Auth

Goal: let the native app open shared Finite Sites invisibly for the user.

Work:

- Add finite-sites native viewer auth endpoint:
  `POST /_finite/auth/native-session`.
- Use direct local NIP-98 signing from the native app over HTTPS.
- Do not involve relays, relay queries, remote signer prompts, or page
  JavaScript.
- Verify exact URL, method, payload hash, timestamp freshness, site host, and
  signer pubkey.
- Map signer to a Native Principal and check site access.
- Mint existing host-scoped HttpOnly viewer cookie.
- Redirect to `return_to`.
- Keep current email magic-link UX unchanged for normal external viewers.

Proposed request:

```json
{
  "purpose": "finite_site_view_session",
  "return_to": "/path",
  "client": "finite-chat-ios",
  "nonce": "<client-random>"
}
```

Acceptance:

- Opening a shared private site from the native app does not ask for email.
- The web page never receives the nsec or signed event.
- Revocation takes effect on the next request because serving re-checks access.
- Public sites still work without auth.
- Unshared private sites show the normal login/unauthorized surface.
- The ceremony works without any Nostr relay network.

Evaluation:

- finite-sites endpoint tests for valid session, stale signature, URL mismatch,
  method mismatch, payload mismatch, wrong host, unshared principal, revoked
  share, malformed return path, and replay/nonce bounds if nonce persistence is
  added.
- iOS browser test proving cookie is set and page loads without JS key access.
- Manual test with network access to relays disabled.

Commit checkpoint:

- Commit finite-sites RFC/doc first.
- Commit server endpoint and tests before iOS browser wiring.

Progress:

- Done in `../finite-sites` branch `codex/native-viewer-auth`: RFC/docs,
  native viewer session endpoint, Principal viewer cookies, nonce replay
  checks, sharing by native pubkey, and endpoint/e2e tests.
- Done in `finite-chat`: UniFFI helper for exact finite-sites native-session
  NIP-98 proof generation, in-app WebKit browser sheet, URL interception from
  chat transcript links, invisible cookie preflight, and public-site fallback.
- Still pending for the Phase 8 gate: manual proof against a deployed/shared
  private site with relay access disabled.

## Phase 8 - Friends Alpha Integration Gate

Goal: prove the whole path works together before inviting friends.

Work:

- Run clean setup from a new Agent Home.
- Install Hermes adapter through `fchat`.
- Start `fchat hermes serve` under finitecomputer/finitec supervision.
- Invite agent to 1:1 and group rooms.
- Send text and media both ways.
- Set and use Hermes home channel.
- Open shared fsite in native browser without email flow.
- Register APNs and verify locked-phone wake.
- Exercise Finite Blob upload/download from at least chat and one other product
  caller.

Acceptance:

- Paul can use the app for a normal day without manual database surgery,
  plugin copying, or relay-dependent fsite browser auth.
- A friend can join, chat with Paul and the agent, receive messages after
  restart, and view shared sites through the intended path.
- Known shortcuts are documented with delete conditions.

Evaluation:

- One simulator run for deterministic UI/product coverage.
- One physical iPhone run for push and real device behavior.
- One finitecomputer runtime run for supervised agent behavior.
- One failure drill: kill/restart Hermes service, pusher, and app runtime.
- Execution checklist and evidence table:
  `docs/friends-alpha-integration-runbook.md`.

Commit checkpoint:

- Commit runbook and final integration evidence after the first full pass.

## Friends Beta - Trusted Hosted Web Device

Status update 2026-07-09: promoted from deferred Friends Beta work to a Finite
Computer SaaS launch requirement by ADR 0011. The candidate shape and tests
below are now launch inputs, not a later milestone.

Goal: provide practical web chat after Friends Alpha, without making browser
MLS part of the product scope.

This work is intentionally deferred because Friends Alpha is about people
talking to people and directly invited agents through the native Finite Chat
app. Revisit the exact product and security specifics before launching this
run of work.

Candidate shape:

- Run one Rust client process per hosted web user/principal.
- Store one SQLite file per hosted device.
- Give the hosted device its own device key and explicit room membership.
- Use the hosted device for bootstrap and convenience, not as a replacement for
  native local E2EE.
- Serve the web UI from the hosted client process or a thin frontend over it.
- Make trust posture visible in docs and internal product copy.

Candidate acceptance:

- Hosted device can be added, sync messages, send messages, restart, and
  recover.
- Hosted device cannot read rooms it was never added to.
- Revoking hosted device removes future access.
- Native app plus hosted device can coexist for the same user principal.
- This is not described as browser E2EE.

Candidate evaluation:

- Multi-device Rust tests for hosted device add/sync/send/restart/revoke.
- Negative test for unjoined room access.
- Web integration test for bootstrap room and basic chat.
- Product trust-mode doc update.

## P1 After Friends Alpha

- Human-to-agent delegation UX across chat, sites, and brain.
- Rich push previews with Notification Service Extension and shared profile
  cache access.
- Billing-shaped allowlist metadata and monthly npub authorization.
- Latitude production rollout if canary results justify it.
- Browser/site editing UX for agents beyond viewer auth.
- External signer support for native fsite browser auth.
- Pure browser MLS/WASM exploration, only after the hosted-device path is
  stable.
