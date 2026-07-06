# Real State And Offline Plan

Date: 2026-06-17
Status: active plan

## Problem Statement

Finite Chat's protocol is designed to avoid user-visible impossible states:
once a device has joined a room and has durable local MLS/client state, the
chat should remain readable and locally usable whether the server is reachable
or not. A stopped or unreachable configured server, failed invite creation, or
launch-test fixture must not turn a real room into `UnavailableOnDevice` or
make the app look empty or read-only. A wrong server URL is separate
diagnostics/configuration coverage, not the product offline-send path.

The current code has the Rust storage pieces for encrypted client SQLite rooms,
selected room, messages, profile cache, and local outbox rows, plus product-flow
proof for durable text offline send and explicit retry after a real server
non-success response. The simulator product harness and committed UI assertions
now cover the text-offline matrix shape; the remaining first-user gate is the
same matrix on a physical phone with local Xcode provisioning in place.

## State Model

There is one real app state:

- Rust owns the durable product state: MLS state, room membership/mapping,
  selected room, local transcript projection, local outbox, profile cache,
  device list, read state, media cache references, and retry policy.
- Swift renders the Rust `AppState` and dispatches typed actions. Swift may
  hold transient OS handles or draft UI state, but it must not persist chat
  routing, room lifecycle, send eligibility, retry decisions, or protocol
  phase state.
- A room is connected when local state proves this device is an active local
  member. Server reachability is runtime connectivity, not a room lifecycle.
- A known room should normally be enterable. `UnavailableOnDevice` is a rare
  repair/corruption state for missing or unusable local membership state, not a
  normal test fixture or transport outcome. If it occurs, the known room stays
  visible with a clear repair/unavailable state; hiding it would look like data
  loss.
- A room with usable local MLS membership is enterable and sendable. A room
  without usable local MLS membership is `UnavailableOnDevice`; "read-only
  cached room" is not a normal product state.
- Send eligibility and composer availability derive from usable local MLS
  membership, not from runtime connectivity or the most recent server operation
  result.
- A stale pre-release/dev/test row that claims `connected` without usable local
  MLS membership is corrupted local state. The product harness must avoid it by
  using real product flows or whole-store reset, not by teaching normal startup
  to preserve malformed rows.
- A server rejection for a message send/upload is message delivery state, not
  room lifecycle. Auth, admission, or room-not-found style send errors mark the
  outbound message `failed` and may emit diagnostics/repair work, but they do
  not make a locally usable room `UnavailableOnDevice`.
- Offline launch reads local SQLite first. Network sync may then update the
  state, but network failure must not hide durable rooms or transcripts.
- Diagnostic/transient stores are explicit test tools. They must not be used
  by ordinary simulator, phone, RMP, Xcode, or Home Screen launches.

This follows ADR 0008 and the RMP rule: native UI is a pure view and bounded
capability layer over Rust-owned state.

## Acceptance Criteria

The product state work is done when all of these are true:

- Stable iOS launches, RMP launches, Xcode launches, Home Screen relaunches,
  and phone installs use the same bundle id, persisted runtime identity, and
  client SQLite store unless an explicit transient flag is supplied.
- Product proof is simulator-first for deterministic automation. Before first
  users, the same online/offline matrix must pass on a physical phone with the
  same state, cleanup, and diagnostic rules.
- The online/offline matrix preserves the configured server URL and toggles
  only reachability. Wrong-server-URL coverage is a separate diagnostics test,
  not the definition of offline send.
- Unit and integration tests that need fake server URLs or fake app support
  paths always pass isolated `applicationSupportURL` and `configStorageURL`
  values, or explicitly opt into `FiniteChatTransient/<device>`.
- Hidden Developer settings show the active server URL, device id, config
  file, store path, transient/stable flag, runtime status, and latest raw
  transport diagnostics, plus a bounded redacted debug log that can be copied
  or exported by explicit user action for fixing product-state failures. Export
  is local copy/share only before first release; there is no automatic debug-log
  upload or telemetry. That log may include event names, timestamps,
  room/message ids, delivery states, error categories, and redacted diagnostics;
  it must not include plaintext message bodies, attachment bytes, plaintext
  filenames, or plaintext media metadata.
- Normal chat list and room transcript surfaces never expose raw HTTP/runtime
  diagnostics as product copy, and they do not show persistent room-level or
  global offline/reconnecting banners just because sends are queued. Queued
  outbound state is shown on the message bubbles.
- Connected rooms never become `UnavailableOnDevice`, read-only, or
  composer-disabled only because the server is down, a message send/upload is
  rejected by the server, or an invite/profile/device-list action failed.
- Normal offline/degraded product paths never project a room as read-only; they
  either keep the room enterable and sendable from local membership or mark the
  rare visible repair state as `UnavailableOnDevice`.
- The canonical product matrix never starts from a stale connected room row
  without usable local MLS membership. That condition is covered only by
  explicitly named corrupted-state repair tests.
- A room with local MLS state can be opened offline, display its cached
  transcript, keep the composer enabled, accept new outbound messages, and keep
  those messages visible after force close.
- Restarting the server drains the local outbox exactly once per message and
  promotes local outbound bubbles to accepted server-backed transcript rows
  without duplicates, reorder flicker, or visible row replacement.

## Offline Send Semantics

Sending while offline is a first-class path, not an error edge.

Target user-visible outbound states have two axes: local send acceptance and
server delivery. Only messages authored by the local device carry outbound
delivery state; inbound messages do not have local send state.

- Local send acceptance: `sending` means the message is being saved locally;
  `sent` means it is saved locally and visible in the transcript.
- For attachment messages, `sent` requires Rust to verify the local file/cache
  path and persist the durable outbox projection. Before that, the message is
  still `sending` or a local composition error.
- Server delivery: `undelivered` means the sent message has not been accepted by
  the server-ordered delivery log; `delivered` means it has; `failed` means the
  message send or upload request reached the server and received a non-success
  response and now needs explicit retry or repair.
- iOS renders outbound state as compact bottom-right marks on outgoing bubbles:
  one check for locally `sent` but server `undelivered`, two checks for
  `delivered`, and filled checks when `read_count > 0`. Active `sending` and
  `failed` states may still show a spinner or retry control.
- The normal room UI stays quiet while messages are queued because runtime
  connectivity is unavailable. Do not add a persistent room-level or global
  offline/reconnecting banner for this path.
- Read receipts are display-only over delivered outbound messages. They never
  promote `undelivered` to `delivered`, clear `failed`, trigger retry, or apply
  to inbound messages.

Implementation rules:

- `SendMessage` persists the local outbox row before attempting transport.
- Text offline send is the v1 durable offline queue. Reply and poll sends may
  share the same Rust-owned outbox promotion rule if they are enabled in the
  release, but the first-user release gate is text.
- Swift media staging is not local send acceptance. In v1, attachment sends
  require upload before they become a chat message. If the upload path is
  unreachable, the attachment send fails immediately with transient feedback
  and no sent bubble, no `outbound_delivery`, and no durable outbox row.
- The Rust projection exposes `outbound_delivery` as an optional field present
  only on messages authored by the local device.
- Swift renders delivery marks from Rust-projected `outbound_delivery` and read
  receipt state; it does not infer delivery from local timers or transport
  errors, and it does not use read state to drive delivery or retry.
- Outbox rows keep the local message id, room id, sender, encrypted/plaintext
  projection needed for local display, local send state, server delivery state,
  local-to-server correlation material, bounded failure reason, and retry
  metadata.
- Attachment sends never store plaintext bytes in SQLite. A failed file/cache
  validation or unreachable upload does not create a sent bubble.
- After an attachment has been accepted as an encrypted blob-reference message,
  missing local plaintext is an attachment cache miss. The message remains
  delivered, and Rust fetches/decrypts from the blob reference only after an
  explicit tap/download action. Any fetch/decrypt failure is an attachment-view
  error or unavailable attachment, not delivery failure or room-state change.
- Upload/download progress must be Rust-projected real transfer state. Swift
  may show coarse in-flight attachment activity, but it must not synthesize
  byte progress from timers, local staging, or view state.
- Retry uses the same persisted row, local message id, visible bubble, and
  deterministic idempotency material. Swift never reconstructs an outbound
  payload from view state, retry never creates a new user-visible message, and
  failed rows are retried only through explicit user action or a named repair
  flow.
- Missing runtime connectivity, airplane mode, a stopped server, or a transport
  attempt with no server response leaves the saved message `sent,
  undelivered`; it is not a delivery failure.
- A saved message becomes `failed` only after that message's send or upload
  request receives a non-success server response.
- A failed outbound message does not mutate room lifecycle, disable the
  composer, or mark the room `UnavailableOnDevice` while local MLS membership
  remains usable.
- Sync, hint, invite, profile, and device-list failures do not mark messages
  failed.
- Invite creation, profile refresh, and device-list refresh are online-only
  control actions in v1. They are not queued offline operations; if unreachable
  they may show transient action feedback and hidden diagnostics, while leaving
  room state and message delivery state alone. `app_profile_scan_offline_without_cache_is_transient_only`,
  `app_invite_pin_offline_is_transient_and_keeps_scanned_invite`, and
  `app_device_actions_offline_are_transient_only` pin the current Rust-owned
  product-state behavior for those branches.
- On accepted append, Rust deletes the matching outbox row, removes the local
  placeholder, inserts the accepted server-backed message/event projection,
  and preserves the same visible message identity and transcript position,
  whether acceptance came from automatic drain or explicit retry.
  Delivery changes from one check to two checks in place; the projection must
  not show a duplicate, flicker through remove/insert, or make Swift infer the
  correlation.
- Automatic drain runs from Rust for `sent, undelivered` rows on bounded
  runtime ticks: startup, successful sync/hint wake, and opening a room. It
  uses per-tick limits and backoff so a dead server does not create an
  unbounded hot loop, and it does not wait for a separate projected
  connectivity field.
- Automatic drain never retries `failed` rows. Manual retry is for `failed`
  rows after a real server send error, unless a future explicit repair flow
  first clears the underlying cause.

## Online/Offline Test Matrix

All tests in this matrix use the same stable scenario account, device identity,
bundle id, server URL, config path, app container, and client SQLite store. The
test toggles server reachability for that same configured server URL; it does
not change device ids, data dirs, bundle ids, server URLs, config files, or app
containers inside the matrix. Cleanup happens only as a documented whole-store
hard reset between scenario runs.

`Server = off` means the same configured server URL is temporarily unreachable,
usually because that server is stopped. It does not mean launching the app with
a different or wrong server URL. Wrong-server-URL coverage belongs to hidden
Developer diagnostics and misconfiguration tests outside this product matrix.

Run this matrix simulator-first. Once it passes there, repeat the same matrix
on a physical phone before first users. The phone run is a release gate for the
same product state model, not a separate smoke test with weaker assertions.

Reset command contract:

```sh
cargo run -p finitechat-rmp -- reset-product-store ios --scenario <scenario> --device <device>
```

That command is the only accepted product-store cleanup path for the harness.
It deletes the whole explicit app-support/config/client-store root for the
named scenario and device, and logs the deleted root. It must not run targeted
SQL cleanup, mutate room rows, or partially clear config/state.

Implementation default: `reset-product-store` refuses to run unless the caller
provides both `--scenario` and `--device`, the resolved path is inside the
explicit harness root, the canonical harness root remains inside the canonical
workspace, the `.state`/product-harness/platform/scenario/device store roots are
not symlinks, and the target is not the default product app support path. This
is an engineering guardrail, not a product decision.

| Scenario | Server | Required proof |
| --- | --- | --- |
| First launch empty | on | app opens stable store, no transient flag, no fake diagnostics |
| Create room | on | room appears in list, opens transcript, local MLS mapping exists |
| Send text | on | message is accepted, survives force close and relaunch |
| Relaunch cached room | off | chat list and transcript render before network sync, composer available from local MLS membership, no `UnavailableOnDevice`, no persistent offline/reconnecting banner |
| Send text offline | off | outbound bubble appears as `undelivered`, survives force close, and normal room UI stays quiet except for message-level marks |
| Restart server | on | undelivered outbox drains automatically, local bubble updates in place to delivered exactly once |
| Peer sync | on | second real client receives exactly one copy |
| Send attachment offline | off | upload-required send fails immediately with transient feedback; no sent bubble, no `outbound_delivery`, no durable outbox row, room remains sendable |
| Send attachment online | on | upload/send promotes through real finitechat-server `/upload` and `/blobs/{sha256}` routes to an accepted encrypted blob-reference message if attachments are enabled in v1 |
| Kill during attachment upload | mixed | relaunch shows no sent bubble unless upload/send reached accepted blob-reference delivery; never creates an undelivered offline media row |
| Relaunch after media delivery | on | delivered blob-reference message survives relaunch and keeps stable visible identity |
| Delivered attachment cache miss | mixed | delivered message remains delivered; app waits for explicit tap/download before fetching/decrypting from the blob reference, and failure only affects attachment view |
| Invite action offline | off | online-only action failure is surfaced as transient action feedback/diagnostics, is not queued, and existing room remains readable and sendable |
| Profile/device-list offline | off | online-only refresh is not queued; stale cache or dev diagnostics only; room lifecycle and message delivery do not change |
| Retry failed send | on | explicit retry reuses the same persisted outbox row, local message id, visible bubble, and idempotency material; success promotes that bubble in place without a duplicate |
| Kill app during retry | mixed | relaunch shows either the same durable outbox row or the accepted row with the same visible identity, never neither and never duplicate |
| Corrupt local attachment staging | mixed | explicit repair test only; missing local bytes project attachment unavailable/composition failure, not delivery failure or room-state change |

Diagnostics-only coverage outside the product matrix:

| Scenario | Required proof |
| --- | --- |
| Wrong server URL | Hidden Developer settings show the active wrong URL and redacted transport diagnostics; durable rooms remain enterable from local MLS state; no outbound message is marked `failed` unless its send/upload reaches a server and receives a non-success response |
| Unavailable on device repair | A known room with confirmed missing/unusable local MLS remains visible in the chat list as informational `UnavailableOnDevice` state; it is not hidden and does not offer rejoin/rescan actions in v1 |

## Implementation Phases

### Phase 1: Delivery Projection Hard Cut

- This phase comes before the canonical product harness. Do not build the
  online/offline product matrix against legacy `pending`/`sent`/`failed`
  semantics, because that would preserve the wrong assertions.
- Replace the one-axis `pending`/`sent`/`failed` Rust delivery projection with
  optional `outbound_delivery` containing local send acceptance plus server
  delivery state.
- Project offline sends as locally `sent` and server `undelivered`.
- Project non-success server responses from message send/upload as `failed`
  with a bounded diagnostic reason.
- Keep message send/upload rejection scoped to `outbound_delivery.failed`; do
  not mutate room lifecycle from the send result.
- Preserve stable visible message identity when an undelivered local send is
  accepted by the server or when a failed outbound message is retried and then
  accepted.
- Make `RetryMessage` a Rust action over the existing outbox row; it must reuse
  the local message id and idempotency material rather than creating a new
  message.
- Remove universal delivery state from inbound messages.
- Render outgoing message delivery as checkmarks instead of status text:
  one check for local sent/undelivered, two for delivered, and filled checks
  when at least one peer has read.
- Keep read receipts display-only: filled checks only apply after delivery and
  never affect delivery, failure, retry, or outbox drain.
- Regenerate native bindings and update Swift UI/tests against the new shape.
- Hard-cut old pre-release dev/test state instead of adding compatibility
  migrations or legacy shims.

### Phase 2: Room Lifecycle Cleanup

- This phase also comes before the canonical product harness. Do not build the
  online/offline product matrix against legacy `Offline`, `NeedsAttention`, or
  normal read-only cached room behavior.

- Delete `AppRoomState::Offline` from Rust, stored room state, native bindings,
  and Swift UI switches.
- Rename `NeedsAttention` to `UnavailableOnDevice`.
- Audit every assignment of `UnavailableOnDevice`.
- Define `UnavailableOnDevice` as a rare repair state only, not a transport
  state or ordinary product path for a known room. When it occurs, keep the room
  visible with informational repair/unavailable state instead of hiding it. Do
  not add rejoin/rescan actions in v1.
- Hard-cut normal read-only cached room projection. Missing or unusable local
  MLS membership projects as `UnavailableOnDevice`; usable local MLS membership
  keeps the room enterable.
- Do not add normal-product compatibility handling for stale pre-release rows
  that claim `connected` without local MLS membership. Hard reset explicit
  dev/test stores or cover the condition in a named corrupted-state repair test.
- Treat auth/admission/room-not-found style send failures as failed outbound
  messages plus diagnostics/repair hooks, not direct room-state transitions.
- Keep runtime connectivity in `AppState.status`, `toast`, and hidden Developer
  diagnostics until a dedicated Rust-projected connectivity field exists.
- Do not surface runtime connectivity as a persistent room-level/global
  offline or reconnecting banner for queued sends. Message-level delivery marks
  are the normal chat UI. Online-only action failures such as invite, profile,
  or device-list refresh may use transient action feedback.
- Keep invite creation, profile refresh, and device-list refresh online-only in
  v1. Do not add durable offline queues for these control actions before first
  users unless the product decision changes.
- Keep the composer available when local MLS membership exists, even if the
  most recent server operation failed.

### Phase 3: Canonical Product Harness

- Build this only after Phase 1 has replaced the legacy delivery projection and
  Phase 2 has removed legacy room lifecycle states. The harness asserts the
  product model, not transitional shapes.
- Add a product-state test harness that launches the iOS app with the same
  scenario account, device identity, bundle id, server URL, persisted config
  path, app container, and client SQLite path across server reachability
  toggles.
- Build room fixtures through real product flows: create, invite, join, open,
  send, kill, relaunch. Tests must not manufacture impossible room rows unless
  the test is explicitly about repair of corrupted local state.
- The harness must not start from stale connected rows with missing local MLS
  state. If such a row appears in pre-release/dev/test data, reset the whole
  explicit store before product-matrix runs.
- Test cleanup mirrors real user boundaries: reset only whole explicit test
  stores/app-support roots between scenarios, never by editing room rows or
  partially clearing state inside a scenario.
- Add the single documented reset command before the harness is considered
  usable. For iOS, the command contract is
  `cargo run -p finitechat-rmp -- reset-product-store ios --scenario <scenario> --device <device>`.
  Ad hoc cleanup scripts, targeted SQL deletes, room-row mutation, and partial
  config clearing are forbidden. The command must require both `--scenario` and
  `--device`, resolve only inside the explicit harness root, keep the canonical
  harness root inside the workspace, reject symlinked harness/platform/scenario
  and device store roots, refuse the default product app support path, and log
  the deleted root.
- Make transient diagnostics opt-in in every launch helper.
- Add a startup assertion/test that stable product launches cannot write test
  config to the normal app support directory.
- Document the exact store path in hidden Developer settings and in test logs.
- Keep the configured server URL stable inside product offline runs. Put
  wrong-server-URL assertions in diagnostics-only tests.
- Current simulator entry point:
  `cargo run -p finitechat-rmp -- product-harness ios-simulator --scenario text-offline --device <device> --server-url <url>`.
  The configured server URL must be an origin-only `http://host:port` URL. The
  command uses `.state/product-harness/ios/<scenario>/<device>` as the explicit
  support root, writes the harness config there, launches the app with
  `--finitechat-product-harness-root`, and toggles reachability by stopping and
  restarting the server bound to that same configured URL. `--dry-run` resolves
  and prints the same paths/phases without creating harness directories,
  resetting stores, building, launching, or writing config. `--no-reset` refuses
  to overwrite an existing harness config with a different server URL or device
  id; changing either identity requires a whole-store reset. The current slice
  logs paths and asserts server-side chat delivery, local projection drain
  shape, and peer receipt: one application-delivery effect and one delivered
  local outbound message after the online phase, still one
  application-delivery effect plus two local outbound messages with exactly one
  undelivered id, the same online delivered message id still present after
  offline force-close, and exactly one durable `client_app_outbox` row while
  unreachable. The offline outbox assertion now also opens the
  encrypted row through the Rust runtime and verifies local state `sent`, server
  delivery state `undelivered`, append-request message id matching the visible
  bubble, and retained idempotency material. The harness assertion helper fails
  closed if a non-empty outbox phase omits expected room identity or local/server
  delivery states, rejects outbox rows for the wrong room, or if an empty phase
  carries stale row identity/state expectations. While the
  server is still stopped, the harness launches the app again with a synthetic
  in-memory attachment send and asserts fail-fast behavior: no new visible
  outbound bubble, no additional durable outbox row, and no new server delivery
  effect. After same-URL
  restart/drain the harness expects two application-delivery effects plus two
  delivered local outbound messages, with both the original online message id
  and that same formerly-undelivered message id present, the latter promoted in
  place and present in the server delivery log only after restart/drain, and no
  remaining outbox row. A 2026-06-17 simulator run using
  `--device codex-sim-v1 --server-url http://127.0.0.1:18987` passed this
  matrix: outbox rows were `0 -> 1 -> 1 -> 0`, the offline row stayed
  `sent/undelivered` with retained idempotency material while unreachable, and
  the same visible message id was delivered after same-URL restart.
  The harness creates a peer through invite/PIN/admission before the offline
  phase and asserts the peer receives the promoted message
  exactly once as inbound state. iOS delivery marks expose stable accessibility
  identifiers for `sent-undelivered`, `delivered-unread`, `delivered-read`, and
  `failed`, and a simulator screenshot from the harness store visually shows
  the delivered transcript with bottom-right double checks and no persistent
  offline/reconnecting banner. A XcodeBuildMCP `snapshot_ui` pass over that
  transcript found the product-harness message bubbles with `AXValue = "two
  checks"` and no normal offline/reconnecting banner element. The bubble
  checkmark accessibility assertion is now repeatable in
  `OutboundDeliveryAccessibilityTests`, and a repeat XcodeBuildMCP `snapshot_ui`
  spot-check after that change still found the two product-harness transcript
  bubbles with `AXValue = "two checks"` and no offline/reconnecting banner. The
  normal notice path now has a committed guard that queued offline text state
  does not request a `NoticeBar`. The committed
  `testConnectedSavedRoomCanSendWhileRuntimeStatusIsOffline` XCTest proves Swift
  still dispatches `SendMessage` for a connected Rust-owned room while runtime
  status is offline, while
  `testUnavailableSavedRoomKeepsCachedMessagesButCannotSend` proves
  `UnavailableOnDevice` does not dispatch text, poll, attachment, invite
  creation, or invite PIN submission from Swift, and the Swift model no longer
  exposes a room retry wrapper, while the Rust
  `app_corrupted_state_unavailable_room_create_invite_is_transient_only` test
  proves `CreateInvite`, `SubmitInvitePin`, startup ticks, and stale
  pending-invite metadata stay transient/inert and preserve the
  `UnavailableOnDevice` room state without durable side effects, and
  `testProductHarnessDeliveredTranscriptPresentationHasNoNormalOfflineBanner`
  XCTest now ties the product-harness delivered transcript shape together:
  the runtime may report offline status and the retry toast, but the selected
  transcript projects the two delivered product-harness bubbles with
  `AXValue = "two checks"` and no normal `NoticeBar`.
- Current physical-phone entry point:

  ```sh
  cargo run -p finitechat-rmp -- product-harness ios-device \
    --scenario text-offline \
    --device <device> \
    --udid <phone-udid-or-coredevice-id> \
    --ios-development-team <team-id> \
    --server-url http://<mac-lan-ip-or-hostname>:<port>
  ```

  `RMP_IOS_DEVELOPMENT_TEAM=<team-id>` may be used instead of the flag. This
  value is passed to Xcode as `DEVELOPMENT_TEAM`, so it must be the
  provisioning-team/App ID prefix value, not just the suffix shown in the
  signing certificate name. On the current Mac, Xcode signs with
  `Apple Development: Paul Miller (Y392XZ3MST)`, but the generated entitlements
  and embedded profile use team identifier `JBLHZ83X6T`; therefore the harness
  team value is `JBLHZ83X6T`. The phone harness requires a non-empty `--udid`
  and a development team before `--dry-run` or build/install work, so the
  physical matrix fails early on missing target/signing inputs rather than
  inside Xcode provisioning. The build/install path accepts either the Xcode
  hardware UDID or the CoreDevice identifier printed by `xcrun devicectl list
  devices`, then normalizes to the hardware UDID used by `xcodebuild`.
  The phone platform rejects loopback configured server URLs because
  `127.0.0.1` on a phone is the phone itself, not the Mac-hosted harness
  server. Its default bind address is `0.0.0.0:<server-url-port>` so the app
  keeps the LAN configured URL while the local harness server listens on all Mac
  interfaces; pass `--server-addr <addr:port>` only to override that default,
  and physical-phone runs reject loopback bind addresses.
  Readiness probes map unspecified bind addresses such as `0.0.0.0` to loopback
  on the Mac, and the harness refuses to start if that probe address is already
  reachable so the matrix owns the finitechat-server instance it toggles.
  Unlike the simulator, the phone uses the app's normal app container;
  the harness force-closes between phases, pulls `Library/Application
  Support/FiniteChatStore` out through `devicectl` for Rust projection
  assertions, and pushes the host-side peer-admission update back before the
  offline phase. The pull path fails closed unless `devicectl` produces either
  a nested `FiniteChatStore` directory or direct store-root contents with
  `account-secret.hex` and `client.sqlite3`; it does not guess at unrelated copy
  shapes or move an ambiguous temporary directory into the harness state. A
  2026-06-17
  unit-test slice now exercises the physical-phone dry-run command path directly:
  valid dry-runs do not create `.state`, `RMP_IOS_DEVELOPMENT_TEAM` is accepted
  as the signing-team source, and loopback configured URLs, loopback bind
  overrides, missing UDIDs, or missing signing teams fail before any build/install
  work. A separate 2026-06-17 run built the `aarch64-apple-ios` Rust slice,
  generated the XCFramework, normalized the attached phone's CoreDevice
  identifier to hardware UDID `00008150-0010149A26F0401C`, and proved
  `xcodebuild` succeeds with `DEVELOPMENT_TEAM=JBLHZ83X6T` for bundle id
  `computer.finite.finitechat`. The same-matrix phone pass remains a first-user
  gate until the full product harness completes on that provisioned phone.

### Phase 4: Durable Outbox Product Semantics

- Verify current Rust outbox paths for text first. Reply, poll, and voice-note
  sends may use the same rule if enabled, but the first-user release gate is
  text offline send.
- Keep attachments out of the v1 offline outbox. Attachment sends require
  upload; if upload is unreachable, fail immediately with transient feedback
  before creating a sent bubble, `outbound_delivery`, or durable outbox row.
- Run attachment online proof through real finitechat-server `/upload` and
  `/blobs/{sha256}` routes with durable server blob storage if attachment UI is
  enabled in v1. `MemoryBlobStore` and mock blob stores are unit-level tools
  only, not acceptable product proof.
- Treat missing plaintext for accepted blob-reference messages as attachment
  cache misses: wait for explicit tap/download, then download/decrypt from the
  reference and keep message delivery and room state unchanged if that fails.
  `app_runtime_downloads_attachment_blob_to_verified_local_cache` covers the
  force-close shape: after a remote delivered blob-reference message is synced,
  reopening on the same live server still projects no local path and no
  download progress until explicit download actions run. The Swift app model
  now also refuses `DownloadAttachment` dispatch for local, missing-reference,
  uploading, or already-downloading attachments, so the explicit cache-miss
  rule is guarded at the action boundary rather than only by individual views.
- Keep missing/corrupt local attachment staging out of delivery failure
  semantics; project it as attachment-unavailable/composition failure and cover
  it only in explicit corrupted-state repair tests.
- Omit byte-level upload/download progress until the Rust blob transport can
  project real transfer progress. Coarse Rust-owned in-flight states are fine;
  fake Swift progress is not.
- Move undelivered outbox drain into bounded runtime ticks instead of relying
  on the user to tap Retry or waiting for a separate connectivity signal.
- Keep `failed` rows out of automatic drain; expose user retry and bounded
  diagnostics for real server rejection. Explicit retry reuses the existing
  outbox row, visible bubble, local message id, and persisted idempotency
  material. `app_server_rejected_text_send_requires_explicit_retry_with_same_outbox_identity`
  proves this against a real HTTP non-success response.
- Keep a hidden Developer view of raw transport errors and a bounded
  copy/export debug log for runtime, transport, persistence, and repair events.
  The log is redacted by contract and excludes plaintext message bodies,
  attachment bytes, plaintext filenames, and plaintext media metadata. Export
  requires explicit user action and stays local copy/share only before first
  release; do not add automatic log upload or telemetry.

### Phase 5: End-To-End Product Proof

- Add an iOS simulator E2E first that creates or opens a real room, sends
  online, kills the app, turns the same configured server off, relaunches with
  the same server URL, sends offline, restarts the server, and verifies
  delivery. The 2026-06-17 `codex-sim-v1` run on
  `http://127.0.0.1:18987` passed with one online delivered message, one offline
  durable undelivered row while unreachable, and both messages delivered after
  same-URL restart.
- Keep the simulator peer proof in the harness: before the offline phase, join
  a peer through invite/PIN/admission, and after same-URL restart assert the
  promoted message is received exactly once as inbound state.
- Durable attachment offline send is not part of the first-user release gate.
  The required v1 proof is fail-fast offline attachment behavior: no sent
  bubble, no durable outbox row, and no room-state change. The product harness
  now attempts an in-memory attachment send while the same configured server is
  unreachable. Delivered cache-miss downloads remain explicit user actions in
  the transcript, voice/file rows, and media gallery, with the Swift app model
  refusing direct download dispatch unless Rust-projected attachment state is a
  remote cache miss; lifecycle hooks do not auto-fetch blob references. If
  broader attachment E2E ships, online attachment
  proof uses real finitechat-server blob routes rather than `MemoryBlobStore` or
  mocks.
- After simulator success, repeat the same matrix against a physical phone
  before first users. The phone gate must use the same assertions and store
  rules as the simulator run. Use an origin-only Mac LAN URL, not loopback. The
  harness defaults the local server bind address to
  `0.0.0.0:<server-url-port>` for physical-phone runs so the phone can reach
  the same configured URL; local readiness is probed through loopback, and an
  already reachable probe address, loopback phone bind, missing UDID, or missing
  development team is a hard failure.

### Phase 6: Cleanup And Guardrails

- Delete stale dev stores that were only useful for malformed tests, or provide
  an explicit documented whole-store reset command for them.
- iOS startup must not scan, recover, or migrate pre-release
  `FiniteChat/<device>` app-support stores. `RuntimeConfigTests` now pin those
  directories as ignored reset-only state.
- Core startup must not import pre-release `app-messages.json`, and client store
  open must not rewrite old app projection schemas. Pre-release
  `client_app_messages`/`client_app_events` tables with plaintext rows or
  missing timestamp/nonce/ciphertext columns fail closed and require the same
  documented whole-store reset. App projection timestamp columns with old
  `DEFAULT 0` schema defaults or extra columns also fail closed. Legacy
  unencrypted tables
  (`client_openmls_storage`, `client_rooms`, `client_profiles`) are reset-only
  too, even when empty; opening a store with those tables must fail instead of
  dropping them. Encrypted room metadata with missing current lifecycle fields
  or old `Offline` / `NeedsAttention` values must fail closed rather than
  defaulting into a connected room. Encrypted outbox metadata with missing
  timestamps or old one-axis `delivery_state` payloads must fail closed rather
  than being interpreted as v1 outbound delivery. Encrypted app-state/profile
  metadata with missing selected-room, revoked-device, or stale-profile fields
  must fail closed rather than being defaulted into product state.
- Keep exactly one documented product-store reset command. Remove or forbid any
  ad hoc cleanup script that can edit rows or partially clear state.
- Add lint/test guards for test config leaking into product app support paths.
- Keep the technical debt ledger updated until the product-state matrix is
  reliable enough to delete the debt row.

## Done List

This work is done when a new session can run one documented local command set
and prove:

1. A saved room opens with the server on.
2. The same saved room opens with the server off.
3. A message can be sent while the server is off.
4. Force close and relaunch keep that undelivered message visible.
5. Restarting the server delivers that message exactly once.
6. A peer client sees the message exactly once as inbound state.
7. No normal user surface says the room is offline, unavailable on this device,
   or broken only because the server was temporarily unreachable.
8. The simulator matrix passes first, and the same matrix passes on a physical
   phone before first users.
9. Offline proof toggles reachability of the same configured server URL; wrong
   server URL behavior is covered only as diagnostics/misconfiguration proof.
10. Explicit retry of a failed outbound message reuses the same visible bubble,
    local message id, and persisted idempotency material.
    iOS exposes a retry affordance for every failed local outbound bubble, not
    only the last message in a grouped run, and Swift dispatches `RetryMessage`
    only for local outbound messages whose Rust-projected server delivery state
    is `failed`.
11. Queued offline messages do not trigger persistent room-level/global
    offline or reconnecting banners; the room stays quiet except for
    message-level delivery marks and transient feedback for online-only actions.
12. A known room in `UnavailableOnDevice` remains visible as a rare repair
    informational state instead of being hidden from the normal chat list or
    offering rejoin/rescan actions in v1.
13. Invite/profile/device-list actions are online-only in v1: unreachable
    attempts use transient feedback/diagnostics, are not queued, and do not
    mutate room or message delivery state. The core product tests now pin
    offline profile lookup without cache, invite PIN submission, unavailable-room
    stale pending-invite metadata, and device refresh/revoke as transient-only
    actions.
14. The first-user release gate requires durable text-only offline send, plus
    upload-required attachment fail-fast proof. Attachment sends fail immediately
    when upload is unreachable, without creating a sent bubble or durable outbox
    row.
15. Delivered attachment cache misses wait for explicit tap/download before
    fetch/decrypt.

## References

- `docs/engineering-style.md`
- `docs/storage-plan.md`
- `docs/adr/0007-hint-channel-abstraction.md`
- `docs/adr/0008-rust-owned-app-runtime.md`
- `docs/technical-debt-ledger.md`
- `docs/feature-audit-marmot-pika.md`
- [RMP Architecture Bible](https://github.com/rust-multiplatform/rmp/blob/master/rmp-architecture-bible.md)
