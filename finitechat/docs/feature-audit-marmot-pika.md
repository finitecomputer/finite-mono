# Feature Audit: What Marmot/Darkmatter and Pika Thought Of That We Haven't

Date: 2026-06-10. Sources audited: `../darkmatter` (branch `http-delivery-upstream`,
including `spec/` MIP drafts and all crates) and
`/Users/futurepaul/dev/sec/other-peoples-code/pika` (shipped iOS/Android/desktop
chat with notification server and QUIC calls).

Purpose: we have not shipped to users yet. This is the catch-up pass on
everything two adjacent projects learned by shipping or speccing, sorted by
whether our protocol needs a *decision now* (so the wire format and server
contract can support it later) versus things we can defer outright.

## How to read the verdicts

- **DECIDE NOW** — shapes the wire contract, the ordered log, or the server's
  authority model. Cheap to leave room for, expensive to retrofit.
- **COVERED** — our protocol already supports it (sometimes better); noted for
  confidence, with the matching finite mechanism.
- **DEFER** — product/client work that our protocol already permits; no
  protocol change needed, just future implementation.

---

## 1. Gaps that need a protocol decision now

### 1.1 Capability negotiation / protocol versioning — biggest structural gap

Darkmatter has a full machinery for this: a runtime `FeatureRegistry` mapping
features to `Capability` requirements (`Extension(u16)` / `Proposal(u16)` /
`AppComponent(u16)`) with `Required | Optional | TransportRequired` levels, a
`capability_manager` that derives per-leaf capabilities for KeyPackages and
validates leaves against group requirements, and an app-component dictionary in
GroupContext with numbered component ids (0x8000 group profile … 0x8007 blossom
image). Pika independently learned the operational half: a `min_version`
enforcement flag and `update_required` app state, plus a documented
compatibility matrix when their MLS kit broke KeyPackage format between 0.5
and 0.6.

We have none of this. Our typed `RoomLogEntry` kinds, membership delta
versions (`MembershipDeltaV1`), and route DTOs are implicitly v1 everywhere.
The first time we add an event kind or change a payload after shipping, every
old client and our server need an answer for "what do I do with a kind I don't
understand."

**Decision needed:** (a) reserve a versioning/capability field in room
metadata (typed bootstrap is the natural home — the server already owns room
metadata), (b) define the unknown-kind rule for clients (skip-and-advance vs
fail-closed — skip-and-advance is right for application kinds, fail-closed for
commit-adjacent kinds), and (c) give the server a `min_client` / required-
capability check at typed routes so we can force upgrades. We do not need the
registry machinery; we need the *slots*.

### 1.2 Admin policy — who is allowed to change a group

Darkmatter's admin-policy component (0x8003) is a sorted list of account
pubkeys; admin-gated actions are profile/avatar changes, admin changes, member
invite/remove, routing, and retention changes — checked against the
committer's credential identity, one admin entry per account across all device
leaves. Pika shipped a binary `is_admin` flag per member.

Our typed `/commits` validates *structure* (epochs, duplicates, caps, direct
room pairs) but not *authority*: today any active member can add or remove any
device. For direct rooms that's nearly fine; for group rooms it is not a
shippable policy.

**Decision needed:** an authority rule in the room-membership projection. The
server-authoritative design makes this easy for us — the projection already
knows accounts; an `admins: BTreeSet<AccountId>` on the room plus a check in
typed `/commits` (and a typed event kind for admin changes) covers it. Decide
now because it changes `SubmitCommitRequest` validation semantics, which is the
contract we least want to churn after shipping.

### 1.3 Leaving a group (self-remove)

Marmot carries MIP-03 SelfRemove proposals — and chose its MLS wire-format
policy (PublicMessage both directions) specifically because SelfRemove
requires it. In plain MLS, a member cannot remove themselves with a commit;
someone else must commit the removal. We have remove-device commits and
removal-interval projections, but no "I leave" flow: a departing device can
only ask another member to remove it, or we accept a server-mediated leave.

**Decision needed:** pick the finite-shaped answer: a typed "leave request"
event that any active member's client (or the room creator's daemon) converts
into a remove commit, or a server-recognized leave that closes the membership
interval immediately while the MLS removal commits later. Either works with
our projection model; not having *any* answer means a user cannot leave a
group, which users notice immediately.

### 1.4 Live token streaming for agents (their QUIC agent-text-stream)

This is the feature closest to our product. Darkmatter's design
(`agent-stream-compose` + spec 0x8006, experimental but implemented): a
*durable* MLS start message (kind 1200) and final message (kind 9) anchor the
stream in the ordered log; *transient* records (TextDelta, ProgressDelta,
Status, Checkpoint, Abort, FinalNotice) flow over QUIC with per-record
ChaCha20-Poly1305, keys derived via MLS-Exporter bound to a stream context, a
running transcript hash so the durable final message can be verified against
what streamed, streams pinned to one MLS epoch, and a broker replay window
(`replay_ttl_secs ≤ 300`) for reconnects. They also keep typed durable agent
rows (activity 1201, operation 1202 — tool calls, approvals, handoffs).

Our nearest mechanisms: ephemeral `/activities` (volatile, opaque, capped,
unordered) and durable runtime-state snapshots. Neither gives ordered,
resumable, verifiable token streams.

**Decision needed (support, don't build):** reserve the shape — (a) a typed
durable "stream started/finished" application kind whose final payload carries
a transcript hash, (b) the rule that transient deltas never enter the ordered
log, and (c) the expectation that a future stream lane (SSE over our HTTP
server is the natural finite transport before QUIC) is keyed per
room+conversation and replayable within a TTL. Their epoch-pinning rule is
worth copying verbatim: it sidesteps every mid-stream-membership-change
question. Our `/activities` route-key scoping (room/conversation) is already
the right addressing model.

### 1.5 Push notifications — registration and wake path

Both projects have working models. Darkmatter (MIP-05 draft): push tokens
encrypted and *gossiped inside the group* so members can trigger
notifications without any party learning tokens-to-account mappings; decoy
tokens; gift-wrapped wake hints to a notification server. Pika (shipped):
device registers token + per-group subscriptions with a dedicated server; the
server watches the relay firehose and wakes devices; an iOS Notification
Service Extension decrypts *on device* and renders rich previews (sender name,
group name, image thumbnails with a 10 MB cap, call invites) — the server
never sees plaintext. Pika's hard-won operational details: badge counts,
self-message filtering, profile-name caches available to the NSE, Android
notification channels.

Our position is actually stronger than both — the server already knows, per
message, who should be pushed and how (`ApplicationDeliveryPolicy` → durable
push/unread/command-inbox counts) without decrypting anything, because the
sender declares the policy. What's missing is the *device edge*: a token
registration route, a wake sender, and the payload contract for the NSE.

**Decision needed:** define the wake payload now: `(room_id, seq)` plus
nothing else. That is enough for an NSE to pull `/sync/group` from its cursor
and decrypt locally, leaks nothing, and works with our requester-filtered
sync. Add `/push-tokens` as wrapper-owned state (it's exactly the shape of our
liveness/link-session wrappers). The notifier daemon itself is DEFER.

### 1.6 Disappearing messages / retention

Darkmatter: component 0x8005, a per-group `disappearing_message_secs`, expiry
computed from sender's created_at; deliberately *not* enforced at the protocol
layer (clients delete; the transport may prune). They also have a server-side
retained-history/anchor model for pruning old epochs.

We have nothing — and our durable ordered log makes deletion *harder* than
their relay model, because our server holds every entry forever and replays
the full op log at startup. Even ignoring the product feature, we need a
**log compaction / pruning story** for the server (also a perf finding — see
perf audit §3). Retention policy and compaction should be designed together:
a room-level retention field (even if v1 says "forever") plus a server rule
for what compaction does to sequence numbers (answer: seqs are never reused;
compaction replaces pruned entries with a tombstone/horizon marker, and
`/sync/group` from a cursor below the horizon returns the horizon).

**Decision needed:** reserve the room-level retention field and define the
sync-below-horizon behavior. Implementation can wait; the cursor semantics
cannot, because every client already depends on "sync from 0 replays
everything."

### 1.7 Account backup / recovery / key rotation

Pika ships account creation handing the `nsec` to the platform keychain, plus
NIP-46 external signers and bunker login — but documented no recovery flow,
and that's a known hole they feel. Darkmatter has KeyPackage *upgrade* flow
(refresh credentials/signature keys) and credential time-bounds.

We have device credentials with `not_before/not_after` (so expiry exists) but
no story for: account secret backup/import, what happens when a credential
expires mid-membership, or rotating a device's MLS signature key without
remove+re-add.

**Decision needed:** minimally, decide that *credential renewal* is a typed
self-update commit (our client already has `prepare_self_update_commit` — the
reprepare test uses it) and that account recovery = restore secret + link a
fresh device via the existing link-session/fanout path (which is genuinely our
best machinery — recovery is just "new device linking" if the secret is
backed up). Then the only new protocol surface is *none*; the decision is to
bless that path and write it down.

---

## 2. Covered — confirmation we already thought of it

| Their feature | Our mechanism | Notes |
| --- | --- | --- |
| Marmot account identity proof (Schnorr-bound leaf extension 0xF2F1) | `FiniteDeviceCredentialV1` (Nostr-signed device credential verified at KeyPackage parse, commit merge, welcome) | Equivalent binding; ours adds device_id and time bounds. |
| Marmot multi-device draft (MIP-06: external commits, join PSK, pairing payloads, encrypted device names) | Link sessions + account-room directory + fanout FSM + Welcome activation | We are *ahead* here — theirs is a draft; ours is implemented and crash-tested. Watch MIP-06 only for wire-compat opportunities. |
| Marmot fork recovery, convergence policy, witness quorum, snapshots, retained anchors | Server-ordered log + same-epoch admission + pending-commit merge rule | Their hardest subsystem exists because they lack an ordering authority. We deleted this problem class by design. Their *invalidation status* on timeline rows (fork losers) has no finite analog because forks can't happen. |
| Marmot encrypted media (per-epoch secrets, HKDF per-file keys, Blossom locators, imeta) | `finitechat-blob`: encrypted Blossom-compatible refs, ciphertext verification, size limits, metadata hiding | Comparable maturity. Two details of theirs worth copying into the blob plan: `thumbhash` + `dim` fields in attachment metadata (pika's NSE renders preview thumbnails from these), and an explicit per-epoch media-secret rotation rule if we ever derive attachment keys from group state rather than per-blob keys. |
| Marmot Hermes integration (socket protocol, streaming config, interim messages) | `finitechat-hermes` + Hermes adapter tests + finitecomputer bridge | Both exist; theirs adds streaming/tool-progress toggles — relevant to §1.4, not to the bridge itself. |
| Reactions / edits / deletes / receipts / read state | Typed application kinds with non-notifying `ApplicationDeliveryPolicy`, proven over `/application-events` | Their timeline projection (reaction summaries, reply preview cache, deleted_by tracking) is client-side product work we'll need, but the protocol carries it already. |
| Pika offline outbox / publish retry (3 attempts, backoff) | Idempotency keys on every publish with exact replay after restart | Ours is strictly stronger (server-verified exact replay vs at-least-once). |
| Pika/Marmot forensic & audit logging | Op-log is already an audit trail; Marmot's `ForensicRecorder` (JSONL of fork events) is convergence-specific | Nothing to do. |
| Marmot group profile/avatar components (0x8000/0x8002/0x8007) | Room metadata in typed bootstrap + product DTOs | Verify our room metadata struct has name/avatar-ref slots; if not, that's a 30-minute DTO addition best done before v1 freeze. |
| Pika relay multi-set config (message vs keypackage vs welcome relays) | Single HTTP server owns all three planes | Single-server model collapses this; if we ever shard, planes are already separate routes. |

---

## 3. Defer — protocol already permits, build later

- **Voice/video calls** (pika: shipped; QUIC/moq, Opus 48 kHz, frame-level
  AES-GCM with sequence nonces, jitter buffer, replay windows, call-invite
  signaling as custom app kind 10, proximity/mute state machines). Call
  *signaling* fits our typed application kinds today (invite/accept/reject as
  durable events with push policy; ringing state as ephemeral activity). Media
  transport is a separate system when we want it. No protocol reservation
  needed beyond an app kind.
- **NSE/share-extension engineering** (pika: decrypt-in-extension UniFFI
  bridge, shared SQLite profile cache for sender names, share-queue with TTL
  and backoff). Pure client architecture; our encrypted client store + pull
  sync supports it. Note for the future: the NSE needs read access to the
  client store — our single-file encrypted SQLite snapshot is compatible with
  an app-group container.
- **Markdown/rich rendering, mentions, link previews** (marmot-markdown).
  Product layer.
- **Zapstore/TestFlight-style release plumbing, CI secrets via age, staged
  release gates** (pika). Operational; their `VERSION`-file → versionCode
  scheme is worth copying when we ship mobile.
- **Rate limiting / abuse controls** — neither project has real answers
  (pika: relay-side only; marmot: relay policy). Our server-authoritative
  design is *better positioned* than both: every send is an authenticated
  HTTP request we can rate-limit per device. Defer the limits, but note this
  as a genuine advantage of the architecture worth preserving.
- **Decoy push tokens / metadata-hiding push** (marmot MIP-05). Our push model
  intentionally trusts our own server with push *routing* (never content). If
  we later want metadata-hiding push, MIP-05 is the reference design; nothing
  in our contract blocks it.

## 4. Where we are ahead (for the record)

- Durable, crash-atomic, restart-replayed server state (their HTTP server is
  in-memory; their durable story is client-side only).
- Multi-device linking end-to-end (their MIP-06 is a draft).
- Server-side delivery-effect projection (push/unread/command-inbox counts
  without decryption) — neither project has an equivalent.
- Idempotent exact-replay semantics on every mutating route.
- KeyPackage lease lifecycle with expiry/reclaim (upstream consume-once only).
- The conformance suite itself.

## 5. Recommended order of protocol decisions

1. Versioning/capability slots in room metadata + unknown-kind rule (§1.1)
2. Admin authority in membership projection + typed commits (§1.2)
3. Leave-group flow (§1.3)
4. Retention field + sync-below-horizon cursor semantics, designed with server
   log compaction (§1.6, pairs with perf audit §3)
5. Push wake payload contract `(room_id, seq)` + token-registration route
   shape (§1.5)
6. Stream-lane reservation: durable start/final kinds + transcript-hash field
   + epoch pinning rule (§1.4)
7. Bless credential-renewal-via-self-update and recovery-via-relink; write the
   doc (§1.7)

Items 1–3 change validation semantics on the typed routes and should land
before any external client exists. Items 4–7 are wire-format reservations and
docs.
