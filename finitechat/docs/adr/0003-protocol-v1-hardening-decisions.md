# ADR 0003: Protocol V1 Hardening Decisions

Status: accepted 2026-06-11

## Context

The Marmot/Pika feature audit (`docs/feature-audit-marmot-pika.md`) identified
seven protocol-shaping gaps to close before external clients exist. The full
proposals, rejected alternatives, and chosen options live in
`docs/protocol-decisions.md`; this ADR records the accepted decisions and
their rationale in one place.

A standing constraint accepted alongside decision 4: **full-history replay is
a rare recovery action, not a day-to-day mechanism.** Any design that makes
ordinary operation depend on replaying large logs (server startup currently
replays the whole op log) is debt against this ADR.

## Decisions

1. **Protocol versioning, per-room.** Room metadata gains `protocol_version`
   and per-room `required_capabilities` (serde-defaulted, initially `1` and
   empty). Clients skip-and-advance unknown application kinds; they fail
   closed on unparseable commit-kind entries. The server gains a
   `min_protocol_version` lever. Rationale: vertically integrated, but client
   form factors (CLI, Electron, native iOS) will diverge in supported
   features, and update rollout needs a version to coordinate on. Copy
   Marmot's shapes where they fit; do not import their registry machinery.
2. **Relay authority boundary.** The server validates relay invariants for
   typed commits (room, epoch, active sender, caps, duplicate adds, and
   structurally valid membership deltas) but does not enforce social authority
   over encrypted rooms. Any active room member may create invite sessions and
   submit structurally valid membership commits, including cross-account adds.
   Admin metadata may exist only as advisory client/product state; it must not
   gate `/invites` or `/commits`.
3. **Leave-group, whole-account.** A typed leave closes all of the account's
   device intervals at the accepted seq (server-recognized immediately); the
   MLS remove commit follows asynchronously from an admin device. Per-device
   removal is the separate unlink flow.
4. **Retention with server deletion.** Seqs are never reused; compaction
   installs a per-room `horizon_seq`; sync below the horizon returns a
   horizon marker, not entries; expired/compacted entries are deleted on the
   server, not merely hidden.
5. **Push wake contract, minimal.** Wake payload is exactly `{room_id, seq}`.
   `/push-tokens` is wrapper-owned device state. Badges are client/NSE
   computed — the server takes on no per-device read-state beyond the
   existing delivery-effect counts.
6. **Agent stream lane, reserved not built.** `StreamStartV1` /
   `StreamFinishV1` typed kinds with a transcript-hash field; transient
   deltas never enter the ordered log; streams pin to the epoch at start.
7. **Recovery and rotation blessed.** Account recovery = restore account
   secret + relink via the existing link/fanout path. Credential renewal =
   self-update commit; 90-day credentials with renewal nudges from 30 days
   out.

## Consequences

Positive: membership authority, departure, and versioning semantics are fixed
before any external client bakes in the permissive v1 behavior; the Phase E
snapshot/horizon work has its cursor contract; push and streaming have frozen
wire shapes without server complexity.

Negative / accepted costs: typed `/commits` validation grows an authority
dimension (more rejection cases to test); leave introduces a
departed-but-not-yet-removed membership state that workers must reconcile;
server deletion makes retention irreversible by design.

Implementation order: decisions 1–3 first (they change typed-route
validation), then 4 with the Phase E work, 5–6 as wire reservations, 7 as
documentation.
