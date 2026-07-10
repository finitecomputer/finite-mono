# Finite Chat

Finite Chat is an encrypted chat and command transport. Its language keeps
Room as the first-level chat object while naming finer structure inside a Room.

## Language

**Room**:
A user-visible chat space backed by one MLS group and one server-ordered delivery log.
_Avoid_: Topic, conversation, direct room

**DM**:
A Room whose members happen to be two accounts. Not a distinct protocol
concept; several named Rooms with the same person are legal and useful.
_Avoid_: Direct room (retired server concept)

**Nostr Profile**:
Public user profile metadata attached to a Nostr account identity.
_Avoid_: Device identity, Room metadata

**Principal**:
The identity permissions attach to across Finite products. In native Finite
surfaces, a Principal is usually represented by a Nostr account id or npub.
_Avoid_: device, profile metadata, email-only viewer

**User Key**:
The human user's personal Nostr key. It may live in a native app keychain and
sign local user actions, but it is not shared with an agent.
_Avoid_: Agent principal, device key

**Agent Home**:
The filesystem root for one agent/runtime identity and local product state.
Agent-side tools use it to find the Agent Principal Key and product-specific
state.
_Avoid_: User home, project directory, Hermes plugin directory

**Agent Principal Key**:
The Nostr key controlled by an agent or runtime. It is the key shared by
agent-side tools such as `fchat`, `fsite`, and future `finite-brain` for that
agent. It does not make the agent the human user.
_Avoid_: User Key, device key, Nostr Profile

**Delegation**:
A Principal-approved authorization that lets an Agent Principal Key act with
bounded capabilities on behalf of that Principal.
_Avoid_: sharing a User Key, implicit trust

**Direct Agent Principal**:
An agent invited or shared with as its own visible Principal, similar to a
human participant in a Room.
_Avoid_: delegated personal assistant

**Device**:
A separately revocable Finite Chat client with its own key and durable local or hosted store.
_Avoid_: Account, Principal, UI surface

**Hosted Web Device**:
A Finite-operated Device whose server-held key and store let an account use Finite Chat from the SaaS web UI.
_Avoid_: Browser-local Device, browser E2EE, Agent Principal Key

**Device List**:
The user-facing list of a user's active and revoked Finite Chat devices.
_Avoid_: Profile, account settings

**Invite**:
A shareable Room entry credential that lets another device request admission to a Room.
_Avoid_: QR, link

**Invite Availability**:
An account-level readiness signal saying a Nostr account currently has Finite
Chat invitation material available, so it can be invited to a Room.
_Avoid_: Online, presence, device liveness, profile status

**Pending Room**:
A Room row created from a Scan Target before the local device can send messages in that Room.
_Avoid_: Half-joined room, unfinalized room

**Unavailable on Device**:
A rare repair state where a known Room cannot be entered because this device lacks usable local membership state.
_Avoid_: Needs attention, offline room, read-only cached room, hidden room

**Account Recovery**:
The restoration of a User Key followed by enrollment of a replacement Device under the same Finite Chat account.
_Avoid_: New account, WorkOS login, Device restart

**History Recovery**:
The restoration or member-authorized sharing of encrypted Room history that predates a replacement Device's membership.
_Avoid_: Server replay, account relink, cursor reset

**Room Admission**:
The Room owner's approval of a Pending Room into a sendable Room.
_Avoid_: Accept button, finalize step

**Room State**:
The user-visible readiness of a Room row: connected, waiting, joining, or unavailable on this device.
_Avoid_: Sync status, protocol phase, server reachability, offline

**Scan Target**:
A scanned or pasted value that the app routes into the appropriate Room flow.
_Avoid_: Join invite

**Runtime Connectivity**:
The current ability of the app runtime to reach Finite Chat server and wake transports.
_Avoid_: Room State, room lifecycle

**Undelivered Message**:
A user-sent Message that is saved locally but has not been accepted by the server-ordered delivery log.
_Avoid_: Failed message, delivery error

**Delivered Message**:
A Message accepted by the server-ordered delivery log.
_Avoid_: Sent message, locally saved message

**Delivery Failure**:
A Message send or upload attempt that reached the server and received a non-success response.
_Avoid_: Offline send, no connectivity, sync failure, undelivered message

**Outbound Delivery**:
The local send and server delivery status of a Message authored by the local device.
_Avoid_: Inbound delivery status, read receipt

**Read Receipt**:
A non-notifying Room event saying a device has read through a Message.
_Avoid_: Delivery receipt, outbound delivery

**Attachment Unavailable**:
An attachment that cannot be opened or sent from this device because its local bytes or durable local reference are missing or unusable.
_Avoid_: Delivery failure, room failure

**Attachment Cache Miss**:
A delivered attachment whose encrypted reference is known but whose plaintext bytes are not present on this device and may be downloaded on demand.
_Avoid_: Delivery failure, undelivered attachment

**Attachment Transfer Progress**:
The current upload or download activity for attachment bytes.
_Avoid_: Delivery status, read receipt, local timer

**Conversation**:
An application-level session inside a room.
_Avoid_: Room, MLS group

**Topic**:
A first-class user-facing conversation lane inside a room.
_Avoid_: Thread, room

**Segment**:
An app-owned context boundary inside a conversation.
_Avoid_: Topic, conversation, room

**Activity**:
TTL-bound encrypted intermediate state inside a room or conversation.
_Avoid_: Message, notification

**Runtime State**:
Structured current condition published by an agent runtime.
_Avoid_: Command, message, activity

**Finite Chat Daemon**:
The hosted, local, or runtime-resident control surface that owns one or more Finite Chat Devices.
_Avoid_: Hermes, agent, inference provider

**Finite Blob**:
A provider-neutral blob service contract used by Finite Chat, Finite Sites,
and future Finite Brain through scoped upload/download capabilities.
_Avoid_: chat attachment policy, direct bucket credentials, profile avatar cache

**Dev Diagnostics**:
A hidden surface for protocol, server, device, and local-state inspection.
_Avoid_: Settings, profile

**Debug Log**:
A bounded developer-facing record of recent runtime, transport, persistence,
and repair events. It may include timestamps, room/message ids, delivery
states, error categories, server URL, device id, config/store paths, and
redacted diagnostics. Export is an explicit user action through local copy or
share-sheet style handoff; it is not automatic upload or telemetry.
_Avoid_: Product copy, chat transcript, plaintext message bodies, attachment bytes, plaintext filenames/media metadata

**Product-State Harness**:
The end-to-end test harness that exercises real app state using a stable
scenario account, device identity, config path, and client store. It runs
simulator-first for deterministic automation, then repeats the same matrix on a
physical phone before first users. Offline cases keep the same configured
server URL and toggle reachability only. Physical-phone runs use a Mac LAN
server URL rather than loopback, build/sign/install the app on the phone, and
copy the app-container `FiniteChatStore` through explicit harness commands for
assertions. Harness-owned server runs refuse pre-existing listeners and probe
all-interface binds through local loopback before toggling reachability. Text
offline assertions check both the raw `client_app_outbox` key and Rust-decrypted
metadata: Sent locally, Undelivered by the server, same append message id, and
retained idempotency material. Attachment offline assertions attempt an
upload-required send while unreachable and require no sent bubble, no additional
outbox row, and no new server delivery effect.
_Avoid_: Unit fixture, row-level cleanup, transient diagnostics

## Relationships

- A **Room** contains zero or more **Conversations**.
- A **DM** is a **Room**, not a separate kind; per-topic lanes with one
  person are **Topics** inside a Room, or separate named Rooms — both legal.
- A **Nostr Profile** describes an account, not an individual device.
- A **Principal** is the identity resource permissions attach to; a
  **Nostr Profile** is display metadata about one possible Principal identity.
- A **User Key** belongs to the human user. It is not copied into an
  **Agent Home** and is not used as an **Agent Principal Key**.
- An **Agent Home** contains one agent/runtime's **Agent Principal Key** and
  local state. Multiple Agent Homes imply separate Agent Principal Keys unless
  explicitly configured otherwise.
- A **Delegation** grants bounded authority from a human-facing Principal to an
  **Agent Principal Key**. It is different from directly sharing with a
  **Direct Agent Principal**.
- A **Direct Agent Principal** is appropriate for bots or shared agents that
  should appear as their own participant or resource principal.
- A **Hosted Web Device** is one revocable **Device**; Electron enrolls a
  separate Device whose key and store remain local.
- A **Hosted Web Device** may decrypt its Rooms by design, so SaaS web chat is
  not described as browser E2EE even though the room server stores ciphertext.
- Restart or loss of one **Device** must not make it room authority or prevent
  another admitted Device from syncing and sending.
- **Account Recovery** restores signing authority and permits a replacement
  **Device**; it does not by itself provide **History Recovery**.
- A release promising retained chat history must cover the Room log, encrypted
  blobs, account material, Device state, and **History Recovery** in its tested
  Recovery Set.
- A **Device List** belongs to an account and is where users revoke devices.
- **Invite Availability** describes whether an account can be invited to a
  **Room** now; it is not onlineness, presence, or device liveness.
- **Invite Availability** is not a ranking or grouping category; it does not
  change People list identity or ordering.
- A **Pending Room** is a **Room** from the user's point of view, but its
  local device is still waiting for admission to complete.
- **Unavailable on Device** is a repair state, not an expected product path; a
  user-visible known **Room** should normally be enterable. If this repair
  state occurs in v1, the known **Room** remains visible as informational
  repair state instead of being hidden. Shipping without a repair, export, or
  surviving-Device history path is a Recoverability Contract gap.
- A **Room** with usable local membership state is enterable and sendable;
  missing or unusable local membership state is **Unavailable on Device**.
- Send eligibility follows usable local membership state, not current
  **Runtime Connectivity** or the result of the last server action.
- **Room Admission** is automatic for a valid Invite plus correct PIN.
- **Room State** names whether a Room is ready to use; it does not expose
  protocol maintenance phases.
- **Runtime Connectivity** is independent from **Room State**; a **Room** can
  remain connected while connectivity is temporarily unavailable.
- A user-sent Message is locally sent when it is saved and visible on the
  sender's device; it becomes a **Delivered Message** only when accepted by the
  server-ordered delivery log.
- In v1, attachment sends require upload before they become sent Messages. If
  upload is unreachable, the app gives transient feedback and does not create
  an **Undelivered Message**.
- An **Undelivered Message** that is later accepted by the server remains the
  same user-visible Message; delivery state changes without creating a second
  bubble.
- An **Undelivered Message** becomes a **Delivery Failure** only when that
  Message's send or upload attempt receives a non-success server response.
- Invite/profile/device-list actions are online-only controls in v1. If they
  are unreachable, Rust projects transient feedback/diagnostics and leaves
  room state, message delivery, and durable outbox state alone.
- A **Delivery Failure** belongs to an outbound Message; it does not change
  **Room State** by itself.
- A **Delivery Failure** is not an automatic-retry state; retry is explicit
  user action or a named repair flow.
- Retrying a **Delivery Failure** reuses the same outbound Message, visible
  bubble, and persisted idempotency material; retry does not create a new
  Message.
- **Outbound Delivery** exists only for Messages authored by the local device;
  inbound Messages do not have local send state.
- A **Read Receipt** is separate from **Outbound Delivery**; it can change how a
  delivered local Message is displayed, but it does not deliver, fail, or retry
  a Message.
- **Attachment Unavailable** is local attachment repair state; it is not a
  **Delivery Failure** and does not change **Room State**.
- **Attachment Cache Miss** keeps the Message delivered and the Room enterable;
  the app fetches and decrypts from the encrypted reference only after an
  explicit tap/download action.
- **Attachment Transfer Progress** describes byte movement only; it does not
  change delivery, failure, read, retry, or Room State.
- A **Topic** is a **Conversation** presented as a named lane.
- A **Conversation** contains one or more **Segments** when an app supports context resets.
- A **Segment** belongs to exactly one **Conversation**.
- **Activity** may be scoped to a **Room** or to one **Conversation**.
- **Runtime State** belongs to one agent runtime device and is projected by key.
- A **Finite Chat Daemon** owns one or more **Devices** and may observe an
  agent runtime, but it is not the agent or its inference provider.
- **Finite Blob** stores bytes behind scoped capabilities. Products own their
  own blob policy: encrypted chat attachments, site assets, brain artifacts,
  and profile avatars are not interchangeable authorization surfaces.
- **Dev Diagnostics** may expose server and device details; normal users should
  not need those details to use Rooms.
- A **Debug Log** belongs in **Dev Diagnostics**, not normal chat UI. It is
  explicitly redacted: no decrypted chat text, attachment bytes, plaintext
  filenames, or plaintext media metadata unless a future debug-only export flow
  is designed with explicit user confirmation. Before first release, export is
  local copy/share only; the app does not automatically upload debug logs.
- The **Product-State Harness** toggles server availability while preserving
  the same scenario account, device identity, server URL, config path, and
  client store. Cleanup happens only through one documented whole-store reset
  command between scenario runs.
- The **Product-State Harness** treats physical phone proof as the same product
  matrix after simulator success, not as a separate looser smoke test. The
  phone gate requires a phone hardware UDID or CoreDevice identifier, a signing
  team from `--ios-development-team` or `RMP_IOS_DEVELOPMENT_TEAM`, and valid
  local Xcode account/provisioning setup for the chosen development team.
- The **Product-State Harness** covers ordinary product flows; corrupted local
  state belongs to explicitly named repair tests.
- Pre-release app-support stores such as `FiniteChat/<device>` are reset-only
  state. Normal iOS startup does not recover or migrate them into product
  state.
- Pre-release app-message JSON files and legacy SQL app projection schemas are
  also reset-only. Runtime startup does not import `app-messages.json`, and
  client store open rejects old `client_app_messages`/`client_app_events`
  schemas instead of rewriting them. App projection timestamp columns with old
  `DEFAULT 0` schema defaults and extra app projection columns fail closed too.
  Legacy unencrypted client-store tables are reset-only too, even when empty;
  store open must fail instead of dropping those tables. Encrypted room metadata
  missing current lifecycle fields or carrying old `Offline` / `NeedsAttention`
  values fails closed instead of defaulting into a connected room. Encrypted
  outbox metadata missing timestamps or carrying old one-axis `delivery_state`
  payloads fails closed instead of becoming v1 outbound delivery. Encrypted
  app-state/profile metadata missing selected-room, revoked-device, or
  stale-profile fields fails
  closed instead of being defaulted into product state.
- Wrong server URL coverage is a diagnostics/configuration test, not the
  product offline path.

## Example Dialogue

> **Dev:** "If the user runs `/new` in the Deploys topic, do we create a new topic?"
> **Domain expert:** "No. Deploys stays the same Topic; `/new` starts a new Segment inside it."

> **Dev:** "Do we send a command whenever the dashboard needs status?"
> **Domain expert:** "No. The runtime publishes Runtime State, and the dashboard reads the latest projection."

> **Dev:** "If Hermes is broken, is chat broken?"
> **Domain expert:** "No. The Finite Chat Daemon still owns sync, Runtime State, and recovery commands while the host is online."

> **Dev:** "Does Electron reuse the Hosted Web Device?"
> **Domain expert:** "No. The same account enrolls Electron as another Device, and Electron keeps that Device's key and store locally."

> **Dev:** "After scanning an invite and entering the PIN, does the inviter need to tap Accept?"
> **Domain expert:** "No. The correct PIN is the approval ceremony; valid Room Admission is automatic."

## Flagged Ambiguities

- "New chat" can mean creating a new **Topic** from the app shell, or starting a
  new **Segment** inside an existing **Topic**. Resolved: app-level "New chat"
  creates a Room; `/new` inside a Topic creates a Segment.
- "Offline Room" was used to mean both **Room State** and missing **Runtime
  Connectivity**. Resolved: server reachability is **Runtime Connectivity**,
  not **Room State**.
- "Offline test" was used for both unreachable configured server and changing
  the configured server URL. Resolved: product offline tests keep the same
  server URL and toggle reachability; wrong URL is diagnostics coverage.
- "Needs attention" was used for a vague Room row problem. Resolved: the
  canonical state is **Unavailable on Device**, and it means local membership
  state is missing or unusable on this device.
- "Read-only cached room" was used for rooms without usable local membership
  state. Resolved: this is **Unavailable on Device**, not a normal product
  Room state.
- "Failed send" was used for both missing **Runtime Connectivity** and real
  send rejection. Resolved: locally saved sends without server acceptance are
  **Undelivered Messages**; only a non-success response to that Message's send
  or upload attempt is a **Delivery Failure**.
- "Server rejected the send" was used as a possible Room lifecycle signal.
  Resolved: it is a **Delivery Failure** for that Message unless local
  membership state is confirmed missing or unusable.
- "Sent" was used for both local save and server acceptance. Resolved: a
  locally saved user Message may be sent but **Undelivered**; server-log
  acceptance makes it a **Delivered Message**.
- "Delivery status" was used for all Messages. Resolved: **Outbound Delivery**
  belongs only to Messages authored by the local device.
- "Test cleanup" was used to mean editing individual rows into desired states.
  Resolved: the **Product-State Harness** preserves real app state inside the
  matrix and resets only whole explicit test stores between scenario runs.
- "Stale connected Room row" was used as a possible normal startup condition.
  Resolved: a connected row without usable local membership state is corrupted
  local state and is not part of the **Product-State Harness**.
- "Missing attachment cache" was used as a possible failed send. Resolved:
  missing or unusable local attachment bytes are **Attachment Unavailable**,
  not **Delivery Failure**.
- "Missing delivered attachment bytes" was used like pending attachment
  corruption. Resolved: when the encrypted reference is delivered, missing
  local plaintext is an **Attachment Cache Miss**.
- "Account recovery" was used as though restoring the nsec also restores old
  MLS history. Resolved: **Account Recovery** and **History Recovery** are
  separate requirements.
