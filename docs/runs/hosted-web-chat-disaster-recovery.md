# Hosted Web Chat Disaster Recovery

Status: PROPOSED

Owner: Paul

Opened: 2026-07-13

Expires: 2026-08-28

Acceptance: One real encrypted Borg archive of the Hosted Web Chat Recovery
Set restores onto an empty isolated target. The restored Hosted Web Device,
Finite Chat server, and SaaS Core preserve the synthetic account's Account,
Device, Room, Topic, Chat, message, attachment, Project, Runtime, and Agent
identifiers; every retained conversation and encrypted attachment is readable;
the separately retained Agent Runtime reconnects, its owner claim replays, and
one fresh Agent turn completes. Wrong-key, corrupt, partial, unsupported, and
non-empty-target attempts fail before mutating the target. Paul performs the
final restored-browser checks; a green timer, successful archive, or
same-volume restart does not claim acceptance.

## Why this is separate

On 2026-07-13, Paul explicitly accepted the shipped Hosted Web Chat
product-continuity outcome, directed that run closed and deleted, and moved its
remaining disaster-recovery work into this no-authority proposal. The durable
binding and fail-closed bootstrap decisions live in ADR and CONTEXT documents.
Disaster recovery is a different outcome: proving that the coordinated service
state can be reconstructed after loss of the source host.

Production already creates service-consistent snapshots and encrypted off-host
Borg archives. The snapshot contains the Hosted Web Device identity, encrypted
client stores and Agent bindings, the complete Finite Chat server SQLite
database, and a SaaS Core Postgres dump. That machinery is not recovery proof
until one selected archive passes the positive and negative empty-target drill.

Current-state correction, 2026-07-20: snapshot creation is deploy/manual-only,
not every 15 minutes. The old stop/start timer broke live chat streams and was
removed. This run must prove a non-disruptive cadence at the accepted RPO; a
daily re-archive of a snapshot that may be seven days old is not sufficient.

This is a coordinated service-set restore, not a selective restore of one
Project or Agent machine. The Agent Runtime is retained separately, fenced
during the drill, and reconnected only after the restored service stack is
verified.

## Authority and boundaries

PROPOSED status grants no work authority. If Paul marks this run ACTIVE, it
authorizes repository code, tests, documentation, and disposable local restore
harnesses required by this queue. Provisioning an external target, accessing
off-host recovery credentials, changing production, switching traffic,
retention or compaction, and admitting customers remain separately authorized
mutations.

The current rsync.net credential is broader than destination-enforced
append-only access. Paul accepted that as non-blocking hardening debt on
2026-07-13; restricting it remains in `parking-lot.md` and is not an acceptance
prerequisite for this run.

## Queue

Work top-down only after this run is explicitly made ACTIVE.

### P0 — Fence and verify the recovery boundary

- Prepare the dedicated synthetic account with multiple Topics and Chats in
  every retained associated Room plus one encrypted attachment. Keep exact
  identifiers only in encrypted evidence outside this public repository.
- Select and record one real Borg archive, verify independent passphrase and
  key custody, and extract it outside the target. Extraction or verification
  failure must leave the target untouched.
- Provision an empty isolated target with public ingress and outbound email,
  webhook, push, billing, and other side effects disabled. Fence the retained
  Agent Runtime so it cannot contact both source and restored stacks.

### P0 — Prove positive and negative restore

- Run the existing verifier and atomic artifact restore, install the three
  service-state components with target-owned permissions, restore SaaS Core in
  one transaction, and start the stack only in isolated mode.
- Compare the encrypted before/after identifier set, open every retained
  conversation, decrypt history, and download the attachment before reconnecting
  the Runtime.
- Reconnect only the fenced retained Runtime, verify owner-claim replay through
  the canonical Room, and complete one fresh Agent turn without changing any
  retained identifier.
- Prove wrong key, truncated archive, modified artifact, missing database,
  unsupported format, and non-empty target all fail before target mutation.

### P1 — Make the proof repeatable

- Record archive, component revisions, target, elapsed recovery time,
  count-only comparison results, and pass/fail without plaintext or live
  identifiers. Update the recovery runbook for every discovered prerequisite.
- Repeat the positive and negative drill after snapshot-format or schema
  changes and before new lat3 customer admission. Add and keep the proven
  non-disruptive 15-minute recovery point and off-host age alerts green;
  neither substitutes for the drill.
- Produce the exact Acceptance Request from `README.md`: selected archive,
  isolated target, synthetic account URL, expected observation after each
  restore/browser step, stop conditions, and estimated minutes. Paul performs
  it last.

## Out of scope

- Hosted Web Chat UI, Room-binding changes, migrations, reconciliation, or
  repair controls.
- Selective per-Project or per-Runtime row restore, Agent Runtime volume
  backup, and full-host or provider disaster recovery.
- Production traffic cutover, retention/pruning/compaction, append-only
  credential hardening, Stripe, or customer admission.

## Governing documents

- [`docs/monorepo-doctrine.md`](../monorepo-doctrine.md)
- [`docs/adr/0001-recoverability-precedes-operator-blindness.md`](../adr/0001-recoverability-precedes-operator-blindness.md)
- [`finitechat/docs/adr/0011-hosted-web-chat-uses-a-revocable-device.md`](../../finitechat/docs/adr/0011-hosted-web-chat-uses-a-revocable-device.md)
- [`infra/README.md`](../../infra/README.md)
- [`infra/runbooks/hosted-web-chat-recovery.md`](../../infra/runbooks/hosted-web-chat-recovery.md)
