# Hosted Web Chat Continuity and Recovery Readiness

Status: PROPOSED

Owner: Paul

Opened: 2026-07-13

Expires: 2026-08-28

Acceptance: A dedicated synthetic account creates one Project and Agent, then
creates multiple Topics, Chats, turns, and encrypted attachments through the
Hosted Web Device. Browser reload, dashboard deployment, Hosted Web Device and
Finite Chat server restart, owner-claim failure and retry, Agent Runtime
restart, Runtime Upgrade, recover-known-good, and canonical/legacy Runtime URL
navigation never change the canonical Agent Room or make any retained
conversation unreachable. A coordinated, encrypted off-host Recovery Snapshot
of the declared Hosted Web Chat Recovery Set restores the Hosted Web Device,
Finite Chat server, and isolated SaaS Core services onto an empty replacement
chat/control-plane target. It preserves the covered Account and Device
identities, Room, Topic, Chat, message, attachment, binding, and selection
identifiers. After an authorized controlled traffic switch, the same
separately retained and lifecycle-fenced Standard/Kata Agent Runtime reuses the
exact durable successful owner claim and completes a fresh turn in the restored
Chat. This proves Hosted Web Chat Recovery Readiness, not Agent Runtime
disaster recovery. Paul performs the final browser and empty-target
restored-data checks.
Automated tests, a green backup timer, a copied live SQLite file, or a
provider-volume restart do not claim acceptance.

## Problem statement

A read-only production audit on 2026-07-13 found intact Hosted Web Device
history hidden behind a second exact-member Agent Room created through
`StartGroupChat`. Finite Chat Core's `StartProfileChat` existing-Room lookup
preferred the currently selected matching Room, bootstrap reopened that
selection, and the sidebar projected Topics only from that Room. A user could
therefore see an apparently empty product even though the older Room, Topic,
Chat, and message records remained in an integrity-clean client store under
the same human and Agent Principals. The audit verified retained projections,
metadata, and counts; it did not individually decrypt every old message body
or render every attachment.

The stable Runtime URL change did not migrate the Project, Runtime, Agent, or
Hosted Web Device store. The failure was a product-identity and projection
bug: a navigation cursor was allowed to become an implicit relationship, and
the UI treated one protocol Room as the complete product conversation set.
That class of bug is unacceptable for a paying user because "retained" data
that the normal product cannot reach is unavailable data.

The same repository and read-only host audit found a second, independent risk.
The declared Borg destination remains a placeholder, the last observed
scheduled job was unsuccessful, and no configured successful,
service-consistent, restore-proven off-host copy was found. Copying live SQLite
directories is not a recovery contract. The Hosted Web Device identity and
encrypted client state must be restored together; room-server ciphertext alone
cannot recover a usable Device or retained history.

This run establishes one bounded outcome: Hosted Web Chat keeps one durable
Agent conversation identity through ordinary lifecycle events, keeps every
legacy retained conversation reachable, and proves the chat portion of the
SaaS Recovery Set on an empty chat/control-plane target before paid customer
admission.

## Authority and boundaries

PROPOSED status grants no work authority. Paul must change this run to ACTIVE
before implementation begins. Do not activate it while [Stripe Checkout
Readiness](stripe-checkout-readiness.md) remains ACTIVE; the repository has one
top-down active queue at a time.

If activated, this run authorizes repository code, tests, ADRs, runbooks, Nix
definitions, and disposable local test state required by the queue. It does
not authorize production deployment, production data migration or selection
repair, restarts, live backup-target or credential changes, destructive fault
injection, customer admission, or use of a real person's account as a
mutating canary. Each is an external mutation requiring separate explicit
authorization.

Passing this run satisfies only the Hosted Web Chat continuity/recovery gate.
Paid customer admission remains governed by the retained admission checklist,
including untrusted owner-claim, stuck-launch escape/reaper, Agent Runtime
Recovery Readiness, every user-data product actually admitted, and every other
unresolved gate named there. Stripe success cannot override any of them. This
is deliberately stricter than the trusted-first-slice allowance in ADR 0001
and does not revoke separately authorized internal or synthetic canaries. The
proposed Phala run inherits this chat gate; a Phala volume or TEE does not
replace it.

This run preserves the product boundaries already accepted:

- Finite Chat owns Accounts, Devices, Rooms, Topics, Chats, encrypted events,
  and delivery. The Finite Computer dashboard composes that product through a
  Hosted Web Device.
- SaaS Core supplies authenticated Account/Project/Runtime ownership and
  access facts. The Hosted Web adapter initially observes the Agent Principal
  through the Project-authorized Runtime contact path, persists that exact
  Principal in its binding, and treats later contact as verification rather
  than a prerequisite for loading history. SaaS Core does not own Room ids,
  select Rooms, or become a plaintext transcript store.
- This run exercises the accepted Standard/Kata Runtime through existing typed
  SaaS Core-to-Runner lifecycle controls as a black-box composition test. Runner
  and Runtime lifecycle code never select, create, merge, delete, snapshot, or
  fence chat-service Rooms, Topics, Chats, or files. No provider shell, remote
  `.env`, filesystem browser, or feature-specific Runtime Management Pipe
  command is added, and no Phala resource is provisioned.
- Retained history loads without a responsive or interactive Agent. The
  existing typed `agent.owner.claim` remains required before Chat send or
  Connections becomes usable; Hermes/model availability is required only for
  a new Agent reply.
- Electron remains parked. This run may not change Electron code, packaging,
  UI, or enrollment.
- The first paid-cohort posture may remain the honestly described O1
  Finite-assisted recovery posture. This run does not choose the final User
  Recovery Key or claim cryptographic operator-blindness.
- No secret value, private message body, user email, or recovery material may
  enter code, tests, public docs, fixtures, or public evidence. Exact live
  identifiers may exist only inside encrypted, access-controlled migration and
  snapshot state where restore requires them. Operational logs and metrics use
  counts and purpose-scoped keyed pseudonyms, never raw live ids or plain
  stable hashes.

## Settled product and recovery decisions

1. **One canonical Agent Room.** For the current one-human/one-Agent Project
   shape, the stable Project, human Account Principal, and exact Agent
   Principal bind to one canonical exact-member Agent Room. Future
   adapter-created Rooms persist a creation purpose so explicit group Rooms
   remain separate. Legacy state has no reliable direct-versus-group creation
   marker, so migration uses observable membership facts and never invents
   provenance.
2. **The binding is durable product state.** The `finitechat-hosted-device`
   Rust service owns an explicit, encrypted, versioned binding and ensure
   intent in the per-user Hosted Web Device store. SaaS Core supplies
   authenticated Project/Runtime ownership; the first Project-authorized
   Runtime contact observation supplies the exact Agent Principal, which the
   binding makes authoritative for later offline history load. Runtime ids,
   provider handles, host names, URL aliases, display names, and
   `selected_room_id` are not binding keys.
3. **Selection is only a cursor.** Remembered Room, Topic, and Chat selection
   determines what the user sees next. It cannot create, replace, infer, or
   repair the canonical Agent Room.
4. **New Chat is not New Room.** The normal `New chat` action starts a Chat in
   a Topic inside the canonical Agent Room. It never invokes
   `StartGroupChat`, changes membership, or creates another Home Topic as a
   recovery side effect.
5. **Bootstrap never guesses destructively.** It opens and validates the
   recorded binding without contacting the Runtime first. Initial onboarding
   uses one durable, idempotent, crash-reconcilable ensure state machine; it
   does not pretend Room creation and local binding are one distributed ACID
   transaction. Restart, retry, URL navigation, or owner-claim failure may not
   create a replacement Room or identity. Missing, partial, corrupt, or
   ambiguous state fails closed as recovery-required.
6. **All retained conversations remain reachable.** Existing duplicate
   exact-member Agent Rooms are not deleted, left, merged, or re-encrypted.
   They become associated historical Agent Rooms, and their Topics and Chats
   appear through a product-level `Previous conversations` projection without
   exposing transport-room jargon in normal UI.
7. **Legacy canonical choice is deterministic.** A valid existing binding
   wins. With no binding, migration considers connected Rooms whose observable
   current membership is exactly the human and Agent Principals, selects the
   Room with the oldest durable local event timestamp, and uses stable Room-id
   order when no comparable timestamp exists or values tie. It records the
   inputs and reason. The current selection alone never wins. Every other
   eligible Room remains associated and reachable; migration does not claim to
   recover a legacy Room's missing creation purpose.
8. **The visible set may not shrink silently.** For each Agent binding, the
   product-visible Topic, Chat, message, and attachment identifiers must be a
   superset across restart, deployment, migration, and recovery. An empty UI
   is valid only when no retained conversation exists.
9. **Lifecycle cannot rotate conversation identity.** Dashboard deployment,
   Hosted Web Device restart, Finite Chat server restart, owner-claim retry,
   Runtime restart, Runtime Upgrade, recover-known-good, and Runtime URL
   canonicalization preserve the human Principal, Agent Principal, Device,
   binding, and canonical Room.
10. **Hard cut the bad recovery path.** Remove the `/fresh` product action,
    UI, helper, and tests that encode a fresh Room as recovery. Do not retain a
    flag, alias, compatibility fallback, or canary-only parallel path.
11. **Recovery restores identity and ciphertext together.** The Hosted Web
    Device identity, its encrypted client state, the whole-service Finite Chat
    checkpoint, the complete isolated SaaS Core checkpoint, and the
    configuration/authorities needed to boot them form one versioned chat
    recovery composition. None is useful proof alone.
12. **Recovery readiness means an empty chat/control-plane target.** A
    snapshot, timer, archive, provider volume, integrity check, or booting
    service is not acceptance. The same identifiers, readable history and
    attachments, exact owner claim, and next Agent turn must work after the
    covered services move to an empty replacement target while the separately
    retained Standard/Kata Agent Runtime remains lifecycle-fenced and
    unchanged. Total
    Agent Runtime loss remains its own Recovery Readiness gate.
13. **Canaries are isolated.** Destructive, migration, and restore tests use a
    dedicated synthetic account and synthetic content. A teammate or customer
    account may be inspected read-only only when explicitly authorized; it is
    never rewritten to prove a release.
14. **Owner claim stays Room-scoped and canonical.** Claim and durable replay
    always travel through the canonical bound Room even when the user is
    reading or continuing an associated historical Chat. Historical selection
    cannot transfer claim authority, choose a different claim result, or
    create another Room. If the canonical Room has no successful claim, it
    remains mutation-gated until a fresh typed claim succeeds there.
15. **Idempotency follows user intent.** One client-generated New Chat intent
    has one durable result that retries replay. A later independent user intent
    legitimately creates a different Chat; broad deduplication across tabs or
    time must not collapse distinct user choices.

## Recoverability contract for this run

The run covers the following failures and required outcomes. Failures outside
this table do not silently become claimed recovery coverage.

| Covered failure | Required outcome |
| --- | --- |
| Browser reload, window close/reopen, direct URL, or canonical/legacy Runtime URL | Reopen the last reachable Chat for that Agent without changing the binding or visible conversation set. |
| Dashboard deployment or rollback-compatible static asset replacement | Preserve stored product state; no bootstrap Room creation or selection reset. |
| Hosted Web Device graceful restart, kill/restart, or Nix service restart | Reopen the exact identity, encrypted client database, binding, outbox, and selection. |
| Finite Chat room-server restart or brief outage | Preserve room order, membership, idempotency, encrypted events and blobs; replay without duplicate user-visible messages. |
| Owner-claim timeout, Agent unavailability, or Hermes/model non-interactivity | Show retained history immediately; keep sending and Connections honestly unavailable until the exact claim succeeds or replays. |
| Agent Runtime restart, Runtime Upgrade, or recover-known-good on mounted state | Preserve Agent Principal, canonical Room, owner-claim authority, workspace relationship, and same-conversation continuation. |
| Concurrent tabs, repeated bootstrap, or retry of one `New chat` intent | Replay one durable result for the same intent key, preserve separate user intents, and never create an extra Agent Room. |
| Legacy duplicate exact-member Agent Rooms | Select one canonical binding deterministically and expose every associated retained Chat without deleting ciphertext or history. |
| Crash during binding or duplicate migration | Reconcile the durable phase to the same result or retry safely; never leave history hidden behind half-written state. |
| Hosted client SQLite/WAL crash, partial identity/store presence, or corrupt snapshot | Recover from a valid snapshot or fail closed with a typed recovery-required state; never mint an unrelated identity or Room. |
| Destructive loss or corruption of the covered Hosted Web Device, Finite Chat, and SaaS Core service roots while the separately retained Standard/Kata Runtime `/data` remains available | Restore the declared chat/control-plane Recovery Set onto an empty isolated replacement target from an independent off-host snapshot. |
| Snapshot taken while chat writes and attachment uploads are active | Restore a transactionally valid cut and reconcile safe idempotent replay without missing or duplicating acknowledged content covered by that cut. |

### Declared Hosted Web Chat Recovery Set

| Component | Data that must be covered | Restore invariant |
| --- | --- | --- |
| Hosted Web Device identity | The exact Account and Device key material, device id, enrollment/revocation facts, and protected bootstrap material required to reopen that identity. | Restored service presents the same Account Principal and Device; it never creates replacements. |
| Hosted Web Device client state | Encrypted client SQLite state including MLS groups/epochs and Device group secrets, application event log and projection, outbox/idempotency state, canonical/associated Room binding, per-Agent selection, durable attachment references, and decryption metadata. Include staged local bytes only when they are the sole acknowledged durable copy, and encrypt them before snapshot. | Integrity passes and the identifier set covered by the snapshot watermark is reachable. Reconstructible plaintext cache is absent. |
| Finite Chat room server | One whole-service SQLite checkpoint containing ordered delivery/admission state, membership projections, opaque Welcome envelopes/claims, encrypted application events, idempotency records, and encrypted blob-object rows. MLS group/ratchet secrets remain Device state. | Exact covered Room/event/blob identifiers and ordering survive; acknowledged sends at the cut are present once. |
| SaaS Core relationship state | One complete PostgreSQL checkpoint containing WorkOS-subject, Project, Runtime, ownership, lifecycle, and canonical URL-alias facts. The exact Agent Principal remains in the Hosted Web binding rather than making SaaS Core Room authority. | In an isolated full restore, the signed-in owner resolves the same Project and Runtime; lifecycle navigation does not become chat identity. |
| Configuration, artifacts, and authorities | Exact Nix closure and image digests plus independently available artifacts; Caddy/DNS/TLS shape; WorkOS application and redirect configuration names; route-scoped service credential names; snapshot repository/key-custody locations; and required Recovery Authority records. No secret values enter the snapshot manifest or repository. | An empty target can boot the exact covered revision in isolated restore mode with every external side effect disabled until authorization and validation. |
| Snapshot manifest | Format/schema and product revisions, component watermarks, stable identifiers, byte sizes, cryptographic hashes, creation time, covered failure set, encryption/key reference names, and authority record references. Exact live identifiers remain inside this encrypted access-controlled artifact. | Missing, incomplete, incompatible, corrupt, or wrong-authority material rejects before target mutation. Staleness blocks promotion but remains eligible for explicit isolated Break-Glass evaluation. |

Hosted Web bundles are per WorkOS subject. Finite Chat server and SaaS Core
checkpoints are whole-service, multi-tenant artifacts; this run does not invent
selective row restore into a live shared database. Each service owns a locally
transaction-consistent snapshot. The acceptance snapshot uses a bounded
coordinated write fence across covered Hosted Web, Finite Chat server, and SaaS
Core relationship writers, finishes or aborts staged attachment work, records
component watermarks, then commits one manifest. It does not claim an
instantaneous cross-database transaction for an unfenced online snapshot.

The existing Standard/Kata Agent Runtime `/data` is an external composition
dependency, not a member of this Recovery Set. It remains separately retained
and lifecycle-fenced while the empty chat/control-plane target is restored. Its
Agent Device identity and owner-claim ledger must compose with the restored
services, but provider-independent Agent Runtime snapshotting remains a
separate paid-admission gate.

The current Standard/Kata Runtime shares a physical host failure domain with
the covered services. Loss of that entire host or primary disk is therefore
not a covered failure in this run: the excluded Runtime `/data` dependency
would be gone too. Full-host recovery requires this chat snapshot to compose
with the separate Agent Runtime Recovery Set and empty-target proof. An
off-host chat snapshot is necessary for that later proof but cannot claim it
alone.

Reconstructible plaintext attachment cache is excluded. Restore starts with an
empty cache and proves that the encrypted reference and decryption metadata in
Device state can fetch the ciphertext from the restored Finite Chat SQLite
blob row, verify it, decrypt it, and render it. Any non-reconstructible local
attachment bytes must first become an explicitly named encrypted Recovery Set
member.

The snapshot contains no plaintext message export merely for operator
convenience. Encryption at rest and in transport, least-privilege snapshot
readers, append-only off-host writes, audited restore authority, and secret
names rather than values are mandatory. Recovery keys or passphrases must have
an independent protected custody path; material stored only on the failed host
does not count.

### Paid-cohort recovery objectives

These values are part of the proposed contract and must be changed before
activation if Paul does not accept them:

- Take a coordinated snapshot at least every 15 minutes. A customer-bearing
  promotion is blocked when the newest valid off-host snapshot is older than
  30 minutes; the declared maximum RPO is 30 minutes.
- Complete the empty-target drill within four hours; this is the initial RTO
  objective, measured from authorized restore start to the successful fresh
  Agent reply.
- Retain 48 hourly, 14 daily, 8 weekly, and 6 monthly recovery points.
- Validate every manifest at creation, run repository integrity verification
  at least weekly, and complete an empty-target drill at least every 30 days
  and before any customer-bearing release that changes recovery schema or
  authority.
- A stale but otherwise valid snapshot blocks automatic promotion. In an
  authorized Break-Glass recovery it may still be safer than permanent loss;
  record its age and possible RPO loss, keep the target isolated, and require
  explicit Recovery Authority acknowledgement before any traffic switch.

## Queue

Work top-down. Every retained item is required.

### P0 — Remove Room creation from recovery and navigation

- Add failing regressions for the exact incident before changing behavior:
  one Agent has several Topics and Chats, owner claim or bootstrap fails,
  recovery is retried, and Room count, canonical Room id, and reachable Chat
  ids remain unchanged.
- Delete the dashboard `/fresh` endpoint/action, `Start a fresh chat` control,
  Room-label helper, and unit/browser fixtures that encode recovery by
  `StartGroupChat`. Rewrite callers and assertions to the accepted shape.
- Keep three distinct user actions: retry retained-state load, retry the typed
  owner claim, and create a new Chat inside the current Agent conversation.
  Their copy and disabled/error states must describe the real operation.
- Route normal `New chat` through the existing typed Topic/Chat action inside
  the canonical Agent Room. Generate and durably replay one client intent key
  across button spam, request retry, and response loss; a separate explicit
  user intent still creates a separate Chat. Neither path may create a second
  Room or Home Topic.
- Finish and enforce the bootstrap split: retained state and the stored Agent
  binding load before any Runtime contact request. The contact endpoint may
  verify the bound Principal later; owner claim independently gates sending
  and Connections. Add regressions with the Runtime contact endpoint
  unreachable and with Hermes/model non-interactive; neither may hide history
  or change the binding.

### P0 — Persist and enforce the canonical Agent Room binding

- Make the `finitechat-hosted-device` Rust service own one versioned
  `AgentConversationBinding`-equivalent record and one durable ensure intent
  in its per-user encrypted store. Key it by stable Project, human Account
  Principal, and exact Agent Principal. Record the canonical Room, associated
  historical Rooms, future creation-purpose metadata, migration
  version/reason, and per-Agent navigation cursor.
- Keep Finite Chat's generic Room model free of Project/Runner/provider facts.
  SaaS Core provides authenticated stable Project/Runtime ownership, while the
  Hosted Web adapter translates the first Project-authorized Runtime contact
  Principal and later verification into typed Finite Chat actions.
- Authenticate the dashboard-to-Hosted-Device binding/ensure routes with a
  dedicated route-scoped service credential plus the verified WorkOS subject
  and Project access context. Reject cross-user, cross-Project, mismatched
  Runtime, forged Principal, missing scope, and generic service credentials.
- Make first onboarding one idempotent, crash-reconcilable phase machine.
  Persist a stable operation id and intent before the first Room mutation;
  adopt one eligible exact-member Room or create at most one initial Room
  through Finite Chat's idempotency contract; validate membership; then commit
  the adapter-local binding in one local transaction. Retry at every remote or
  local crash boundary adopts the same operation result and never creates a
  second Room. Do not claim a distributed transaction across the adapter and
  room server.
- On every reopen, validate that the bound Room is connected and contains the
  exact human and stored Agent Principals. A later Runtime contact observation
  may verify that Principal but is not needed to load retained state. A
  mismatch, missing member, ambiguous replacement, or Principal change becomes
  a typed recovery-required error; it never triggers automatic Room creation.
- During legacy migration with no binding and an unavailable Runtime contact,
  return the signed-in account's retained local conversation inventory in an
  explicitly read-only recovery state rather than an empty UI. Do not guess a
  Project mapping or permit sends. Resume the idempotent binding migration when
  the Project-authorized Principal becomes available.
- Replace Finite Chat Core's selected-matching-Room preference in
  `StartProfileChat`. SaaS Core remains free of Room identifiers and
  Room-selection logic; the Hosted Web adapter enforces its explicit binding
  through typed Finite Chat actions. Delete the regression that blesses a
  selected duplicate and add positive and negative tests for explicit binding,
  exact membership, ambiguity, and idempotency.
- Always issue or replay `agent.owner.claim` over the canonical bound Room.
  Reading or continuing an associated historical Chat cannot change the claim
  Room or reuse a result from that historical Room. If no canonical claim has
  succeeded, history remains readable while sending and Connections stay
  gated until one succeeds.
- Store last-open Room/Topic/Chat per Agent binding. Switching Projects or
  Agents and returning restores each Agent's own cursor without changing the
  binding. A missing/deleted cursor falls back deterministically to the newest
  reachable Chat, then Home, without creating data.
- Prove stable Runtime ids, legacy aliases, provider handles, display names,
  and host names all resolve to the same Project/binding and never choose a
  Hosted Web state directory.

### P0 — Make all retained Agent conversations reachable

- Project the product conversation tree across the canonical Room and every
  associated historical Room instead of filtering the sidebar to only
  `selected_room_id`.
- Present one current Home surface and a clear `Previous conversations`
  section for legacy Chats. Preserve Topic and Chat names and ordering, and
  disambiguate duplicate display names without exposing raw Room ids in normal
  navigation.
- Opening a retained historical Chat must display and continue that exact
  Room/Topic/Chat when its membership is still valid. `New chat` always targets
  the canonical Room regardless of which historical Chat is being viewed.
- Derive empty/loading/recovery UI from the complete associated projection.
  Never show an empty Agent merely because the selected Room has no Topics or
  the Agent is unavailable.
- Add one invariant at the presentation boundary: every retained projected
  Topic/Chat with valid membership is reachable from normal product
  navigation. Treat a retained-versus-visible count mismatch as a release
  failure, not an ignorable warning.
- Keep Connections and owner-claim authority attached to the exact Agent
  Principal/binding, not the selected historical Room. The Room-scoped claim
  transport always uses the canonical Room. Preserve the rule that history is
  readable while mutation controls fail closed.

### P0 — Migrate duplicate Agent Rooms without data loss

- Implement a one-time, versioned, transactional migration for Hosted Web
  state without a canonical binding. Use the only reliable legacy predicate:
  connected Rooms whose observable current membership is exactly the human and
  Agent Principals. Do not use display names. Do not claim that legacy state
  reveals whether `StartProfileChat` or `StartGroupChat` created a Room; future
  creation-purpose metadata keeps explicit group Rooms separate.
- If a valid binding already exists, preserve it. Otherwise select the
  canonical Room by the settled stable rule, associate every other eligible
  Room, and record migration version and non-secret reason. Never use current
  selection as the deciding fact.
- Record the exact Room, Topic, Chat, message, and blob identifier/count
  preflight only inside encrypted, access-controlled migration state and
  compare it after migration. Public logs/evidence receive counts and
  purpose-scoped keyed pseudonyms. The visible identifier set may grow but may
  not shrink.
- Make migration retry and crash recovery idempotent. Test failure before and
  after each durable write, two concurrent processes, schema upgrade replay,
  and an already-migrated database.
- Do not leave, delete, merge, copy, or re-encrypt legacy Rooms. Do not mutate
  server history to make the UI simpler. A newly observed duplicate after
  migration is retained, associated visibly, and reported as an invariant
  failure rather than silently selected.
- Do not infer owner-claim authority from the selected or newest historical
  Room. Reuse an exact durable successful claim only when it belongs to the
  chosen canonical Room; otherwise keep mutation gated and run a new typed
  claim there when the Agent is available.
- Provide only typed, authenticated diagnostic/open actions for later
  production repair. Runbooks may not instruct operators to edit encrypted
  client SQLite or SaaS Core rows by hand.

### P0 — Produce service-consistent Recovery Snapshots

- Give the Hosted Web Device and Finite Chat server service-owned snapshot
  commands using SQLite's online backup/snapshot API or an explicit bounded
  write fence. Do not copy live database, WAL, or SHM files as the snapshot.
- Stage each Hosted Web Device identity, client database, durable encrypted
  attachment references/decryption metadata, any encrypted sole-copy staged
  bytes, and manifest as one tenant bundle. Exclude reconstructible plaintext
  cache. A concurrent send, owner-claim replay, or attachment upload must
  resolve to a valid before-or-after local cut.
- Snapshot the whole Finite Chat server SQLite database through the same local
  consistency rule. Its encrypted blob ciphertext is already stored in the
  database, so blob rows and bytes share one validated checkpoint; do not
  invent a separate blob-filesystem phase.
- Add a bounded snapshot coordinator for the accepted cut. Hosted Web owns its
  tenant writer fence, Finite Chat server/traffic management owns the Room-
  server fence, and SaaS Core owns its relationship/database fence. Finish or
  abort staged attachment work, record each component watermark, take the
  service snapshots plus one complete SaaS Core PostgreSQL dump into an
  isolated target, then commit the manifest. Runner retains only its existing
  `/data` single-writer lifecycle fence and never inspects or fences
  chat-service files.
- Do not claim a global ACID transaction across these stores. Routine online
  snapshots must use either the same bounded coordinated fence or explicitly
  proven component-watermark and idempotent replay rules that restore the
  acknowledged set at the manifest cut.
- Emit snapshots into dedicated immutable staging paths. Change the Borg job
  to archive those service-owned artifacts and the existing consistent
  PostgreSQL dumps, not the live SQLite directories.
- Configure a real encrypted, append-only off-host target with a dedicated
  repository-path-scoped Borg credential. It may read repository metadata
  required by Borg but the server enforces append-only behavior; it cannot
  delete or compact archives. Hold the repository passphrase independently
  and record only secret names and custody locations in the repository.
- Enforce retention target-side or with a separate audited maintenance
  credential/job. The routine snapshot uploader never prunes, deletes,
  rewrites, or expires archives.
- Preserve the exact covered Nix closure/image digests and independently
  available artifacts, and document the Caddy/DNS/TLS, WorkOS configuration,
  route-scoped service credentials, snapshot keys, and Recovery Authorities
  needed to boot an empty isolated target. Secret values remain in their
  approved external stores.
- Enforce the proposed recovery-objective schedule, retention, maximum
  snapshot age, failed-job alerting, capacity monitoring, integrity checks,
  and restore-drill freshness. A timer being active is not success.
- Build restore validation for manifest version, component compatibility,
  hashes, sizes, identity/binding consistency, Recovery Authority, and
  complete required members. Reject missing identity, missing client store,
  mismatched server state, wrong key, corruption, partial upload, and unknown
  future formats before writing target state. Treat staleness as a promotion
  block and explicit Break-Glass decision, not as format corruption.

### P0 — Restore the same product onto an empty target

- Add a documented, repeatable restore path that begins with empty Hosted Web
  Device, Finite Chat server, and isolated SaaS Core service/database roots.
  Restore the complete Finite Chat and SaaS Core checkpoints rather than
  selecting tenant rows into a live shared database. Reusing the source live
  directory, VM, provider volume, key cache, or database does not count.
- Boot the target in isolated restore mode. Keep public traffic, Runner timers,
  provider mutations, Stripe/webhook processing, push delivery, outbound
  callbacks, and every nonessential external side effect disabled until
  validation and an explicit traffic-switch authorization succeed.
- Fence source writers for the acceptance snapshot and restore. Never run two
  Hosted Web Device, room-server, or SaaS Core writers against the same live
  state, and do not switch traffic until target validation passes. The
  separately retained Standard/Kata Agent Runtime remains lifecycle-fenced but
  is not restored by this run.
- Restore and verify the exact Account Principal, Device id, Agent Principal,
  canonical and associated Room ids, Topics, Chats, message/event ordering,
  encrypted attachments, outbox/idempotency records, binding, and per-Agent
  cursor.
- Reauthenticate through normal WorkOS Account Auth, replay the exact durable
  owner claim, open the last Chat, render its retained attachment, send a fresh
  turn, receive one Agent response, and restart the restored services once
  more.
- Test incomplete, corrupt, incompatible, wrong-authority, and stale snapshots.
  Invalid material leaves the prior target untouched and reports recovery
  required; none may mint a new identity, Room, or empty database. A stale but
  valid snapshot stays isolated and may proceed only through the documented
  Break-Glass acknowledgement of possible RPO loss.
- Measure and publish the observed recovery point and recovery time in the
  access-controlled operational/release record and compare them with the
  proposed RPO/RTO. Do not turn those observations into a broader unearned
  SLA.
- Keep the source recovery material through the accepted retention period.
  Runtime Retirement, subscription cancellation, failed launch cleanup, and
  restore success never imply Purge User Data.

### P1 — Unit, contract, and migration coverage

- Canonical binding: create, reopen, validate, exact-member mismatch, missing
  Agent, Agent Principal rotation, future creation-purpose exclusion, legacy
  creation-purpose ambiguity, concurrency, idempotent ensure phases, and
  partial-write recovery.
- Principal/context boundary: first Project-authorized Runtime contact
  observation, later verification, contact endpoint unavailable after binding,
  read-only retained inventory when legacy unbound migration cannot contact the
  Runtime, WorkOS subject/Project/Runtime mismatch, forged Principal, and
  positive plus negative route-scoped Hosted Device service auth.
- Selection: selected duplicate cannot hijack the binding; last Chat is
  remembered per Agent; invalid cursor falls back without data creation; URL
  aliases share the same binding.
- Conversation creation: profile-chat ensure creates at most one Room; new
  Topic/Chat leaves Room count and membership unchanged; one client intent's
  retries replay its result; two distinct user intent keys produce two Chats.
- Owner claim: exact durable replay occurs only in the canonical Room;
  historical selection cannot change the claim Room or transfer authority.
- Projection: canonical plus associated Rooms produce a complete, stable
  navigation tree; duplicate Home/name handling is deterministic; retained
  versus visible mismatch fails.
- Migration: valid existing binding, one legacy Room, several duplicates,
  future purpose-tagged group Room, untagged legacy exact-member Room, empty
  state, corrupt state, crash at every write, rerun, and version upgrade
  preserve identifier/count supersets.
- Snapshot format: deterministic manifest, integrity hashes, compatibility
  rules, secret redaction, active-WAL snapshot, coordinated watermarks,
  attachment consistency, empty plaintext cache recovery, partial bundle,
  wrong identity, wrong key, corruption rejection, and stale Break-Glass
  isolation.
- Replace every test that treats selection, display name, machine id, Runtime
  id, or provider alias as conversation identity. Do not keep old and new
  behavior behind fixtures or fallback branches.

### P1 — Integration and fault-injection coverage

- Reproduce the production failure with several Topics/Chats, create the old
  recovery duplicate through the historical group action, migrate, reload, and
  prove all original conversations are reachable while only one Room is
  canonical. Do not rely on unavailable legacy creation-purpose metadata.
- Exercise graceful and abrupt Hosted Web Device restarts, Finite Chat server
  restarts, dashboard rebuild/deploy boundaries, and Nix service restart. At
  every boundary compare Principal, Device, Room, Topic, Chat, message, blob,
  binding, and cursor identifiers before and after.
- Exercise Runtime contact endpoint outage, owner-claim timeout, invalid
  response, Agent startup delay, Hermes unavailable, canonical-Room claim
  replay, historical Chat selection, and Agent recovery. Retained state stays
  visible; send and Connections remain honestly gated; retry creates no Room
  and historical selection never changes claim authority.
- Exercise Runtime restart, Runtime Upgrade, failed upgrade/rollback, and
  recover-known-good on the currently accepted Standard/Kata path through
  existing typed lifecycle controls. Treat Runner as a black box: the Agent
  Principal and conversation binding remain exact, and a second turn continues
  the same Chat. Do not provision Phala or add chat-specific Runner behavior.
- Exercise concurrent tabs and repeated bootstraps, retries of the same New
  Chat intent, two independent New Chat intents, server response loss,
  room-server outage, outbox replay, stale `rev`, and duplicated delivery.
  Ordering and intent-scoped idempotency remain correct and Room count is
  stable.
- Exercise one WorkOS user with multiple Agents/Projects. Each restores its own
  binding and last Chat; navigation to one cannot change or hide another.
- Exercise a WorkOS email/profile change with the same stable subject. It
  resolves the same Hosted Web Device state and never creates a new identity.
  A different WorkOS subject with the same email must never reuse the first
  subject's hashed state directory, identity, binding, or history.
- Snapshot under sustained message and attachment writes, restore on an empty
  target, and compare the acknowledged identifier set covered by the snapshot
  manifest. Then inject missing identity, missing database, corrupt WAL-era
  state, missing blob, and wrong manifest/key cases and prove fail-closed
  behavior.
- Run the composition with the Finite Chat server offline and later restored;
  pending safe work replays once, server-only ciphertext is never presented as
  a recovered client, and no bootstrap replacement occurs.
- Restore the whole Finite Chat and SaaS Core checkpoints in isolated mode and
  assert that Runner/provider, Stripe/webhook, push, callback, and public-
  traffic side effects remain disabled. After an authorized controlled switch,
  the separately retained Kata Agent reconnects and completes one turn; this
  test must not be labeled Agent Runtime empty-target recovery.

### P1 — Browser acceptance harness and release invariants

- Use a dedicated synthetic account, Project, and Agent with synthetic
  sentinel Topics, Chats, turns, and attachments. Never reuse a teammate's or
  customer's account and never copy real message content into fixtures.
- Through the normal browser product, create several Topics and Chats, select
  a non-default Chat, reload/direct-link away and back, use `New chat`, retry a
  failed owner claim, and perform each covered restart/lifecycle control.
- After every control, assert the same Account/Device/Agent/canonical-Room
  identifiers, unchanged Room count, a non-shrinking reachable conversation
  set, restored per-Agent last Chat, ordered transcript, usable attachment,
  and same-conversation Agent reply.
- Add a migration browser fixture containing multiple retained exact-member
  Agent Rooms. The user must find and open every historical Chat without
  knowing a Room id, while the normal `New chat` path stays in the canonical
  Agent Room.
- Run the same assertions after an empty-target restore. Automated browser
  evidence remains an engineering gate; it does not replace Paul's final
  acceptance.
- Use normal typed APIs, assertions, and privacy-safe structured logs. Do not
  add a bespoke evidence form, report generator, or product instrumentation
  merely to demonstrate completion.

### P1 — Operational guardrails and recovery observability

- Add privacy-safe structured signals for unexpected Agent Room creation,
  duplicate binding discovery, binding/Principal changes, retained-versus-
  visible count mismatches, identifier-set shrinkage, migration failure,
  snapshot failure/age, integrity failure, restore age, and dual-writer fences.
- Logs and metrics may contain counts, synthetic identifiers, and bounded
  purpose-scoped keyed pseudonyms, not raw live ids, plain stable hashes,
  message bodies, attachment plaintext, secret values, private keys, JWTs, or
  unnecessary user identifiers.
- Make deployment preflight capture the synthetic canary's exact identity and
  conversation-set manifest only in encrypted, access-controlled operational
  state. Public evidence gets counts and keyed pseudonyms. Promotion stops if
  the post-deploy set shrinks, Room count grows unexpectedly, the binding or
  Principal changes, or the last Chat cannot continue.
- Require an off-host snapshot no older than 30 minutes and a successful
  empty-target drill no older than 30 days before customer-bearing promotion.
  A stale/failed Borg unit is a release blocker for promotion, while an older
  valid archive remains available only through explicit isolated Break-Glass
  recovery.
- Define explicit stop conditions: unexpected Room creation, hidden retained
  content, identity/binding change, snapshot inconsistency, corrupt restore,
  dual writers, or acknowledged-content loss. Stop promotion and customer
  admission immediately; do not "repair" by creating fresh state.
- Migration deployment requires a preflight service-consistent snapshot and a
  reviewed roll-forward/restore plan. Never run an older binary that can write
  an incompatible migrated schema merely as automatic rollback.
- Any later production selection repair uses typed authenticated actions and
  read-only pre/post verification under separate authorization. No manual SQL
  edits or account-wide destructive cleanup appear in the runbook.

### P1 — Durable documentation and local development handoff

- Add an ADR for the canonical Agent Room binding, associated historical
  Rooms, product projection, per-Agent cursor, migration rule, and the
  prohibition on Room creation during recovery.
- Update [ADR 0011](../../finitechat/docs/adr/0011-hosted-web-chat-uses-a-revocable-device.md)
  with the accepted binding and tested Recovery Set/empty-target behavior;
  preserve its revocable trusted-server posture.
- Update [`CONTEXT.md`](../../finitecomputer-v2/CONTEXT.md) with the named
  relationships among Project, Agent Principal, Hosted Web Device,
  canonical Agent Room, Topic, Chat, selection cursor, Recoverability
  Contract, Recovery Set, and Recovery Readiness.
- Document the complete storage/authority map: WorkOS identity, SaaS Core
  PostgreSQL, Hosted Web per-user identity and encrypted SQLite, Finite Chat
  server SQLite/encrypted blobs, Agent Runtime Device/owner-claim state,
  snapshot staging, off-host repository, and Recovery Authorities.
- Add a `Chats appear missing` runbook that distinguishes loading, selection,
  binding, owner claim, Agent interactivity, server reachability, corruption,
  and actual loss. It must start read-only and permit only typed safe actions.
- Add snapshot, retention, monitoring, Break-Glass authorization, empty-target
  isolated restore, external-side-effect fence, target switch,
  rollback/roll-forward, and post-restore validation runbooks. Name secret
  locations only.
- Preserve a fast Skyler-ready local path: root `just` commands start the full
  SaaS stack with synthetic multi-Room/multi-Chat fixtures, exercise the real
  Hosted Web surface, and run focused browser tests without production access
  or secrets. Document the exact commands and expected local URLs only after
  verifying them in the pinned Nix environment.
- Update the single customer-admission checklist so Stripe success cannot
  override chat continuity/recovery failure. Keep every other retained gate,
  including untrusted owner-claim, stuck-launch escape/reaper, Agent Runtime
  Recovery Readiness, every user-data product actually admitted, and remaining
  open questions in its owning run rather than pulling them into this queue.
- After acceptance, append only verified evidence to the Hosted Web continuity
  open question, extract the durable decisions, and close this run according
  to [`docs/runs/README.md`](README.md).

### P1 — Engineering gates

- Run formatting, clippy with warnings denied, and focused/workspace Rust tests
  for Finite Chat Core, client, Hosted Web Device, room server, RMP/product
  harness, and affected Finite Computer crates through the pinned Nix/root
  `just` paths.
- Run dashboard lint, unit tests, production build, and the focused browser
  suites, including migration, lifecycle, non-interactive Agent, per-Agent
  selection, and empty-target restore composition.
- Run the Finite Chat Hermes encrypted bridge suite and prove the continuity
  assertions come from the real hook. If the relevant gate can synthesize a
  passing report when the hook is absent, hard-cut that fallback; do not pull
  unrelated aliases or generic Hermes cleanup into this run.
- Run `just dev smoke` last. Record true environment/external failures as
  blockers; never weaken a gate, install host dependencies, or fabricate
  evidence.

### P1 — Authorized production canary and Paul acceptance

- Only after separate production authorization, take a verified preflight
  snapshot and deploy the exact tested revision through the normal workflow.
  Do not mutate existing teammate/customer conversation state.
- If a production migration canary is required, pre-seed the dedicated
  synthetic account with an exact-member duplicate using the already-shipped
  pre-cut `/fresh` action before deployment and under explicit authorization.
  The deployed revision must not retain or add a hidden duplicate-Room creator
  merely for testing.
- Use the dedicated synthetic production account to repeat the browser,
  restart/lifecycle, pre-seeded migration, off-host snapshot, and fenced empty
  chat/control-plane target checks with the same separately retained Kata
  Runtime. Stop on any invariant failure and do not admit customers.
- Paul completes the acceptance statement at the top of this run, including
  finding all retained Chats through normal navigation and continuing the
  restored conversation. Do not claim the run passed from automation or
  operator inspection alone.

## Stop and rollback rules

Stop implementation or promotion and rescope before proceeding if any of the
following occurs:

- a retained Topic, Chat, message, or attachment becomes unreachable;
- an operation changes the Account, Device, Agent Principal, binding, or
  canonical Room unexpectedly;
- Room count grows outside explicit initial onboarding or a legitimate user
  group-Room action;
- migration cannot account for every preflight identifier or is not
  idempotent;
- snapshot/restore cannot produce a coherent identifier superset, exact
  identity, readable attachment, owner-claim replay, and next turn;
- a service would start with partial identity/store state, silently mint fresh
  state, or permit two writers;
- acknowledged content covered by the snapshot cut is absent or duplicated;
- the off-host target, key custody, integrity check, alerting, or restore drill
  is missing or outside the promotion freshness budget; or
- completing the work would require changing the settled identity, privacy,
  thin-coupling, or customer-admission decisions above.

Before schema or migration rollout, create and validate the service-consistent
preflight snapshot. Prefer a forward fix. If restore is required, fence writers
and restore the complete declared Recovery Set; do not mix individual files or
run an older writer against a newer schema. Production repair, deployment, and
traffic switching always require their own explicit authorization.

## Out of scope

- Recovering, deleting, renaming, or rewriting a particular internal user's
  historical test conversations.
- Electron implementation, packaging, design parity, device linking, or native
  key custody.
- Stripe Checkout mechanics, Launch Codes, pricing, refunds, Portal behavior,
  or customer admission itself.
- Runner, Kata, or Phala implementation changes; Phala provisioning; limiter
  changes; runtime shell/filesystem/environment control; or a general control
  plane. If a newly observed provider-neutral lifecycle defect directly blocks
  the mounted-state composition test, stop and amend/authorize its owning run
  rather than adding chat-specific Runner coupling here.
- A general group-chat/room-management redesign, merging MLS histories, or
  making the dashboard/SaaS Core a Room authority.
- Final User Recovery Key, fully user-only Recovery Authority, higher
  Operator-Privacy Level, or operator-blindness claims.
- Agent Runtime provider-independent snapshot implementation, Sites and Brain
  data recovery, turn cancellation, stuck-launch cleanup, or untrusted
  owner-claim redesign. They remain separate customer-admission gates.
- Bespoke evidence products, report generators, or instrumentation whose only
  purpose is proving this run completed.

## Governing documents

- [`docs/monorepo-doctrine.md`](../monorepo-doctrine.md)
- [`docs/adr/0001-recoverability-precedes-operator-blindness.md`](../adr/0001-recoverability-precedes-operator-blindness.md)
- [`docs/open-questions.md`](../open-questions.md)
- [`finitecomputer-v2/CONTEXT.md`](../../finitecomputer-v2/CONTEXT.md)
- [`finitecomputer-v2/docs/identity-boundary-v1.md`](../../finitecomputer-v2/docs/identity-boundary-v1.md)
- [`finitecomputer-v2/docs/runtime-recovery-and-observability-plan.md`](../../finitecomputer-v2/docs/runtime-recovery-and-observability-plan.md)
- [`finitechat/CONTEXT.md`](../../finitechat/CONTEXT.md)
- [`finitechat/docs/adr/0011-hosted-web-chat-uses-a-revocable-device.md`](../../finitechat/docs/adr/0011-hosted-web-chat-uses-a-revocable-device.md)
- [`finitechat/docs/architecture.md`](../../finitechat/docs/architecture.md)
- [`finitechat/docs/engineering-style.md`](../../finitechat/docs/engineering-style.md)
- [`finitechat/docs/protocol-v1.md`](../../finitechat/docs/protocol-v1.md)
- [`finite-brain/docs/engineering-style.md`](../../finite-brain/docs/engineering-style.md)
- [`infra/README.md`](../../infra/README.md)
- [`infra/nixos/modules/backups.nix`](../../infra/nixos/modules/backups.nix)
