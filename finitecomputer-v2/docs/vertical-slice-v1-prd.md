# SaaS v1 PRD: Sign Up → Launch → Web Chat

Status: active first-pass product plan.

Date: 2026-07-09.

## Outcome

Ship the shortest credible Box1-parity SaaS path: a person signs up, gets an
approved entitlement, names an agent, launches it, and chats with it in the
dashboard. Restarting any one component must not strand the user or silently
create a different identity.
This slice is intentionally small enough to ship and iterate; the existing
Finite Chat experience remains the product northstar.

Finite is one product assembled from independently replaceable services and
binaries. A feature request must not turn the Agent Runtime image, Runtime
Management, or Runner into a product control plane.

## Golden Path

1. Sign up or log in through WorkOS and land in the personal dashboard.
2. Redeem an approved single-use Launch Code or complete Stripe Checkout, or
   see the already-active entitlement. Launch Codes remain the intentional
   white-glove and sponsored-access path; the paid customer run must exercise
   Stripe.
3. Choose an agent name and icon. Core assigns the standard hosting class from
   product policy; Kata is the first production Runner and Phala follows behind
   the same provider-neutral contract.
4. Launch one lockstep Finite Product Release. The dashboard shows bounded,
   understandable launch progress and a retryable failure state.
5. Open **Chat** in the dashboard. Account Auth opens the user's durable Hosted
   Web Device; the browser never receives its chat secret.
6. The Hosted Web Device contacts the agent through its Nostr profile and
   published KeyPackage, then creates or opens the canonical Room. Core,
   Runner, and Runtime Management do not broker chat. A temporary invite-based
   bootstrap may remain behind the product service while direct contact lands,
   but it is not a platform contract.
7. Exchange real messages through Finite Chat. Browser updates arrive through
   the resident event stream, not polling.
8. Restart the Finite Chat server, agent, or Hosted Web Device and resume the
   same identity, Room, and durable state.

Electron is a later **new Device**, not a second account or second chat product.
It uses the same login/account-linking flow and canonical chat UI, but keeps its
Device key and store locally, so it does not need the Hosted Web Device to run.
The launch dashboard must not depend on Electron's best-effort UX.

## Product And Identity Boundaries

- WorkOS gates the dashboard, billing, and Hosted Web Device. It is not a
  Nostr signer and may be replaced later without changing agent or chat
  identities.
- Each agent owns one Agent Principal npub/nsec in its durable Finite Home.
  `finite-identity` makes `finitechat`, `fsite`, and `fbrain` use that identity;
  Core, Runner, and dashboard never receive its nsec.
- The human's Finite Chat identity is separate and may eventually be imported
  by nsec. Each Hosted, Electron, or native Device has independent revocable
  Device state under that account.
- The Hosted Web Device is an honest trusted-server web-chat device, not a
  claim of browser E2EE or operator blindness.
- Runtime restart, replace, and stop preserve `/data`. Compute retirement never
  implies data purge. The first slice keeps an explicit Finite-assisted escape
  route while stronger recovery and operator-blind custody are proven.

## The Thin-Coupling Wall

Every feature proposal must pass this placement test before implementation:

1. Can it live in the dashboard, its owning product service, a stable CLI, or a
   Finite Skill? That is the default.
2. Does it behave the same when the unchanged release moves between local
   Docker, Kata, and Phala?
3. Does Runtime Management expose only generic release/readiness/health
   telemetry, with no feature-specific verbs, secrets, payloads, or commands?
4. Does Runner do only provider lifecycle: launch, inspect/adopt, restart,
   replace, stop/retire, and separately authorized purge?
5. Can a future Electron/native Device consume the same chat state and actions
   without depending on the Hosted Web Device?

If any answer is no, the feature is rejected or redesigned. In particular,
Google, Telegram, Sites, Brain, and skills do not earn special Runner operations
or Runtime Management messages. This wall supersedes earlier plans that put
product-feature commands or payloads on Runtime Management:

- Google and Telegram setup belongs to product UI plus a stable agent-facing
  CLI/skill and the owning service's explicit, revocable grants.
- Sites publish remains an agent `fsite` operation; list and preview, including
  chat result previews, belong to the Sites service and dashboard.
- Brain belongs to its authenticated dashboard client and product-scoped
  grants; encrypted Folder Key grants remain a separate Brain concern.
- New agents ship with a pinned `finite-skills` baseline. Agents update at
  their own pace with the simple `finite skills sync` workflow; no automatic
  skills control plane, Runner hook, or image rebuild is introduced.
- Runtime images change only to promote a tested lockstep product release, not
  to encode dashboard workflows.

## First-Slice Acceptance Criteria

A fresh email user can complete WorkOS auth, approved entitlement,
name/icon selection, launch, and a real dashboard chat turn without choosing
infrastructure, reading docs or raw JSON, installing Electron, or involving an
operator in the walkthrough. The user sees useful waiting and retry states at
entitlement, launch, and chat connection boundaries.

The same release boots with one resident Hermes sidecar and event-driven
inbound delivery. No Hermes polling or silent CLI fallback is allowed. Agent,
Hosted Device, and server state are durable enough that supported restarts
heal automatically and preserve the conversation. Secrets do not appear in
browser state, argv, logs, runtime facts, or release evidence.

The release manifest pins all first-party hosted services and binaries plus the
Hermes version. Promotion rejects unversioned or drifting components.

The internal production canary proves the normal path and does not pre-build a
general stuck-launch cancellation/reconciliation system. A stuck canary is a
failed canary and its concrete cause is fixed before retrying. A bounded user
escape with provider-cleanup and exactly-once entitlement semantics remains a
customer-admission gate.

## Current Status

Implemented in the current first-pass slice:

- [x] Standalone, per-account Hosted Web Device service with durable isolated
  state and internal authentication.
- [x] WorkOS-gated dashboard Chat surface backed by Finite Chat
  `AppState`/`AppAction`, with a server-side proxy and SSE update path.
- [x] Devfinity wiring and production Nix service/package definitions for the
  Hosted Web Device.
- [x] Runtime image lockstep checks, Hermes `0.18.2`, and strict resident-stream
  inbound behavior with reconnect instead of polling or CLI fallback.
- [x] A real server + agent + Hosted Web Device restart integration test for
  chat continuity at the service boundary.

Remaining launch gates:

- [x] Offer agent picture selection and persist the Core-assigned Runner class
  through the sealed creation draft and provider-neutral RuntimeSpec without a
  provider selector in onboarding.
- [ ] Run one actual full-stack browser canary through auth, single-use Launch
  Code redemption, agent creation, Runner launch, Hosted Web chat,
  send/receive, and restart. Exercise Stripe separately in the paid-customer
  run.
- [ ] Implement and prove the generic Kata adapter on finite-lat-1; run Phala
  unchanged as the fast-follow conformance target.
- [ ] Configure production secret references, build/promote the pinned images,
  deploy, and capture release evidence.
- [ ] Finish first-login human account-key bootstrap, bring-your-own-nsec
  import, Device linking, key backup, and an understandable recovery flow.
- [ ] Pass the outage matrix for Finite Chat server, Hosted Web Device, and,
  when reintroduced, Electron stopping and restarting independently.
- [x] Use direct agent profile/KeyPackage contact for Hosted Web chat rather
  than coupling the dashboard to the legacy invite action.
- [ ] Add Google, Telegram, Sites dashboard preview/list/publish, and Brain as
  product-owned follow-on slices without widening Runtime Management or Runner.

## Recovery TODO (Not A Slice Design Gate)

Recovery Snapshot format, off-host storage, Restic suitability, and long-term
key custody remain an explicit open question. Do not delay the working SaaS
loop to choose the final mechanism. For this slice, preserve state, retain the
Finite-assisted Recovery Authority, never couple teardown to purge, provide an
escape/export path where possible, and make no stronger privacy or recovery
claim than the evidence supports.

## Evaluation

- Unit/contract: Account Auth scoping, per-user Hosted Device isolation,
  browser action allowlist, SSE reconnect, stream-only Hermes behavior, and
  version-lock checks.
- Integration: real Finite Chat server, agent, and Hosted Device exchange
  messages before and after independent restarts with the same identities and
  Room.
- Product canary: the named human completes the blessed production run. Agents
  may record facts already available during that run, but a bespoke
  machine-readable report or new evidence instrumentation is not a gate. The
  human uses only the normal product; needing a worksheet, debug surface, or
  operator reconstruction is a failed canary rather than an evidence task. The
  internal canary completes real agent turns before and after a dashboard-
  initiated same-volume Agent Runtime restart in the same visible conversation;
  shared-service process restarts remain automated integration coverage.
- Architecture review: every new feature records its answers to the five-part
  thin-coupling test. A feature-specific Runtime Management or Runner change is
  a failed review, even if its demo works.
- Release gate: local full-stack first, then Kata, then unchanged Phala. A
  provider accepting a deployment is not evidence that the product works.
