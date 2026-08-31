# Identity Rollout: Reconciled Plan

Status: **CHAT AMENDMENT AWAITING REVIEW — NOT PRODUCTION MUTATION AUTHORITY**

Owner: Paul
Driver: Kimi session (post-camp reconciliation)
Opened: 2026-07-30

This plan reconciles two prior planning documents from the
`agent-camp-followup` worktree with the post-camp grilling session and
supersedes them where they conflict:

- `chat-concurrency-presentation-repair-plan.md`
- `identity-service-product-boundaries-plan.md`

Scope discipline: everything below traces to a decision Paul explicitly made in
the reconciliation Q&A. Deferred items are listed as deferred, not scheduled.

## Current state after rollback (2026-07-31)

- PRs #336, #337, and #172 are merged. PR #172 is not deployed by this run;
  Brain navigation remains disabled.
- PR #355 (tool disclosure ownership) and #356 (background updates do not
  navigate) are merged and remain the accepted Chat baseline.
- PR #357 was reverted by #361, #358 was closed, and Sites PRs #359/#360 were
  reverted by #362. Current `origin/main` is the known-good base.
- Chat clarification is the active implementation stream. Sites proceeds in a
  separate stream using the unchanged Sites phases in this plan.
- Reverted worktrees are evidence only. New work starts from current
  `origin/main`; product code is not cherry-picked from them.
- Preparing a PR does not authorize merging or deploying it. Both require a
  separate explicit instruction from Paul.

## Locked decisions (Paul, 2026-07-30)

1. Merge PR #337 as-is as the first stage of a wanted trajectory: catch
   problems before deploy and iterate faster.
2. Merge PR #336. Human PRs may change human governance rules; this is one.
3. Validate PR #172 (brain) locally with devfinity, deploy, test with a new
   agent and Upgrade Canary on lat3 (possibly also a lat1 agent), human-test,
   then roll out fleet-wide with Brain navigation still disabled.
4. Merge PR #172 as-is. Old-brain reconciliation is NOT a priority (no "real"
   brain users). Hard requirement: break no non-brain functionality for
   existing users.
5. Sites work lands as THREE PRs (history + reviewability) but likely ONE
   deploy. Every deploy must be classified: dashboard-only / Core / servers /
   CLIs / agent image. This train eventually touches all of them.
6. One unified mailer (the identity service's), first-publication notification
   sent once per site, never per push.
7. Mailbox-click approval is sufficient authority (Google Docs is the UX
   benchmark, not traditional login). Shares and viewer sessions are
   PERSISTENT — no single-use UX. Once shared, shared.
8. Production quirks (below) must be documented durably enough to survive the
   whole journey.
9. Clarification and compaction are Hermes lifecycle concepts, not Finite Chat
   core protocol concepts. Consume real Hermes state at the adapter boundary;
   do not add `Clarification*`/`Compaction*` Chat types or infer them from
   emoji/prose. Reverted work is informational only.
10. Worktrees for all work. We take over PR #172's finish line with full
    authority; comment reasoning on changes; stop and ask Paul to ask Austin
    if a change diverges from his identity DESIGN rather than hardening it.
11. After 172 merges: sites slices and chat repair proceed in PARALLEL.
12. Prefer a true env-repair mechanism if it fits Phala / ends the env
    debacle; otherwise a one-off script plus a filed tech-debt item.
13. lat1/lat3 drift prevention is a separate infra project; this train keeps a
    written watch on WHY/HOW drift occurs to inform it.

## Design principles

1. **Every authenticated action is signed by an npub; durable authorization is
   product-owned.** Sites may durably authorize several npubs under one
   verified mailbox principal. Chat addresses, and Brain encrypts to, exact
   npubs. Mailbox addresses and NIP-05 names are proof/resolution inputs.
2. **Mailbox proof is sufficient authority** to append a new npub to a Sites
   keyset (and eventually revoke). A registered npub then authenticates by
   signing. (Paul's rule; the self-authorization-via-connected-Gmail concern
   was raised and accepted.)
3. **CLI behavior never branches on env-var presence.** Typed inputs
   (`--email` / `--nip05` / `--npub`) or server-derived config only. Identity
   endpoints are compiled-in production defaults; env vars are overrides for
   tests/self-hosting. Any genuinely required env var fails fast at startup,
   is enumerated in one documented place, and has a conformance test booting
   without it.
4. **Logic lives in typed contracts and server state**, not in UI derivation,
   emoji/prose classification, or special domain knowledge.
5. **Google Docs UX benchmark**: persistent shares and durable host-scoped
   viewer sessions; re-auth by mailbox proof on expiry. One session primitive,
   two proof methods (mailbox, native key).
6. **Fail closed, record repairs, never guess ambiguous state.** Migrations
   preserve all existing access; they may only narrow ambiguity, and record
   what they could not resolve.
7. **Independent CLIs.** No platform hatch required. When no trustworthy
   default mailbox exists, the agent asks the human.
8. **Canonical terminology** (update `finite-identity/CONTEXT.md` + ADRs):
   Mailbox Address (deliverable), NIP-05 Name (resolution only), Managed Agent
   NIP-05 (one name ↔ one agent npub, NOT a mailbox — never passed to an
   email-delivery flag), Sites Email Principal (durable owner keyed by verified
   mailbox), Authorized Sites Key (revocable human or agent npub), Originating
   Publisher (audit provenance npub).
9. **No contract stubs.** Dev sinks (outbox files, log-printed tokens) keep
   the real flow and swap only delivery. The hosted path is validated against
   the real staging WorkOS tenant via `devfinity up --workos-staging`.

## Sequence

### Phase 0 — merge queue (immediately after GO)

- Merge #336, then #337 (as-is per decision 1).
- All further work starts from the merged main, in worktrees.

### Phase 1 — PR #172 takeover, validation, rollout (driver: this session)

1. Rebase `codex/hybrid-wiki-search-slices-1-3` onto post-337 main.
2. Local validation with devfinity: brain create/invite/mount/collaborator
   flows; migrations 13–20 against a production-like brain DB copy; access
   before/after diff (equal-or-narrower only); assert brain nav disabled.
3. Fix what validation finds, in-branch, with reasoning comments. Escalate to
   Paul→Austin only if a fix diverges from the identity design (decision 10).
4. Deploy (servers + agent image classes). Canary: one NEW agent, then
   Upgrade Canary on lat3, then optionally one lat1 agent. Human-test.
5. Fleet rollout: runtime image carries updated skills; old-skill brain
   breakage in the window is ACCEPTED (decision: no compat shim) provided the
   new CLI can create a new brain and failure is loud and non-corrupting —
   verify that in step 2.
6. Brain navigation stays disabled throughout; re-assert in the dashboard
   browser test at every step.

### Phase 2 — Sites identity slices (parallel with Phase 3, after 172 merges)

**PR (a) — CLI hygiene and terminology.** Typed `--email`/`--nip05`/`--npub`
targets; passing a Managed Agent NIP-05 to `--email` fails with corrective
guidance and never attempts delivery or silently picks another key. Fold in
the `codex/fsite-agent-nip05-sharing` stopgap ALIGNED: keep the native-key
selection fix, but no env-presence branching and no env-scraped name
derivation (name comes from the identity service). Wire
`FINITE_BRAIN_INVITE_MAILER=dev` in devfinity. Terminology updates to
`finite-identity/CONTEXT.md` and ADRs. Class: CLIs + servers(config-only).

**PR (b) — Sites keyset model and reconciliation.** New Sites-owned tables:
`sites_email_principals` (verified mailbox), `sites_authorized_keys`
(↔ native principal, proof kind, revoked_at), publisher attribution columns
(`publisher_email_principal_id`, `originating_publisher_principal_id`).
Registration = fresh mailbox challenge + NIP-98 signature from the exact
npub; keyset add/revoke requires fresh mailbox authority. Reconciliation
migration: preserve every legacy owner/collaborator/share/visibility/URL/git
credential; establish email principals only from durable evidence (existing
verified mailbox proof, verified Core account ownership + agent association,
or fresh proof); convert legacy managed-agent-NIP-05 strings found in email
fields into native npub grants ADDITIVELY (never mail them); conflicts fail
closed into repair records; idempotent, producing
migrated/unchanged/conflict/needs_proof reports. Build the multi-product
identity conformance fixture here. Decide env-repair: general mechanism if it
fits Phala, else one-off script for the 21 stale `FINITE_BRAIN_*` agents +
filed debt (decision 12). Class: servers + CLIs.

**PR (c) — UX flows.** Project init default-mailbox order: (1) signing npub
already an Authorized Key; (2) hosted requester assertion carrying the
WorkOS-verified mailbox — EXTEND the lease/hosted-device path, which today
carries only the WorkOS user id; (3) explicit `--owner-email`; (4) structured
`requester_email_required` and the skill asks the human. Durable viewer
sessions replacing single-use redemption, one primitive for mailbox and
native proofs. Repair the private-site email-auth page: verify mailbox BEFORE
revealing share status; truthful "not shared" + Request Access; approval
creates an explicit share; persistent access per decision 7. First-publication
notification, once per site, via the identity mailer. Class: servers +
dashboard + CLIs + agent image (skills).

### Phase 3 — Chat concurrency and presentation repair (parallel stream)

PRs #355 and #356 completed the navigation and disclosure slices. Do not
redesign them; keep their tests as regression coverage.

**Clarification PR:**

- First inspect pinned Hermes and its Telegram and Discord adapters to see how
  they consume clarification, working, typing, thinking, and status callbacks.
  Working/typing/thinking are context, not additional deliverables.
- Reproduce open-ended and choice clarification through the real Hermes
  pending-question mechanism, Finite durable inbox, and two concurrent Chats.
- Hermes owns pending clarification state. The question and answer remain
  ordinary Chat messages; the Finite adapter owns exact route correlation.
- Ambiguous routes fail visibly and never fall back to Home or the active Chat.
- Remove or bypass existing emoji/prose inference where it interferes; do not
  replace it with another classifier or add a core Chat protocol type.
- Test locally in a real browser, then open the PR. Do not merge it.

**Compaction check, then parking-lot rule:**

- Inspect the pinned Hermes adapter API for a clean semantic compaction
  start/finish signal. Telegram and Discord must be reviewed, but they are not
  assumed to implement a compaction UX.
- If that signal is directly available, a separate bounded PR may project it
  through existing generic, exact-route, expiring Chat activity.
- If observing it requires marker/prose matching, a Hermes fork/upgrade, or a
  new Finite Chat protocol concept, move compaction UI to the parking lot and
  continue. Recheck when Hermes 0.19 or 0.20 is considered.

### Phase 4 — One deploy train

- Classify every step: dashboard-only / Core / servers / CLIs / agent image.
- Production boundary per doctrine: sanitized replay, pre-migration ACL
  export, named backups, empty-target restore with the same Recovery Set,
  access before/after diff, one existing-agent and one new-agent canary,
  mixed old/new CLI checks, explicit approval, documented rollback.
- First reconciliation canary: Paul's own account (`paul@finite.vip` mis-link,
  below).

## Local validation strategy (no contract stubs)

- Always work from current `origin/main` (+ PR branches), never stale
  checkouts.
- Hosted path: `devfinity up --workos-staging` with Paul's staging
  credentials (`WORKOS_STAGING_API_KEY` / `WORKOS_STAGING_CLIENT_ID`, held by
  Paul, never committed); staging AuthKit app must register the
  `127.0.0.1:13002` redirect URLs.
- Mailbox proofs: real flows against local identityd (`--mailer dev
  --dev-print-email-tokens`) and finitesitesd DevMailer outbox — dev sinks,
  not stubs.
- Harness gaps to close (composition, not new pieces): the email-half smoke
  (share → outbox → redeem → view), a browser-level owner-views-private-site
  run, and the old/new CLI mixed-version matrix.
- Gates: `just identity-conformance`, `just test`, `just dev smoke`.

## Production quirks register (durable facts — decision 8)

- `paul@finite.vip` is currently linked to the native key of Upgrade Canary
  0715. First reconciliation canary; do not "fix" by hand beforehand.
- 21 of 22 lat1 agents have stale `FINITE_BRAIN_*` URLs pointing at
  `https://finite.computer`; `fbrain doctor` falsely reports healthy there
  (dashboard proxies `/health`). Repaired per the Phase 2b env decision.
- Older agents may be missing managed NIP-05 bindings (Waffle Prime needed an
  operator repair 2026-07-27; binding backup at
  `/data/backups/identity/identity-20260727T120447Z.db` on lat1).
- Phala runner currently sets `agent_email: None` and skips identity binding
  entirely (`finite-saas-runner/src/phala.rs`) — the mailbox-proof fallback is
  the Phala story, so it must stay first-class.
- Retired agent data sets on lat1 must never be reactivated.
- Drift watch: record every observed cause of lat1/lat3 divergence during
  this train to inform the future host-parity infra project (decision 13).

## Deferred (Paul's explicit "eventually"s — NOT scheduled)

- Sites list UI; JIT open-in-new-tab polish beyond the durable-session
  primitive; Chat `@`-mention NIP-05 autocomplete.
- Compaction UI when pinned Hermes lacks a clean semantic adapter hook; revisit
  against Hermes 0.19/0.20 instead of adding local inference.
- Full Identity Contract typed-resolution rewrite + shared client libraries;
  static-gate machinery; capability matrix.
- Host-parity (lat1/lat3 anti-drift) infra project.
- Enabling Brain navigation.
- Electron/iOS pairing.

## Escalation points — stop and ask Paul before

- Merging or closing any implementation PR.
- Any change to PR #172 that diverges from Austin's identity DESIGN (Paul
  relays to Austin).
- Any production mutation, deploy, rollback, or canary.
- Upgrading or forking Hermes as part of the Chat clarification work.
- Introducing any new REQUIRED env var (design principle 3 violation).
- Emailing users beyond the two approved flows (first-publication record,
  Request Access approval).
- Granting any npub keyset membership without mailbox proof.
- Enabling Brain navigation, or touching Electron/iOS pairing.
- Merging the durable follow-up queue work into the chat repair.
