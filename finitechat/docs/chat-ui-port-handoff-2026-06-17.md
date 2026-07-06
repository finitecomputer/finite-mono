# Chat UI Port Handoff

Date: 2026-06-17
Audience: next Codex session or human continuing the FiniteChat iOS work

## Read First

Start with these files:

- `docs/engineering-style.md`
- `docs/real-state-offline-plan.md`
- `docs/storage-plan.md`
- `docs/technical-debt-ledger.md`
- `docs/adr/0007-hint-channel-abstraction.md`
- `docs/adr/0008-rust-owned-app-runtime.md`
- `docs/feature-audit-marmot-pika.md`
- `README.md`

The governing architecture is RMP: Rust owns app state, networking,
persistence, protocol decisions, retry policy, and user-visible state
derivation. Swift renders projected state and performs bounded OS capability
bridges such as file pickers, photo saving, previews, camera/QR scanning, and
audio capture.

## Current Repo State

Workspace:

```text
/Users/futurepaul/dev/finite/finite-chat-darkmatter
```

Current branch when this handoff was written:

```text
import-pika
```

Recent commits from the Pika-quality chat UI port and persistence hardening:

```text
d2bcd99 Repair stale room state on app startup
2ba8054 Keep cached chats readable in iOS app
e62e27a Move media gallery into Rust state
065c746 Add chat media gallery
7a4a960 Prove force-close chat persistence
4a38db5 Add save photo action for chat media
145b0b6 Persist iOS runtime identity across relaunch
02ec442 Port tappable reply previews
3036bbc Preserve stable chat relaunch state
b0624d2 Protect stable chat relaunch identity
ddd214c Add voice transcript captions
2f7605c Persist chat timestamps through app relaunch
8ff1658 Restore core-created chats on app relaunch
0e35907 Add live typing activity to chat UI
1bae372 Persist iOS launch config for stable relaunch
7bb11d8 Retry failed outbox messages after restart
c3a867a Prevent diagnostic launches from poisoning app persistence
eeb7aac Persist failed outbound media sends
d74f71e Isolate iOS launch automation stores
904ecbe Hide runtime diagnostics from chat UI
d12e653 Prove app relaunch persistence
ec802c7 Add Rust-backed poll chat UI
a2501df Port Pika voice message composer
9d0426a Load local chat state before launch automation
4352f6f Port Pika paste-aware composer input
a2ee04d Port Pika staged media composer
c6c7a19 Port Pika input accessory transcript behavior
65295b2 Persist local chat outbox across restarts
30f1d4d Persist Rust-owned selected room state
5833cec Persist server-backed profile cache
c0f3ae0 Persist Rust-owned unread state
```

## What Has Been Built

Rust-owned app/runtime state:

- selected room persisted in encrypted client SQLite;
- room summaries persisted and repaired on startup;
- local transcript projection persisted before startup sync;
- undelivered/failed outbound text outbox persisted;
- attachment sends use upload-required blob-reference delivery, not v1 offline
  media outbox persistence;
- server-backed/stale Nostr profile cache persisted;
- unread state persisted and clearable offline;
- raw/display timestamps persisted through relaunch;
- media gallery moved into Rust-projected selected-room state;
- verified attachment download cache and local path projection;
- room details/device list projection for settings/details views.

iOS UI:

- Home is now the default authenticated surface. It uses the real Finite mark
  as vector SwiftUI geometry, a floating glass intention composer, and compact
  suggestion chips for "Message someone" and "Chat with Agent" directly above
  the composer so those suggestions can later become type/speech-driven.
- Chats, People, Agents, and New use native SwiftUI `TabView` navigation. The
  New tab is a real fourth tab item that routes to the Home surface rather than
  a custom/fake tab bar or separate accessory.
- The app icon is now supplied by `Sources/Assets.xcassets/AppIcon.appiconset`
  and configured through XcodeGen's
  `ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon`.
- chat list rows instead of protocol controls;
- room transcript with performant collection-backed rows;
- input accessory composer behavior;
- staged multi-photo/video and file attachments;
- pasted image/GIF staging;
- replies and tappable reply previews;
- reactions, read receipts, polls/votes;
- voice recording, voice playback, and optional transcript captions;
- attachment previews and save-to-Photos bridge;
- media gallery screen;
- hidden Developer settings for raw runtime diagnostics, persistence data, and
  a bounded redacted debug log. Export requires explicit user action and is
  local copy/share only before first release, with no automatic upload or
  telemetry; the log excludes plaintext message bodies, attachment bytes,
  plaintext filenames, and plaintext media metadata.

Protocol/product flow:

- user-facing flow starts on Home with an intention composer, then routes into
  Message someone, Chat with Agent, Invite, Scan/Paste, PIN, and Chat;
- no user-facing manual sync, accept, or finalize action;
- SSE hint loop is behind Rust `wait_for_update`;
- server-backed Nostr profiles are the v1 profile source, with local cache;
- server is the first-class full-service backer; Nostr relay compatibility is
  deferred.

## Verification Already Run

Recently reported passing gates:

```text
cargo test -p finitechat-core
cargo clippy -p finitechat-core --all-targets -- -D warnings
full iOS simulator test suite: 73/73
```

Recent live simulator proof:

- app launched without injected args;
- list showed persisted room state from the stable store;
- room opened with normal composer when local state was healthy;
- hidden config showed the intended stable server/device identity.

Do not treat that as enough. The missing proof is the full real-state
online/offline product matrix in `docs/real-state-offline-plan.md`.

## Important Recent Fixes

`2ba8054 Keep cached chats readable in iOS app`:

- added room-details projection;
- made cached/degraded transcript readable;
- stopped invite failure from mutating an existing connected room into
  `UnavailableOnDevice`;
- changed chat-list rows to neutral room avatars and less misleading preview
  copy.

`d2bcd99 Repair stale room state on app startup`:

- repaired stale persisted app-room rows when local MLS exists;
- projected rooms without local MLS as read-only cached state instead of
  pretending they are active. Current target semantics supersede this: missing
  or unusable local MLS membership is `UnavailableOnDevice`, not a normal
  read-only product state;
- kept non-authoritative invite maintenance and activity/profile fetch failures
  from poisoning ordinary chat state;
- isolated iOS test config paths when tests inject `applicationSupportURL`.

## Current Risk

The product has been moving quickly and several dev/test launch paths existed
at once. That created states where a simulator could show a room row that did
not match local MLS membership or could inherit a stale server/device config.
Those states are unacceptable as product behavior and must not be normalized.

Treat any new occurrence of these as high priority:

- a connected local room becomes `UnavailableOnDevice` because the server is down;
- a known room in `UnavailableOnDevice` is hidden from the chat list instead of
  shown as a rare repair/unavailable state;
- the product harness starts from a stale connected room row with no usable
  local MLS membership instead of resetting the explicit test store or using a
  named corrupted-state repair fixture;
- a test creates impossible room state instead of using real product flows or
  an explicitly named corrupted-state repair fixture;
- test cleanup mutates room rows or partially clears local state instead of
  resetting a whole explicit test store at a scenario boundary;
- product-store cleanup uses ad hoc scripts, targeted SQL, row mutation, or
  partial config clearing instead of the single documented reset command;
- a force-close relaunch loses a saved transcript;
- a server-off launch hides rooms that were visible with the server on;
- a sendable room becomes read-only because of transport failure;
- a message send/upload rejection mutates room state or disables the composer
  while local MLS membership is still usable;
- composer availability is derived from the last server operation result
  instead of usable local MLS membership;
- an undelivered local bubble becomes a duplicate, reordered flicker, or
  visually separate row when the server accepts it;
- automatic outbox drain retries `failed` rows instead of leaving them for an
  explicit user retry or named repair flow;
- explicit retry creates a new message row or visible bubble instead of
  reusing the failed outbound row, local message id, and idempotency material;
- read receipts advance delivery, clear failure, or trigger retry instead of
  only filling delivered-message checkmarks;
- Swift-staged media appears as sent before upload/send accepts an encrypted
  blob-reference message;
- offline attachment sends create sent bubbles, `outbound_delivery`, or durable
  outbox rows instead of failing immediately with transient feedback;
- missing or corrupt local attachment staging bytes are treated as delivery
  failure or room failure instead of attachment-unavailable/composition failure;
- a delivered blob-reference attachment with missing local plaintext is treated
  as failed/undelivered or auto-downloaded instead of a cache miss that waits
  for explicit tap/download;
- attachment product proof uses `MemoryBlobStore` or a mock instead of the real
  finitechat-server `/upload` and `/blobs/{sha256}` routes;
- attachment kill/relaunch shows a sent bubble before upload/server acceptance;
- Swift shows fake upload/download percentages instead of Rust-projected real
  transfer progress or coarse in-flight activity;
- hidden Developer settings lack enough inspectable/copyable redacted debug log
  data to diagnose product-state, store, transport, or repair failures without
  logging plaintext message bodies, attachment bytes, plaintext filenames, or
  plaintext media metadata;
- debug logs are uploaded automatically or through telemetry instead of an
  explicit local copy/share action;
- the offline product matrix changes the configured server URL instead of
  stopping or unreaching the same configured server;
- queued offline messages trigger persistent room-level or global
  offline/reconnecting banners instead of staying quiet with message-level
  delivery marks;
- invite/profile/device-list actions are queued as durable offline operations
  instead of staying online-only with transient feedback in v1;
- any normal product path depends on a read-only cached room state instead of
  either keeping usable local MLS rooms enterable or marking missing local MLS
  as `UnavailableOnDevice`;
- a test or launch automation writes fake server/device config into the stable
  product app support directory;
- the normal chat UI shows raw HTTP diagnostics.

## Next Work

1. Run `git status --short` and inspect any dirty files before editing.
2. Read `docs/real-state-offline-plan.md`.
   Treat that plan as already delegating implementation guardrails to the
   engineer. Ask the user about product semantics and user-visible tradeoffs,
   not command flags or path-check details.
3. Hard-cut the Rust delivery projection into optional `outbound_delivery` with
   local send acceptance plus server delivery state before building the harness.
   Do not build the product matrix against legacy `pending`/`sent`/`failed`;
   that would bake in the wrong semantics and create churn.
   The iOS transcript should render outgoing state as bottom-right checkmarks:
   one check for local sent/undelivered, two checks for delivered, filled for
   read when at least one peer has read. Server send/upload rejection should
   project only as `outbound_delivery.failed` with a bounded reason, not as a
   room-state transition. Server acceptance should promote the local bubble in
   place with stable visible message identity. Read receipts are display-only:
   they fill delivered checkmarks but do not affect delivery, failure, retry,
   or outbox drain.
   Queued offline sends should not add persistent room-level or global
   offline/reconnecting banners; the room stays quiet except for message-level
   marks and transient feedback for online-only actions.
   Invite creation, profile refresh, and device-list refresh are online-only in
   v1. Do not add durable offline queues for them; unreachable attempts use
   transient feedback and hidden diagnostics while leaving room/message state
   alone.
   Explicit retry should be a Rust action over the existing failed outbound
   row, reusing the same visible bubble, local message id, and persisted
   idempotency material; it must not create a new message.
   Attachment sends are upload-required in v1 and are not part of the offline
   outbox. If upload is unreachable, fail immediately with transient feedback
   before creating a sent bubble, `outbound_delivery`, or durable outbox row.
   If attachment UI ships, online attachment E2E proof must use the real
   finitechat-server `/upload` and `/blobs/{sha256}` routes with durable blob
   storage. `MemoryBlobStore` remains unit-level only.
   Once an attachment is delivered as a blob reference, missing local plaintext
   is an attachment cache miss: wait for explicit tap/download, then
   fetch/decrypt from the reference, and keep the message delivered even if the
   attachment view shows unavailable. Transcript media tiles, file rows, voice
   rows, and media-gallery items must not start downloads from lifecycle hooks.
   Attachment crash proof for v1 is limited to no sent bubble before
   upload/server acceptance and restored delivered blob-reference after
   acceptance.
   Omit byte-level upload/download progress until Rust projects real transfer
   progress; Swift may render only Rust-owned coarse in-flight activity.
4. Delete `AppRoomState::Offline` from Rust, stored room state, bindings, and
   Swift UI switches; rename `NeedsAttention` to `UnavailableOnDevice` and
   audit its transitions before building the canonical product harness.
   Connected local membership should survive server
   outages, and product tests should build rooms through real create/invite/join
   flows unless they are explicitly testing corrupted-state repair. Cleanup
   should reset whole explicit test stores between scenarios, not mutate room
   state inside a scenario.
   Hard-cut normal read-only cached room behavior: usable local MLS keeps the
   room enterable and sendable with the composer available; missing or unusable
   local MLS is `UnavailableOnDevice`. If that rare repair state occurs, keep
   the known room visible with informational repair/unavailable presentation
   instead of hiding it; do not add rejoin/rescan actions in v1.
   Auth/admission/room-not-found style send errors may trigger diagnostics or
   repair work, but they must not directly demote the room while local MLS is
   usable.
   Stale pre-release/dev/test rows that claim connected without usable local
   MLS membership are corrupted fixtures. Product matrix runs should hard reset
   the explicit store rather than preserve or normalize them.
5. Build the canonical product-state E2E harness before more UI features.
   It should preserve one stable scenario account, device identity, bundle id,
   server URL, config path, app container, and client SQLite store while
   toggling only server reachability inside the online/offline matrix. Do this
   after the delivery projection and room lifecycle hard cuts so the matrix
   asserts the product model, not transitional states.
   The simulator harness should assert both Rust projection and direct
   `client_app_outbox` shape: no outbox row after online send, one durable row
   keyed by the visible offline message id after force-close while unreachable,
   no row after same-server drain, and server delivery message ids that match
   the local visible ids only after the expected delivery phase.
   The harness must never manufacture or rely on stale connected-without-MLS
   rows; those belong only to explicitly named corrupted-state repair tests.
   Wrong-server-URL behavior belongs in hidden diagnostics/misconfiguration
   tests, not the offline-send matrix.
   Add one documented reset command for scenario boundaries:
   `cargo run -p finitechat-rmp -- reset-product-store ios --scenario <scenario> --device <device>`.
   It should delete the whole explicit app-support/config/client-store root for
   that scenario/device, require both flags, refuse the default product app
   support path, reject symlinked harness/platform/scenario/device roots, and
   must not run SQL cleanup or mutate room rows.
6. Add bounded, opportunistic Rust-owned outbox drain on startup, sync/hint
   wake, and open-room if it is not already complete for the product path.
   Drain only `sent, undelivered` rows; `failed` rows require explicit retry or
   a named repair flow. Retry reuses the existing outbox row and visible bubble.
7. Prove the server-on/server-off matrix in the simulator first, then repeat
   the same matrix on a real phone before first users. The phone pass is the
   same product gate, not a looser smoke test. The phone harness entry point is

   ```sh
   cargo run -p finitechat-rmp -- product-harness ios-device \
     --scenario text-offline \
     --device <device> \
     --udid <phone-udid-or-coredevice-id> \
     --ios-development-team <team-id> \
     --server-url http://<mac-lan-ip-or-hostname>:<port>
   ```

   The configured server URL must be an origin-only `http://host:port` URL. For
   physical-phone runs the harness rejects loopback URLs and defaults the local
   server bind address to `0.0.0.0:<server-url-port>`; pass `--server-addr` only
   when overriding that bind address, and loopback bind addresses are rejected.
   The harness probes unspecified bind addresses through Mac loopback and
   refuses to run if the probe address is already reachable, so the matrix owns
   the server it later stops and restarts.
   `--dry-run` is read-only: it resolves the matrix paths and phases without
   creating harness directories, resetting stores, building, launching, or
   writing config, but it still requires a non-empty `--udid` and either
   `--ios-development-team` or `RMP_IOS_DEVELOPMENT_TEAM` for the physical
   phone gate. That team value is passed as Xcode's `DEVELOPMENT_TEAM`. On the
   current Mac, the signing certificate label contains
   `Apple Development: Paul Miller (Y392XZ3MST)`, while the generated
   entitlements and embedded profile use provisioning team identifier
   `JBLHZ83X6T`; the harness must therefore pass `JBLHZ83X6T`. The
   build/install path accepts either the Xcode hardware UDID or the CoreDevice
   identifier printed by `xcrun devicectl list devices`, then normalizes to the
   hardware UDID used by `xcodebuild`. `--no-reset` refuses to change an
   existing harness config's server URL or device id; use the whole store reset
   path for identity changes.

   It requires a valid local Xcode account/provisioning profile for that
   `DEVELOPMENT_TEAM` value. The 2026-06-17 phone build now succeeds from
   command-line `xcodebuild` with `DEVELOPMENT_TEAM=JBLHZ83X6T` and bundle id
   `computer.finite.finitechat`; the remaining release gate is the full harness
   run on the provisioned physical phone.

## Local Runbook

Start the server:

```sh
cargo run -p finitechat-server -- serve 127.0.0.1:8787 --sqlite .state/finitechat.sqlite3
```

Run the iOS app through RMP:

```sh
FINITECHAT_SERVER_URL=http://127.0.0.1:8787 cargo run -p finitechat-rmp -- run ios
```

The app and RMP runner use bundle id `computer.finite.finitechat`. Do not add a
debug bundle suffix for ordinary persistence testing.

Useful checks:

```sh
cargo test -p finitechat-core
cargo clippy -p finitechat-core --all-targets -- -D warnings
cargo run -p finitechat-rmp -- doctor
cargo run -p finitechat-rmp -- bindings swift
uvx --no-config ruff format --check .
uvx --no-config ruff check .
uvx --no-config --with hermes-agent basedpyright
python3 -m unittest discover -s tests -p '*test*.py'
```

## Friction And Debt To Keep Visible

Primary debt rows:

- `In-memory-only app runtime projections`
- `Product app state can be polluted by non-product launch paths`
- `Transport failure conflated with room lifecycle`
- `Chat transcript UI ahead of product projection commands`
- `Rebuilt selected-room media gallery`
- `Pika composer media parity is incomplete`

Deferred decisions or still-open work:

- dynamic Home suggestions backed by typed contact/agent intent projection
  instead of static buttons;
- live outbound upload progress;
- durable offline attachment send is deferred beyond v1; the product harness now
  attempts an in-memory attachment send while the same configured server is
  unreachable and asserts fail-fast behavior because upload is required;
- indexed client media-gallery table;
- revoked-device marks moved from in-memory runtime state into encrypted
  client SQLite projection;
- product-grade automatic outbox drain proof across real iOS app relaunch and
  server restart;
- byte-level download/upload progress through Rust projection. Do not fake this
  in Swift before Rust reports real transfer progress;
- physical-phone E2E against local server once the local Xcode
  account/provisioning prerequisite is satisfied, then hosted/Hermes paths;
- eventual Postgres hosted server proof;
- future Nostr relay compatibility after server-backed profiles/messages are
  stable.

## Definition Of Done For The Next Goal

The next goal should be considered done only when:

- a single documented product-state harness exists;
- the app passes the online/offline matrix in `docs/real-state-offline-plan.md`
  simulator-first, then repeats the same matrix on a physical phone before
  first users, while preserving the same configured server URL and toggling
  only reachability;
- the current simulator proof passed on 2026-06-17 with
  `--device codex-sim-v1 --server-url http://127.0.0.1:18987`: the offline text
  row survived force close as `sent/undelivered`, the offline attachment attempt
  added no bubble/outbox row, same-URL restart drained the outbox, and the peer
  saw the offline message exactly once as inbound state;
- text offline send is durable through force close and drains exactly once when
  the server returns;
- explicit retry of a failed send preserves the same visible bubble/local
  message identity and does not duplicate the transcript; iOS shows an explicit
  retry affordance on each failed local outbound bubble, including non-last
  bubbles inside grouped sender runs, and the Swift model dispatches
  `RetryMessage` only for local outbound messages whose Rust-projected server
  delivery state is `failed`;
- offline attachment send fails immediately with transient feedback because
  upload is required, with product-harness proof that no sent bubble or durable
  outbox row appears; first users are not blocked on durable attachment offline
  send;
- normal UI copy treats server outages as connectivity, not room failure;
- normal chat UI stays quiet for queued offline sends, using message-level
  delivery marks rather than persistent offline/reconnecting banners;
- composer/send availability follows Rust-owned room state: connected rooms can
  still dispatch text sends while runtime status is offline, while the rare
  `UnavailableOnDevice` repair state keeps cached messages visible but does not
  dispatch text, poll, attachment, invite creation, invite PIN submission, or
  room retry actions; the Swift model no longer exposes a room retry wrapper,
  and Rust guards `CreateInvite`, `SubmitInvitePin`, startup ticks, and stale
  pending-invite metadata so unavailable/corrupted local state stays
  transient/inert and informational;
- invite/profile/device-list actions stay online-only in v1, with transient
  feedback when unreachable and no durable offline queue; core tests now cover
  offline profile lookup without cache, invite PIN submission, and
  device-list/revoke failure as transient-only product state;
- iOS launch no longer recovers or migrates pre-release
  `FiniteChat/<device>` app-support stores; those directories are ignored
  reset-only state, with `RuntimeConfigTests` pinning the hard cut;
- core launch no longer imports pre-release `app-messages.json`, and client
  store open rejects legacy app projection tables rather than rewriting them;
  app projection timestamp columns with old `DEFAULT 0` schema defaults fail
  closed as reset-only state, and extra app projection columns are unsupported;
  legacy unencrypted client-store tables fail closed even when empty instead of
  being dropped as compatibility cleanup; encrypted room metadata missing
  current lifecycle fields or carrying old `Offline` / `NeedsAttention` values
  also fails closed instead of becoming a connected room; encrypted outbox
  metadata missing timestamps or carrying old one-axis `delivery_state` payloads
  fails closed instead of becoming v1 outbound delivery; encrypted
  app-state/profile metadata missing selected-room, revoked-device, or
  stale-profile fields fails closed instead of being defaulted into product
  state;
- `UnavailableOnDevice` remains visible as a rare repair/unavailable room
  informational state rather than being hidden from the normal chat list or
  offering rejoin/rescan actions in v1;
- delivered attachment cache misses wait for explicit tap/download rather than
  auto-fetching; transcript, voice, file, and gallery UI now dispatch downloads
  only from tap/open actions, and the Swift app model refuses direct
  `DownloadAttachment` dispatch for local, missing-reference, uploading, or
  already-downloading attachment projections;
- hidden Developer settings retain enough diagnostics to debug bad stores
  without showing them to users, including explicit local copy/share export of
  a bounded redacted debug log, no automatic upload or telemetry, and no
  plaintext message bodies, attachment bytes, plaintext filenames, or plaintext
  media metadata;
- the debt ledger has smaller delete conditions or removed rows for every
  completed piece.
