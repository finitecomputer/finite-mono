# Hosted Web Chat Continuity (Slim)

Status: ACTIVE

Sequence note: Paul explicitly directed this run's Borg work on 2026-07-13;
Stripe Checkout Readiness is PAUSED until this run reaches Ready-for-Paul.

Owner: Paul

Opened: 2026-07-13

Expires: 2026-08-28

Acceptance: Using a dedicated synthetic account with several Topics, Chats,
and an encrypted attachment, every lifecycle event we actually perform —
browser reload, dashboard deploy, Hosted Web Device and Finite Chat server
restart, owner-claim failure and retry, Runtime restart/upgrade — leaves the
same canonical Agent Room and every retained conversation reachable. Then a
real encrypted off-host snapshot restores the Hosted Web Device, Finite Chat
server, and SaaS Core onto an empty target: same identifiers, readable
history and attachment, replayed owner claim, and one fresh Agent turn. Paul
performs the final browser checks; green timers and automated tests alone do
not claim acceptance.

## Why

On 2026-07-13, Sol 2 replied in older Chats shown under `Previous
conversations` but initially did not reply in newly created top-level Topics
and Chats. It later began replying there too. The available evidence did not
identify the causal transition. An earlier version of this run incorrectly
declared a split Room/Topic target to be the root cause; that claim is retracted
below. The confirmed remaining product defect is that selecting a Chat in the
sidebar does not update the chat pane until the pane receives another local
interaction.

The incident established that speculative binding migration is itself an
availability risk. Retained state must not be reclassified or rewritten from a
selection cursor or arbitrary identifier order merely because a newer product
model expects a canonical binding.

Separately, the audit found a placeholder Borg destination, a failed last job,
and live SQLite copies instead of a recovery contract. This run now configures
the real destination, but there is still no restore-proven off-host copy until
the operator activation and empty-target drill pass.

## Rules (the durable decisions)

1. **One canonical Agent Room** per (Project, human Principal, Agent
   Principal), recorded as a versioned encrypted binding owned by
   `finitechat-hosted-device`. Once written, ordinary product flows treat the
   whole binding as immutable: open validates and uses it without reconciling
   or rewriting it. Selection is only a cursor; it can never create, choose,
   replace, or repair the binding.
2. **Recovery never creates a Room.** Delete `/fresh` outright. Bootstrap for
   an existing Project opens the recorded binding before any Runtime contact;
   missing or ambiguous retained state fails closed as recovery-required. The
   explicitly authorized new-Project case is the initialization exception in
   rule 3.
   Owner claim gates sending, never reading history, and always travels through
   the canonical Room.
3. **The visible conversation set may only grow.** Rooms already recorded as
   associated are never deleted or merged; they appear as `Previous
   conversations`. There is no automatic Room reconciliation, including legacy
   selection or migration. An already-valid binding reopens byte-for-byte
   unchanged. The Project-creation workflow must durably authorize initial chat
   bootstrap before ordinary chat load; only that authorization may create the
   initial Room. Bootstrap uses a sealed staged journal. Before any server
   mutation it records the exact Room create request, including the intended
   Room id and MLS group id. It then records the claimed Agent KeyPackage before
   Room creation and records the exact prepared add-member commit before submit.
   If the server accepted Room creation but the Device failed before saving the
   matching local MLS group, restart replays the same journaled request and
   group id. A crash retry therefore resumes only the intended Room and exact
   journaled artifacts.

   If Core committed creation but the dashboard lost the authorization
   response, ordinary chat load must not grant authority. A user-visible
   `Finish chat setup` action may replay only that lost handoff: a fresh Core
   read must prove the exact Account-owned Project and exactly one durable
   creation request for it in `requested`, `launching`, or `running` state
   before the action writes authorization. It never scans or chooses Rooms.
   Missing authorization or ambiguous unbound retained state fails closed
   without creating, choosing, reclassifying, or binding a Room. Ordinary
   protocol sync may still converge already-authorized messages and Welcomes.
   Selection and ordering never confer binding authority.
4. **New chat = new Chat in the canonical Room**, idempotent per client
   intent key. It never creates a Room.
5. **Snapshots are service-consistent and off-host.** Use SQLite's backup
   API / a brief write fence — never copies of live db/WAL files. One
   snapshot = Hosted Web Device identity + encrypted client state, the whole
   Finite Chat server SQLite, a SaaS Core pg_dump, and a manifest with
   hashes. Encrypted, real Borg destination, passphrase held off-host, and no
   automated prune/compact from the production host. The archival credential
   should be destination-restricted append-only, but its current broader access
   is accepted hardening debt rather than an admission blocker. No plaintext
   bodies, secrets, or live ids in logs or manifest metadata.
6. **Recovery readiness = a proven empty-target restore**, not a green
   timer. The separately retained Agent Runtime is out of scope and must
   merely reconnect; full-host loss remains a separate gate.

## Queue

Work top-down.

### P0 — Fix the bug

- Failing regressions first: selecting each sidebar Chat updates the pane
  immediately without a second click; several Topics/Chats, failed claim, and
  retried load leave Room count, canonical Room id, and reachable Chat ids
  unchanged.
- Delete `/fresh` and its fixtures; no flag or fallback. Keep the honest normal
  actions retry load, retry claim, and new Chat. Show `Finish chat setup` only
  for the narrowly detected lost-creation-authorization case; it is not a Room
  recovery or migration action.
- Remove the parked public `/dashboard/device-link` page and its browser-facing
  approve/status APIs; keep device-link protocol support internal until a later
  explicitly authorized client run needs it.
- Persist the canonical binding in `finitechat-hosted-device`; validate it
  without rewriting it on reopen; remove `StartProfileChat`'s selected-Room
  preference.
- Load retained history before Runtime contact; test with the contact
  endpoint down and the model non-interactive.
- Remove automatic Room reconciliation, including legacy selection/migration.
  Test unchanged reopen for an already-valid binding, one-time authorization
  from the Project-creation flow, and the exact journal order: persist the Room
  create request and MLS group id before any server mutation, persist the
  claimed KeyPackage before Room creation, and persist the exact prepared
  add-member commit before submit. Regress the crash after the server accepts
  Room creation but before the Device saves the matching local MLS group; retry
  must replay the exact request and produce only the intended Room. Also prove
  failure without binding/Room mutation when authorization is absent or
  retained state is ambiguous. Prove ordinary load cannot authorize; prove the
  explicit `Finish chat setup` action requires a fresh Core-owned exact Project
  plus one durable `requested`, `launching`, or `running` creation request and
  performs no Room scan or selection. Normal protocol convergence remains
  enabled. Project the sidebar across already-recorded canonical + associated
  Rooms and fail the release on any retained-vs-visible mismatch.

### P0 — Real backups and one proven restore

- Service-owned consistent snapshot commands; Borg archives those artifacts
  (plus the pg_dump) to a real encrypted off-host target without production-host
  pruning; snapshot every 15 minutes, alert on failure or age > 30 minutes.
  Record destination-enforced append-only credentials as recommended hardening.
- Restore onto an empty target in isolated mode (public traffic and outbound
  side effects off), verify identifiers/history/attachment/claim/fresh turn,
  and reject corrupt, partial, or wrong-key snapshots without touching the
  target. Document the drill as a runbook and repeat it before paid
  admission.

### P1 — Consolidate and request acceptance

- Lifecycle sweep with the synthetic account: restart/deploy/upgrade each
  covered service and diff the identifier set before and after.
- ADR for the canonical binding + fail-closed bootstrap rule; update ADR 0011 and
  CONTEXT.md; add a read-only-first `Chats appear missing` runbook.
- Update the admission checklist: chat continuity/recovery failure blocks
  paid admission regardless of Stripe.
- Deploy the accepted revision under separate production-mutation authority,
  then produce the exact Acceptance Request defined in `README.md`: deployed
  revision, synthetic account and URL, lifecycle/identifier observations,
  selected Borg archive, empty-target location, stop conditions, and estimated
  minutes. Paul executes it and the acceptance statement at the top last.

## Production evidence — 2026-07-13

- Observed: Sol 2 replied in older Chats displayed under `Previous
  conversations`, while newly created top-level Topics and Chats initially did
  not receive replies. The retained messages remained visible. Sol 2 later
  began replying in the top-level conversations as well; this run has no
  read-only evidence identifying why.
- Retracted: this run previously called a canonical-Room/legacy-Topic split the
  root cause and treated source revision `3857559` in deployed revision
  `a350b42` as its correction. That diagnosis did not match the full symptom
  set and was not proven. The historical deployment record below does not
  validate that diagnosis or authorize further state migration.
- Separately observed: switching Chats in the sidebar can leave the chat pane
  showing the prior Chat until the pane is clicked. This is a UI state-
  propagation defect and does not justify changing Room bindings or retained
  conversation state.
- Before the replacement binding code is deployed, a count-only read-only
  inventory must compare Core Projects with a reachable Agent contact against
  exact hashed binding filenames. The 2026-07-13 preflight found every such
  Project already bound: zero reachable Projects were unbound. No legacy Room
  selection, repair, or production state rewrite was performed.
- The immutable-binding, exact Room-create replay after server acceptance but
  before local save, add-member journal resume, lost-authorization, and sidebar
  regression test results are local evidence only until the replacement
  revision is deployed and the lifecycle sweep is repeated against it. They do
  not explain Sol 2's observed recovery and do not satisfy this run's production
  acceptance.

- Deployed revision: `a350b42`; Nix system closure:
  `/nix/store/kg0wdxilbjqh4wb5bx9gfmyzr4sam5fd-nixos-system-finite-lat-1-25.11.20260630.b6018f8`.
- Dashboard image:
  `ghcr.io/finitecomputer/finite-saas-dashboard@sha256:e8195f83980c3b8d75bef6aa1c6832522408c9d724eee2c8de9f8a126b271e51`.
- The finitecomputer rsync.net credential bundle was copied byte-for-byte to
  finite-lat-1 without entering Git. Snapshot and offsite age checks pass;
  application services and public health endpoints are healthy. The new
  repository's encrypted repokey was exported to the ignored off-host
  finitecomputer secrets directory and its temporary on-host export removed.
- Selected archive:
  `finite-lat-1-hosted-web-chat-2026-07-13T15:00:05` in the dedicated
  `finitecomputer/finite-lat-1` repository. Its pre-create manifest check and
  create completed successfully, and a subsequent remote listing found it.
- The reused SSH credential accepted an arbitrary remote command. It should be
  replaced or restricted to append-only, but Paul explicitly accepted this as
  hardening debt on 2026-07-13. The empty-target service restore has not been
  completed, and Paul's browser lifecycle checks remain, so this run is not yet
  Ready-for-Paul.

## Acceptance Request — blocked on retained queue prerequisites

- **Revision:** to be recorded after the retained fixes and lifecycle proofs;
  the historical `a350b42` deployment does not satisfy this run's acceptance.
- **Where:** `https://finite.computer`, `https://chat.finite.computer`,
  finite-lat-1, the dedicated synthetic account, and an empty isolated restore
  target. Secrets remain only at the paths named in the recovery runbook.
- **Time:** estimate 20 minutes for Paul's final browser lifecycle checks after
  the automated empty-target drill passes.
- **Steps and observations:** reload every retained Chat; restart/deploy each
  covered service; exercise failed and retried owner claim; verify the same
  canonical Room and retained Chat set after each action; then inspect the
  restored isolated account, attachment, replayed claim, and fresh Agent turn.
- **Pass:** the Acceptance statement at the top of this run, using encrypted
  identifier evidence from the synthetic account and the selected archive.
- **Fail/stop:** any identifier-set change, unreachable retained Chat,
  unreadable attachment, claim divergence, restore mutation before complete
  verification, or archive/extraction failure. Capture count-only/read-only
  evidence and stop; do not switch restored traffic or admit paid users.

## Out of scope

Electron, Stripe, Runner/Kata/Phala changes, Agent Runtime snapshotting,
selective row-level restore, group-chat redesign, User Recovery Key /
operator-blindness, retention automation beyond the alerting above, and any
production mutation, deploy, or traffic switch without separate explicit
authorization. Destructive tests use the synthetic account only.

## Governing documents

- [`docs/monorepo-doctrine.md`](../monorepo-doctrine.md)
- [`docs/adr/0001-recoverability-precedes-operator-blindness.md`](../adr/0001-recoverability-precedes-operator-blindness.md)
- [`finitechat/docs/adr/0011-hosted-web-chat-uses-a-revocable-device.md`](../../finitechat/docs/adr/0011-hosted-web-chat-uses-a-revocable-device.md)
- [`finitecomputer-v2/CONTEXT.md`](../../finitecomputer-v2/CONTEXT.md)
- [`infra/nixos/modules/backups.nix`](../../infra/nixos/modules/backups.nix)
