# Protocol Decision Queue

Date: 2026-06-11. Status: **all seven ACCEPTED 2026-06-11** — recorded in
`docs/adr/0003-protocol-v1-hardening-decisions.md`. Chosen options inline
below.

Source analysis: `docs/feature-audit-marmot-pika.md` §1 (what Marmot and Pika
thought of that we hadn't) and §5 (ordering rationale). This document turns
those findings into concrete, individually reviewable proposals. Accepted
decisions should graduate to ADRs (`docs/adr/`) per the grill-with-docs
convention; each section below is written to be liftable into one.

Decisions 1–3 change typed-route validation semantics and should land before
any external client exists. Decisions 4–6 are wire-format reservations whose
implementations can wait. Decision 7 is a documentation blessing of existing
machinery.

---

## 1. Protocol versioning & capability slots

**Context.** Everything is implicitly v1: typed `RoomLogEntry` kinds,
`MembershipDeltaV1`, route DTOs. The first post-ship addition of an event kind
or payload change has no answer for "what does an old client do with a kind it
doesn't understand," and the server has no lever to force an upgrade. Marmot
solves this with a capability registry; Pika learned the operational half the
hard way (`min_version` / `update_required` after a breaking format change).

**Proposal.**
- Typed room metadata (set at bootstrap, stored in the room-membership
  projection) gains `protocol_version: u32` (initially `1`) and
  `required_capabilities: Vec<String>` (initially empty). Serde defaults make
  both backward-compatible with existing rows.
- Client unknown-kind rule, fixed now: **application-kind** entries with an
  unrecognized payload type are skipped-and-advanced (cursor moves; the UI may
  show an "unsupported message" placeholder); **commit-kind** entries or
  membership projections the client cannot parse **fail closed** (the room
  stops locally, equivalent to needs-repair) because misreading membership is
  not survivable.
- Server-side: a config-level `min_protocol_version`; typed routes reject
  requests from clients that announce less (clients send their version in a
  header or request field — wire slot reserved now, enforcement later).

**Rejected alternative.** Marmot's full registry machinery
(extension/proposal/component capability negotiation): more than we need; we
control both ends until external clients exist, and the slots above are what
make later negotiation possible.

**DECIDED: per-room.** Vertically integrated service, but clients take
different forms (CLI, Electron, native iOS) and may support different
features; rollout coordination is the real driver. Don't overengineer; copy
Marmot's shapes where they serve us.

## 2. Admin authority for group rooms

**Context.** Typed `/commits` validates structure (epochs, caps, duplicates,
direct-room pairs) but not authority: today any active member can add or
remove any device in a group room. Marmot gates membership/profile/policy
changes on an admin pubkey list (one entry per account, across all device
leaves); Pika shipped a binary per-member admin flag.

**Proposal.**
- The room-membership projection gains `admins: BTreeSet<AccountId>`,
  initialized to the creator's account at typed bootstrap.
- Typed `/commits` authority rule: adds/removes touching **another account's
  devices** require `sender.account_id ∈ admins`. Same-account operations stay
  open to any active member: linking your own device (the fanout path) and
  removing your own devices never require admin.
- Direct rooms: both accounts are implicitly admins; existing third-account
  rejection already covers the rest.
- Admin changes ride a new typed commit-adjacent event `AdminChangeV1`
  (grant/revoke an account), sendable only by admins; revoking the last admin
  is rejected.
- Self-update commits (key rotation) require no authority.

**Rejected alternative.** Per-device admin (Pika's shape): conflicts with our
account/device model — authority is an account property; devices come and go
via linking.

**DECIDED: admins yes; no anyone-may-invite toggle for v1.** Admins may
revoke other admins, never the last one.

## 3. Leaving a group

**Context.** Plain MLS cannot self-remove with a commit; someone else must
commit your removal (Marmot carries MIP-03 SelfRemove proposals and chose its
wire-format policy around this). We have remove-device commits and removal
intervals, but no flow a *departing* user can drive. Today a user cannot
leave a group — visible on day one.

**Proposal.** Server-recognized leave, MLS cleanup async:
- New typed route `/rooms/leave`: an active device's leave closes **all of
  that account's device intervals** in the room at the accepted seq. From that
  seq the server stops delivering new entries to those devices (same
  filtering machinery as removal) and rejects their sends.
- The MLS removal commit follows asynchronously: remaining members' workers
  observe the departed-but-not-yet-removed state in the projection and any
  admin device submits the actual remove commit (restores forward secrecy
  against the departed devices' keys). Until that commit lands, departed
  devices hold keys but receive no ciphertext from our server.
- Departed devices can sync up to their leave seq, never past it.

**Rejected alternative.** Pure-MLS leave (wait for another member to commit
first): leaves the departing user hostage to other members' availability, and
"I left but I'm still in the group" is a product failure even if it's
cryptographically honest.

**DECIDED: whole-account leave.** Per-device removal is the unlink flow,
separate work.

## 4. Retention field + below-horizon sync semantics

**Context.** Disappearing messages (Marmot component 0x8005) is a product
feature we may want, but the protocol-shaping part is on our server: the
durable ordered log currently keeps everything forever, every client assumes
"sync from 0 replays everything," and the Phase E compaction/snapshot work
(perf-log items 2–4) needs cursor semantics decided before it can exist.

**Proposal.** Decide the cursor contract now, implement at Phase E:
- Room metadata gains `retention: Option<RetentionPolicyV1>` with
  `disappear_after_secs`; `None` (v1 always) means keep forever.
- Sequence numbers are **never reused**. Compaction installs a per-room
  `horizon_seq`; entries at or below it may be deleted.
- `/sync/group` with `after_seq < horizon_seq` returns an explicit horizon
  marker (and the horizon as the new cursor) instead of entries; clients
  render a "history unavailable before this point" boundary and continue
  forward. New devices already start at their add-commit seq, so the horizon
  only affects pre-existing devices syncing very old cursors.
- Client-side deletion of expired plaintext is client policy, not protocol.

**DECIDED: server-deleted.** Also a standing design directive recorded
here: full-history replay must be a rare recovery action, not a day-to-day
mechanism — this raises the priority of the Phase E snapshot/horizon work
(today the server replays the whole op log on every startup).

## 5. Push notification wake contract

**Context.** Our delivery-effect projection already tells the server exactly
who should be woken and how (push/unread/command-inbox) without decrypting
anything — stronger ground than either Marmot's token-gossip draft or Pika's
shipped notification server. What is missing is the device edge. Pika's
proven shape: tokens registered with the server; a Notification Service
Extension decrypts on-device and renders rich previews.

**Proposal.** Freeze the two wire shapes now, build the pusher later:
- Wake payload is exactly `{room_id, seq}` — nothing else. Enough for an NSE
  to pull `/sync/group` from its cursor and decrypt locally; leaks nothing
  but room-activity timing the push platform sees anyway.
- New wrapper-owned route family `/push-tokens` (register/replace/remove):
  `{device_ref, platform (apns|fcm), token, app_group}` — durable state with
  the same lifecycle character as link sessions. Revoked/removed devices'
  tokens are dropped by the existing device-lifecycle hooks.
- The pusher daemon consumes the delivery-effect projection (it already
  records per-message push counts) — no new protocol surface.

**DECIDED: client/NSE-computed badges; no server badge tracking.** Erring
on the side of protocol simplicity: the wake payload stays exactly
`{room_id, seq}` and the server takes on no additional per-device read-state
beyond the existing delivery-effect counts.

## 6. Agent stream lane reservation

**Context.** Closest feature to the product. Marmot's design (experimental
but implemented): durable start/final messages anchor a stream in the ordered
log; transient deltas flow off-log with a running transcript hash so the
final durable message is verifiable against what streamed; streams are pinned
to one MLS epoch; broker replay window for reconnects.

**Proposal.** Reserve, don't build:
- Two typed application kinds frozen now: `StreamStartV1 {stream_id,
  conversation_id}` and `StreamFinishV1 {stream_id, transcript_hash,
  final_payload_ref}`.
- Rule fixed now: transient deltas **never** enter the ordered log.
- Epoch pinning copied from Marmot verbatim: a stream is bound to the room
  epoch at `StreamStartV1`; a membership change mid-stream aborts the stream
  (the finish references what was actually streamed; the client re-issues
  under the new epoch if desired).
- Future transport: SSE off our HTTP server keyed by (room, conversation),
  replay TTL ≤ 300 s — our server-ordered architecture makes the broker
  trivial compared to their QUIC mesh; QUIC remains an option later.

**DECIDED: reserve, don't build.** Transcript-hash algorithm deferred to
implementation (opaque bytes on the wire).

## 7. Recovery and credential rotation (documentation blessing)

**Context.** Pika shipped with no recovery story and feels it. We already
have the machinery; it has just never been named as the recovery path.

**Proposal.** No new wire surface. Bless and document:
- **Account recovery** = restore the account secret (user-held backup; format
  out of scope here) + link a fresh device through the existing
  link-session/fanout path. A recovered account is just a new device link.
- **Credential renewal** = self-update commit before `not_after` (the client
  already has `prepare_self_update_commit`); the sync worker gains a nudge
  when expiry is near. Expired-credential devices are treated as
  pending-removal, recoverable by relink.
- Write as a short ADR once accepted.

**DECIDED: 90-day credentials, renewal nudges from 30 days out.**

---

## Review checklist

| # | Decision | Blocking for | Outcome |
| - | --- | --- | --- |
| 1 | Versioning slots | any external client | accepted; per-room capabilities |
| 2 | Admin authority | any external client | accepted; no invite toggle |
| 3 | Leave-group | first real users | accepted; whole-account |
| 4 | Retention/horizon | Phase E compaction | accepted; server-deleted |
| 5 | Push wake contract | mobile clients | accepted; client-computed badges |
| 6 | Stream lane | agent streaming | accepted; reservation only |
| 7 | Recovery blessing | docs only | accepted; 90-day credentials |
