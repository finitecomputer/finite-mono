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

A 2026-07-13 production audit found intact chat history hidden behind a
duplicate Agent Room: the `/fresh` recovery action created a second
exact-member Room via `StartGroupChat`, `StartProfileChat` preferred the
currently selected Room, and the sidebar only projected the selected Room.
Data was retained but unreachable — for a paying user that is loss.

Separately, the audit found a placeholder Borg destination, a failed last job,
and live SQLite copies instead of a recovery contract. This run now configures
the real destination, but there is still no restore-proven off-host copy until
the operator activation and empty-target drill pass.

## Rules (the durable decisions)

1. **One canonical Agent Room** per (Project, human Principal, Agent
   Principal), recorded as a versioned encrypted binding owned by
   `finitechat-hosted-device`. Selection is only a cursor; it can never
   create, choose, or repair the binding.
2. **Recovery never creates a Room.** Delete `/fresh` outright. Bootstrap
   opens the recorded binding before any Runtime contact; missing or
   ambiguous state fails closed as recovery-required. Owner claim gates
   sending, never reading history, and always travels through the canonical
   Room.
3. **The visible conversation set may only grow.** Legacy duplicate Rooms
   are never deleted or merged; they appear as `Previous conversations`.
   Migration picks the canonical Room deterministically (existing valid
   binding wins; else oldest exact-member Room, Room-id order as tiebreak)
   and is idempotent under crash and rerun.
4. **New chat = new Chat in the canonical Room**, idempotent per client
   intent key. It never creates a Room.
5. **Snapshots are service-consistent and off-host.** Use SQLite's backup
   API / a brief write fence — never copies of live db/WAL files. One
   snapshot = Hosted Web Device identity + encrypted client state, the whole
   Finite Chat server SQLite, a SaaS Core pg_dump, and a manifest with
   hashes. Encrypted, append-only, real Borg destination, passphrase held
   off-host. No plaintext bodies, secrets, or live ids in logs or manifest
   metadata.
6. **Recovery readiness = a proven empty-target restore**, not a green
   timer. The separately retained Agent Runtime is out of scope and must
   merely reconnect; full-host loss remains a separate gate.

## Queue

Work top-down.

### P0 — Fix the bug

- Failing regression first: several Topics/Chats, failed claim, retried
  recovery — Room count, canonical Room id, and reachable Chat ids
  unchanged.
- Delete `/fresh` and its fixtures; no flag or fallback. Replace with three
  honest actions: retry load, retry claim, new Chat.
- Remove the parked public `/dashboard/device-link` page and its browser-facing
  approve/status APIs; keep device-link protocol support internal until a later
  explicitly authorized client run needs it.
- Persist the canonical binding in `finitechat-hosted-device`; validate it
  on reopen; remove `StartProfileChat`'s selected-Room preference.
- Load retained history before Runtime contact; test with the contact
  endpoint down and the model non-interactive.
- Migrate unbound legacy state per rule 3, with a preflight/postflight
  identifier-count comparison; project the sidebar across canonical +
  associated Rooms and fail the release on any retained-vs-visible mismatch.

### P0 — Real backups and one proven restore

- Service-owned consistent snapshot commands; Borg archives those artifacts
  (plus the pg_dump) to a real encrypted append-only off-host target;
  snapshot every 15 minutes, alert on failure or age > 30 minutes.
- Restore onto an empty target in isolated mode (public traffic and outbound
  side effects off), verify identifiers/history/attachment/claim/fresh turn,
  and reject corrupt, partial, or wrong-key snapshots without touching the
  target. Document the drill as a runbook and repeat it before paid
  admission.

### P1 — Consolidate and request acceptance

- Lifecycle sweep with the synthetic account: restart/deploy/upgrade each
  covered service and diff the identifier set before and after.
- ADR for the canonical binding + migration rule; update ADR 0011 and
  CONTEXT.md; add a read-only-first `Chats appear missing` runbook.
- Update the admission checklist: chat continuity/recovery failure blocks
  paid admission regardless of Stripe.
- Deploy the accepted revision under separate production-mutation authority,
  then produce the exact Acceptance Request defined in `README.md`: deployed
  revision, synthetic account and URL, lifecycle/identifier observations,
  selected Borg archive, empty-target location, stop conditions, and estimated
  minutes. Paul executes it and the acceptance statement at the top last.

## Production evidence — 2026-07-13

- Deployed revision: `3f26292`; Nix system closure:
  `/nix/store/mw5mspgziwkyw8bp2m75zrrjqa0sa5wp-nixos-system-finite-lat-1-25.11.20260630.b6018f8`.
- Dashboard image:
  `ghcr.io/finitecomputer/finite-saas-dashboard@sha256:1880f742aa93cac12bc1dcd65c5aebab685359b0d9b1fce74209b83ec51bfa60`.
- The finitecomputer rsync.net credential bundle was copied byte-for-byte to
  finite-lat-1 without entering Git. Snapshot and offsite age checks pass;
  application services and public health endpoints are healthy.
- Selected archive:
  `finite-lat-1-hosted-web-chat-2026-07-13T15:00:05` in the dedicated
  `finitecomputer/finite-lat-1` repository. Its pre-create manifest check and
  create completed successfully, and a subsequent remote listing found it.
- Append-only is **not proven**: the reused SSH credential accepted an
  arbitrary remote command. The empty-target service restore and Paul's
  browser lifecycle checks also remain. Therefore this run is not yet
  Ready-for-Paul and paid admission remains blocked.

## Acceptance Request — blocked on retained queue prerequisites

- **Revision:** `3f26292`; dashboard digest and Nix closure above.
- **Where:** `https://finite.computer`, `https://chat.finite.computer`,
  finite-lat-1, the dedicated synthetic account, and an empty isolated restore
  target. Secrets remain only at the paths named in the recovery runbook.
- **Time:** estimate 20 minutes for Paul's final browser lifecycle checks after
  the automated empty-target drill and append-only restriction pass.
- **Steps and observations:** reload every retained Chat; restart/deploy each
  covered service; exercise failed and retried owner claim; verify the same
  canonical Room and retained Chat set after each action; then inspect the
  restored isolated account, attachment, replayed claim, and fresh Agent turn.
- **Pass:** the Acceptance statement at the top of this run, using encrypted
  identifier evidence from the synthetic account and the selected archive.
- **Fail/stop:** any identifier-set change, unreachable retained Chat,
  unreadable attachment, claim divergence, restore mutation before complete
  verification, or an archival credential able to rewrite history. Capture
  count-only/read-only evidence and stop; do not switch restored traffic or
  admit paid users.

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
