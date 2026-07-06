# Vertical Slice v1 PRD: Login → Pay → Agent → Invite

Status: active. Pairing decision resolved 2026-07-03: no PIN (see Phase 3).
Phases 1-2 implemented (worktree `vertical-slice-p1-money-path`), awaiting
review.

Date: 2026-07-03.

## Problem Statement

finitecomputer-v2 has every subsystem of the self-serve product working in
isolation: WorkOS login, Stripe Billing v0 (hardened, test-clock E2E'd), Core
entitlements, runner launch on Docker and Phala, and a runtime that publishes
a Finite Chat `/invite` status endpoint. What does not exist is the continuous
product experience: a new user should be able to sign up, pay, watch their
agent launch, and pair their iOS Finite Chat app with it — without reading
docs, hitting a raw JSON endpoint, or waiting on an operator.

The deliverable is that golden path, demoable end-to-end on a real provider
with a real phone, with machine-readable evidence at each hop. This slice is
the vantage point for all future iteration.

## What Already Exists (verified 2026-07-03)

- WorkOS login/signup/callback routes, env-gated; billing setup panel;
  Stripe Checkout + Billing Portal server actions; hardened webhook sync
  (re-fetch on subscription events, double-subscribe guard, price check) and
  a Stripe test-clock E2E (`384bc08`, `40a9240`).
- Core: billing-gated agent creation (`BillingRequired`), count-based
  entitlements, full lifecycle (create/restart/recover/stop/destroy with
  offboarding), runtime facts including `published_app_urls` = the runtime's
  `/invite` status URL.
- Runner: Docker + Phala launchers; both wait for `/invite` readiness before
  registering the runtime.
- Runtime image: Hermes + finitechat CLI + finite-platform plugin;
  `health_server.py` serves `GET /invite` → `{ready, room_id, invite_id, url}`
  where `url` is a `finite://join?...` invite code (finitechat invite v1).
- Dashboard: billing panel, create-agent form, creation-progress states
  (queued / launching / failed-retry), a project card that links out to the
  raw invite URL. `scripts/local_create_agent_canary.sh` drives a wonky but
  real local demo.
- iOS Finite Chat: QR scanner + `finite://join` handling + PIN entry exist in
  the app (finitechat ADR 0006 flow).

## The Gap

1. The dashboard never renders the invite. The user's payoff moment — "scan
   this with your phone" — is a link to a JSON page.
2. The flow has seams: after Stripe Checkout returns, billing state depends
   on webhook arrival; after creation, invite readiness lags runtime
   registration. Neither seam has a designed waiting state.
3. Pairing security is undecided and currently contradictory (below).
4. No end-to-end evidence artifact covers signup → paid → launched → paired.

## Pairing security (resolved 2026-07-03: no PIN)

Decision: no PIN, per custody brief Decision 6. finitechat main has already
hard-cut the agent invite flow to the no-PIN hosted shape (`e81683e`,
`7ed872d`, `3482ea4`): invite code v1 carries no PIN; admission is an HMAC
join proof binding the invite token (in the URL, never seen by the
rendezvous server) to the joiner's exact identity and KeyPackage. The earlier
Option A/B framing in this doc cited ADR 0006 and the invite execution plan,
which describe the pre-cut PIN flow — those finitechat docs are stale
relative to code.

With no PIN, possession of the invite URL is admission. Three gaps make that
unacceptable as-is, and define Phase 3:

1. **Invites are not single-use in the hosted lane.** The `hermes invite`
   default is `max_joins = 8`, TTL 24h (`finitechat-cli/src/hermes.rs:74`).
   Protocol supports `--max-joins 1`; the runtime just doesn't pass it.
2. **The runtime caches the invite forever.** `health_server.py` writes
   `current-invite.json` once and serves it even after the invite is
   consumed or expired — a paired agent keeps advertising a dead (or worse,
   still-live multi-use) invite on a public endpoint.
3. **The invite is served on the public runtime URL.** Anyone who discovers
   the hostname can fetch the one live invite and win the pairing race.
   Single-use narrows this to a race; credential-gating closes it without
   any user-visible UX (server-to-server only — the user still just scans).

Also requested: the invite URI is too long/ugly for the copy-paste surface
(`finite://join?v=1&s=<pct-encoded-server>&r=…&i=…&t=<64-hex>&a=<npub>&n=…`).
Compaction (default-server elision and/or a packed v2 encoding) is
finitechat protocol-surface work — tracked as a finitechat work item, not a
v2 blocker; the dashboard truncates for display meanwhile.

## Acceptance Criteria

A fresh user with only an email address and a credit card, on the production
dashboard, can:

1. Sign up via WorkOS and land on the dashboard signed in.
2. See a billing setup panel; complete Stripe Checkout (promo codes work);
   return to a dashboard that resolves to "paid" without a manual refresh,
   even if the webhook is still in flight (bounded polling, not a blank
   state).
3. Create their agent with one action; watch queued → launching → ready
   states update without manual refresh.
4. On the ready agent card, see a QR code + copyable `finite://join?...`
   invite (no PIN), rendered in the dashboard, with "open in Finite Chat"
   for on-device Safari. The invite is single-use; after pairing the card
   shows paired state, not a live invite.
5. Scan with the iOS app, complete pairing, exchange a real Hermes message
   round trip.
6. Restart the runtime from the dashboard; pairing and chat history survive;
   the invite surface still works.

Non-goals for v1: push notifications (iOS polling/foreground is fine), team
seats, multiple agents per org, no-PIN pairing UX, agent web chat in the
dashboard, migration of legacy users.

## Constraints

Musts:
- Core stores no PIN, invite URL contents, or pairing secrets beyond
  credential hashes (custody brief rules).
- Billing gates stay in Core; the dashboard remains an adapter.
- QR rendering is self-contained in the dashboard (bundled QR lib, no
  external image service).
- Every phase lands with tests in the existing CI jobs; the runtime-image
  contract change (if Option A) lands in finitechat's repo with its own
  tests, versioned in the image contract doc.
- Read `apps/dashboard/AGENTS.md` before dashboard work: this Next.js
  version diverges from convention — consult `node_modules/next/dist/docs/`.

Must-nots:
- No new cross-repo source coupling: v2 consumes the invite contract via the
  health server JSON shape only; changes to that shape happen in finitechat.
- No polling loops without caps/backoff (billing sync wait, invite
  readiness).
- Don't regress the failed-launch retry path or billing v0 test coverage.

Preferences:
- Reuse the creation-progress panel patterns for the two new waiting states.
- Extend `local_create_agent_canary.sh` into the evidence harness rather
  than writing a parallel script.

Escalations (stop and ask):
- Pairing decision A/B needs explicit sign-off (custody sync).
- Any change to what Phala exposes publicly (health port surface).
- If webhook-race polling reveals a deeper Core consistency issue, surface
  it rather than papering over it in the UI.

## Decomposition

Phase 0 — Decide pairing. DONE: no PIN (above).

Phase 1 — Seamless money path (dashboard only). Checkout-return state
machine: `billing=success` → bounded server-side poll of Core billing until
active or timeout → auto-advance to create-agent. Tests: webhook-slow,
webhook-first, checkout-cancelled paths.

Phase 2 — Invite surface (dashboard only, works under either pairing
option). Agent-ready card gains: QR of the invite URL, copy button, "open in
Finite Chat" link, invite-not-ready waiting state (bounded refresh), error
state when `/invite` reports not-ready with error. Server action fetches
invite JSON (today: public URL; Phase 3 swaps in credential). Tests: render
states from fixture JSON; no client-side fetch of runtime origin (CORS +
credential hygiene).

Phase 3 — Pairing hardening (no user-visible UX change). Split by repo:
- finitechat repo: health server mints `--max-joins 1` with a short TTL;
  invite cache invalidated on consumption/expiry; `/invite` reports
  `{paired: true}` after first join instead of a dead invite; re-invite is
  an explicit action, not an automatic re-mint on GET. Its own tests.
- v2 repo: per-runtime pairing credential (relay-credential pattern) minted
  by the runner, hash in Core, owner-scoped Core endpoint for the dashboard;
  `src/lib/agent-invite.ts` fetch presents it; public hits get `{ready}`
  only. Dashboard gains paired-state and re-invite affordances.
- finitechat work item (not slice-blocking): compact invite URI encoding.

Phase 4 — Local harness + end-to-end evidence, as real code (product rule:
no product logic in .sh). Replace `scripts/local_create_agent_canary.sh`
with a workspace crate (e.g. `crates/finite-saas-local`): subcommands `up`
(postgres + Core + dashboard + optional runner/agent with dev auth — the
manual-testing server), `demo` (drive signup → test-clock subscription →
launch → invite-ready → CLI join → Hermes round trip), emitting a JSON
evidence file per run; unit-tested orchestration logic, `--json` output.
Promote the Docker lane into CI; document the manual Phala + real-iPhone
checklist as the release gate (finitechat phone-canary pattern).

Phase 5 — Durability of the payoff. Restart-from-dashboard keeps pairing
(state on `/data` durable mount); invite surface behavior across restart;
destroy hides the invite surface and invalidates the pairing credential.

Phases 1 and 2 are independent and can run in parallel; 3 depends on 0;
4 depends on 2 (full value after 3); 5 depends on 3.

## Evaluation Design

- Unit/CI: state-machine tests for both waiting seams; invite-render fixture
  tests; credential round-trip tests in Core + runner; health-server auth
  tests in finitechat.
- Integration: the extended canary run in CI (Docker lane) producing a
  machine-readable evidence JSON: timestamps for signup, checkout, webhook
  sync, entitlement grant, launch, invite-ready, join, first message.
- Release gate: one scripted Phala run + one human iPhone pairing per the
  finitechat phone-canary loop, evidence archived. "It demos" is not done;
  the evidence file is done.
- Regression guard: billing v0 acceptance tests and test-clock E2E stay
  green throughout.
