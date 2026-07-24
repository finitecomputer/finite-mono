# Room/Topics/Chats Electron Daemon Plan

Status: active implementation plan.

## Goal

Build a desktop Finite Chat client that proves the room/topic/chat product
model end to end while keeping `finitechat-core::AppState` and `AppAction` as
the source of truth for CLI, iOS, Electron, hosted web bridge, and runtime
daemon behavior.

This is not a compatibility layer for legacy dashboard chat. The legacy web UI
is a source of proven frontend code and interaction detail, not an API or data
model to preserve. The implementation should deepen the Finite Chat model:
Rooms own membership and encryption, Topics own work lanes inside a Room, and
Chats own resumable context sessions inside a Topic. Protocol-level segments
are the durable backing record for product chats; "segment" is not user-facing
product language.

## Product Shape

- A Room is the MLS-backed membership and delivery boundary.
- A Topic is a user-facing grouping lane inside a Room.
- A Chat is a resumable context/session inside a Topic.
- A protocol Segment is the durable backing identity for a Chat.
- Every Room has a durable `Home` Topic. New Chats default to Home unless the
  user is inside another Topic.
- Home has a deterministic default Chat id, `home-chat`, so two devices that
  join the same Room do not invent different empty first Chats before either
  device sends a message.
- Creating a new Topic immediately creates a first Chat under that Topic.
- `/new` inside a Chat creates a new Chat in the current Topic.
- Messages belong to a Room, Topic, and, for all newly produced topic messages,
  a Chat.
- Unscoped legacy/plain Hermes messages are accepted into `Home`/`home-chat`;
  new product sends should produce explicit Topic and Chat ids.
- Runtime state, command results, and activity are projected through typed
  Finite Chat app events, not inferred from transcript strings.
- Electron and iOS render Rust-projected state; they do not own sync, room
  admission, retries, send eligibility, topic semantics, or runtime command
  policy.
- Adding people is a Room membership operation. Pasting an invite code joins a
  Room through invite-session admission; pasting an npub performs an MLS add
  using that account's published KeyPackage. It is not a pending notification
  channel.

## Musts

- Keep the daemon API as close as practical to serialized `AppState`,
  `AppUpdate`, and typed `AppAction`/intent shapes from `finitechat-core`.
- Add missing topic and segment concepts to `finitechat-core` first, then use
  them from Electron.
- Import real frontend code from the legacy chat UI where it carries product
  value: transcript rendering, composer ergonomics, attachments, run-state
  affordances, responsive layout, and existing shadcn/ui primitives.
- Keep frontend-only state limited to window/layout concerns, drafts, focus,
  local file handles, protocol-handler plumbing, and optimistic rendering that
  is reconciled by Rust state.
- Test against the current `finitecomputer-v2` runtime image shape:
  `deploy/finite-computer/images/runtime.Dockerfile`, the hosted `/invite`
  health surface, and the packaged Hermes/finitechat plugin layout.
- Register and handle `finite://join?...` in the desktop shell and route it to
  Rust-owned scan target handling.
- Preserve local-device E2EE language only for the Electron daemon mode where
  device secrets stay on the user's machine.

## Must Nots

- Do not recreate `chat.bootstrap`, `chat.list_threads`, `chat.send_message`,
  or the machine relay API as the desktop daemon contract.
- Do not map Topics through a legacy `Thread` compatibility model.
- Do not make "new chat" create a new Room. New Rooms are only for new
  membership boundaries.
- Do not hide topic/segment gaps in Electron-local state.
- Do not create a second desktop-only projection that iOS cannot reuse.
- Do not add hosted web E2EE product copy. Hosted web bridge mode is useful web
  chat, but it is a trusted server client if the server holds device secrets.
- Do not make npub invites look like out-of-band notifications. If the target
  account has no KeyPackage, surface that honestly and use invite codes for
  rendezvous.

## Phase 1: Core Topic/Chat Projection

Hard-cut `finitechat-core` so AppState exposes Topics with Chats underneath.

Scope:

- Add `AppTopicSummary` and selected-topic fields to `AppState`.
- Add `AppChatSummary` and `selected_chat_id` to `AppState`.
- Add topic-aware room details where useful.
- Project messages for the selected `(room_id, topic_id, chat_id)` tuple.
- Preserve unscoped room messages for older or system events.
- Preserve old topic messages without a chat id by projecting them into an
  inferred active/default Chat when possible; strict writes produce chat ids.
- Add `AppAction::CreateTopic`, `OpenTopic`, `RenameTopic`, `ArchiveTopic`,
  `OpenChat`, and `StartTopicChat`.
- Add topic-aware message send actions or extend send actions with
  `conversation_id` and `segment_id`.
- Ensure `Home` exists for every created Room and for joined Rooms once they are
  connected.
- Keep existing iOS-compatible behavior compiling while the iOS UI remains
  room-first.

Acceptance:

- Rust tests prove creating a topic appends/loads conversation metadata.
- Rust tests prove opening a topic changes selected messages without changing
  Room membership.
- Rust tests prove `/new`-equivalent chat creation appends a segment boundary
  in the selected Topic, not a new Room or Topic.
- Rust tests prove selecting an older Chat and sending keeps messages scoped to
  that Chat even when another Chat in the same Topic is active elsewhere.
- Rust tests prove a newly created Topic has its first Chat immediately.
- Rust and CLI integration tests prove two devices converge on the same
  default Home Chat and do not strand messages behind device-local empty Chats.
- Existing invite, send, attachment, poll, receipt, and device tests still pass.

## Phase 2: Daemon Surface

Add a daemon crate or CLI subcommand that exposes the core app model over a
local authenticated HTTP/SSE boundary.

Preferred shape:

- `GET /v1/app/state` returns serialized `AppState`.
- `GET /v1/app/updates` streams `AppUpdate` as SSE.
- `POST /v1/app/actions` accepts typed intent JSON that maps directly to
  `AppAction` or narrow daemon-only lifecycle actions.
- `GET /v1/healthz` returns process, server URL, device id, and store path
  diagnostics without plaintext message bodies.

Local security:

- Bind to loopback by default.
- Use a random per-install bearer token or Unix-domain socket where practical.
- Store daemon state under the platform app-support directory.
- Keep account secrets in the shared Finite identity path or an explicit
  platform secret-store bridge, not in Electron renderer storage.

Acceptance:

- Daemon can open a stable store, emit initial `AppState`, stream updates, and
  dispatch core actions.
- Daemon restart preserves identity, device id, selected room/topic, pending
  outbox, invite state, and sync cursors.
- Daemon can scan a `finite://join?...` URL and converge through Rust-owned room
  admission.

## Phase 3: Electron Remote-Dashboard Shell

This phase's original local-renderer plan has been superseded. Electron loads
the configured trusted dashboard origin; the dashboard owns Rooms, Topics,
transcripts, composer, attachments, and activity UI. The packaged app contains
the Electron main/preload scripts and `finitechatd`, not a second web build.

The sandboxed preload exposes only the versioned `local-chat-v1`,
`automatic-device-link-v1`, and `revoked-device-recovery-v1` capabilities.
Electron main validates the trusted frame, brokers bounded chat actions to the
local encrypted daemon, and automatically links the local Device after the
dashboard session is authenticated. Development uses `npm run dev:electron`
against the local dashboard; `npm run dev:electron:fixture` opens the dedicated
web-design route without exposing the local chat bridge.

## Phase 4: Cross-Device And Hosted Runtime Proof

Prove the product goal against real Finite Chat protocol participants.

Matrix:

- Electron daemon device and iOS app device share one Nostr account/npub but
  have different device ids.
- Hosted Docker runtime from `finitecomputer-v2` joins as its own runtime
  principal through the current runtime image shape.
- All participants use the deployed/default server unless explicitly testing a
  branch server.

Acceptance:

- Electron and iOS can both send as the same account; the runtime sees the same
  account/npub with distinct sender device ids.
- A Topic created from Electron is visible to iOS after sync.
- A Segment created in Electron is visible to iOS as an ordered boundary.
- Runtime replies and activity land in the selected Topic.
- Docker runtime restart preserves agent identity, invite room, topic state,
  Hermes memory, and Finite Chat state.

## Phase 5: Promote To iOS

Use the Electron-proven room/topic model to reshape the iOS app.

Scope:

- Replace the generic chat-list product surface with the same Room/Topic/Chat
  hierarchy as Electron.
- Use the top-left hamburger menu as the iOS sidebar: Rooms are membership
  contexts, Topics are headers, Chats are rows.
- The initial home screen composer sends into a new Chat in the Home Topic with
  the default agent Room.
- Support invite-code agent connect, Home chat, new Topic, new Chat in Topic,
  and shared multi-device sync.
- Defer people/contact lists until Electron and iOS share the same core
  multiplayer surface.
- Keep SwiftUI as a renderer and OS-capability bridge.

Acceptance:

- iOS uses the same topic/chat state and action names as Electron.
- Electron and iOS logged in with the same nsec show the same Topics, Chats,
  messages, and room members while presenting one user account with distinct
  devices.
- Product harness proves invite join, Home chat send, topic create, chat create,
  open/send, and sync on iOS Simulator and then physical device.
- No Electron-only topic semantics remain.

## Phase 6: Hosted Web Bridge And TEE Candidate

Only after local Electron daemon semantics are stable, evaluate a hosted bridge
or TEE deployment.

Rules:

- A hosted bridge may decrypt on behalf of a web UI only under
  `hosted_trusted_server_client` disclosure.
- A TEE bridge must use the same daemon surface, storage layout, and runtime
  evidence gates as local daemon/Electron where possible.
- Do not fork the app protocol for hosted web.

Acceptance:

- Hosted bridge renders the same room/topic projection as a derived web surface.
- Trust disclosure is explicit.
- The same daemon tests run against the TEE/hosted deployment with only
  provider-specific state mount and ingress differences.

## Protocol Consolidation Gates

The Electron daemon is a protocol proving surface, not a place to hide protocol
drift. A build is not considered playable unless the current app, daemon,
server, and Hermes runtime image all pass the same finitechat contract:

- `/health` reports the expected `server_contract_version`, finitechat source
  commit, and `source_dirty: false` before any production/default-server test.
- Runtime clients treat `server_contract_version` as a minimum
  transport/admission contract, not an exact encrypted app protocol match.
  Exact commit/contract matching belongs to release deployment gates.
- Delivery `MemberId` remains the compact opaque `fcdev1` route id derived from
  typed `DeviceRef`; JSON `DeviceRef` blobs must not re-enter HTTP delivery
  routing.
- Identity-sensitive checks read typed Finite identity from KeyPackage
  metadata, Welcome payloads, encrypted application payloads, or
  room-membership projections, then verify any compact route id only as a
  routing projection.
- The real Docker runtime image must complete room admission and answer at
  least two Hermes turns with an Electron-style long device id before promotion.
- Home-server failures must not block room-server invite finalization when the
  invite's room server is healthy; sync should report failure only when no
  useful progress was possible.

These gates are deliberately product-level. If Electron, iOS, CLI, or Hermes
need a compatibility shim to talk to the current server, stop and fix the core
protocol or release alignment instead.

## Evaluation Design

Core tests:

- Topic create/open/rename/archive projection.
- Chat-scoped message send and selected-message window.
- Chat/segment boundary projection and replay.
- Home Topic creation and default-chat selection.
- Invite scan/join does not lose selected topic state.
- Invite code generation from Electron produces a joinable URL.
- npub member add succeeds when the target account has a KeyPackage and
  honestly reports unavailable when it does not.
- Same-account multi-device visibility.

Daemon tests:

- AppState JSON schema round-trip.
- Action dispatch and update stream.
- Restart survival across pending send, pending invite, selected room/topic, and
  outbox drain.
- Loopback auth rejects unauthenticated requests.

Frontend tests:

- Imported legacy composer can send text and attachments through daemon actions.
- New chat creates a Topic.
- Segment creation renders an ordered divider.
- Protocol handler opens invite scan flow.
- Responsive desktop/mobile-width layouts do not overlap text or controls.

Runtime image checks:

- Build or inspect the latest `finitecomputer-v2` runtime image shape.
- Verify the image packages `finitechat`, Hermes, plugin files, entrypoint,
  health server, `/healthz`, and `/invite`.
- Run the local Docker canary when credentials/environment are available.
- If full canary cannot run, record the missing env and run static image-shape
  checks plus unit tests that validate the `/invite` payload contract.
