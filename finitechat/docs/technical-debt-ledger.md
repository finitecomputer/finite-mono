# Technical Debt Ledger

Status: active ledger.

This file is where tolerated integration debt goes before it becomes product
architecture by accident. A debt item is allowed only when it has:

- an observed source;
- why the shortcut is risky;
- the first proof that keeps it bounded;
- a delete condition.

Do not call something "temporary" without a delete condition. Do not add a new
finitecomputer integration shortcut without adding or updating a row here.
Before the first user release, debt rows are not compatibility commitments:
prefer hard cuts, delete obsolete shapes, and reset dev/test state instead of
adding migrations or legacy shims for pre-release data.

## Finitecomputer Integration Debt

| Debt | Observed Source | Why It Is Risky | First Proof | Delete Condition |
| --- | --- | --- | --- | --- |
| Plaintext relay and mirrored chat snapshots | `finited` exposes chat snapshot and SSE stream endpoints; `finitecomputer` docs call this Half-Moved | The snapshot can quietly become canonical chat state instead of a transition bridge | Finite Chat durable room-log store can replay the same dashboard DTOs from ordered encrypted events | Dashboard reads projection from durable Finite Chat room events; snapshot mirror is removed or becomes a derived cache with named source/invalidation |
| File-backed relay commands | `finited` relay currently stores events/results outside the Finite Chat room sequencer | Command/result delivery can drift from room ordering, idempotency, and runtime command ledger rules | Runtime command request/result DTOs are tested through the Finite Chat reducer/store before finitecomputer integration | Runtime commands that affect chat/runtime state are ordered durable room events; relay callbacks only wake sync or carry hosted-runner admin events |
| Plaintext `ChatRuntime` as canonical transcript | `finite-core::ChatRuntime` stores threads, messages, gateway inbox, and attachments in local SQLite | New protocol semantics can get bolted onto the old transcript schema, creating two chat models | Encrypted room projection can render the existing dashboard DTO contract without mutating plaintext `messages` | Plaintext `ChatRuntime` is read-only archive/import input or deleted for canary Projects |
| Stringly relay lanes and kinds | Dashboard relay calls use `{ lane, kind, payload }` strings such as `chat.send_message` and `runtime.inference.apply` | Command names and payloads can spread without typed validation or idempotency semantics | `finitechat-proto` owns typed command, topic, state, and message DTOs with invalid-data tests | Dashboard/server routes translate into typed Finite Chat events before runtime scheduling; untyped relay names are not the application protocol |
| Runtime-local attachment bytes | Current finitecomputer chat attachments are stored under the runtime and can travel through relay JSON/base64 paths | Attachments are not portable across clients, and storage policy is mixed into chat transport | `finitechat-blob` proves encrypted Blossom-compatible references, ciphertext upload verification, ciphertext-before-decrypt checks, plaintext-after-decrypt checks, v1 size rejection, metadata hiding from the blob store, and Rust-owned verified plaintext cache projection for native clients | finitecomputer chat messages carry encrypted blob references; dashboard/runtime no longer fetch plaintext attachment bytes from runtime-specific stores |
| Dashboard status as request/response | Existing dashboard surfaces use runtime commands/status calls for some state | Page loads can become durable command spam and make "read status" look like "ask runtime to work" | `runtime.state.snapshot` projection tests prove non-notifying latest-state reads | Dashboard status cards read projected snapshots by state key; explicit refresh is a user command |
| Chat control coupled to Hermes health | Current finitecomputer chat loop routes through `ChatRuntime` and Hermes gateway behavior for ordinary replies | If Hermes breaks, users can lose the only practical control surface for observing and repairing the runtime | HTTP route and runtime-delivery tests prove durable opaque command/state delivery, command-inbox effects, liveness separation, and non-notifying snapshot reads without Hermes; full daemon crash-recovery proofs return when a production daemon entrypoint exists (the old `finitechat-sim` daemon-survival suite was retired with the fake delivery service) | Finite Chat daemon remains usable for status and recovery while host is online, even when Hermes and inference are down |
| Hosted-runner admin mixed with portable commands | finitecomputer still has hosted operations for routes, auth policy, runner images, and emergency pod work | Portable Finite Chat can accidentally depend on Finite Computer's hosting substrate | Integration docs classify each surface as portable finitec command, runtime state snapshot, or hosted-runner admin | A self-managed agent with only `finitec` can use chat, topics, commands, attachments, and state snapshots without hosted-runner APIs |
| Trusted-server hosted web decryption | Hosted finitecomputer web may use a server-side Rust Finite Chat client to decrypt and render DTOs | Product copy could imply E2EE where the hosted server has device secrets | Docs and UI copy call hosted mode "web chat" or "topics", not E2EE. finitecomputer account summaries expose `ProductTrustDisclosureV1` for `hosted_web_bridge`, with `may_claim_e2ee = false`, as a typed product contract. | E2EE language is used only for clients that keep Finite Chat device secrets on the user's device |
| Premature separate `finitechatd` process | A standalone daemon is the long-term product shape, but the first canary can embed Rust crates | A new process adds auth, deployment, logging, upgrade, and local-debug burden before protocol fit is proven | Embedded crates expose a daemon-shaped API and pass local-loop restart tests | Extract `finitechatd` only after the embedded boundary has stable DTOs, sync ticks, command ledger, and storage layout |
| Shared id validators permit empty strings | `finitechat-proto::validate_room_id` currently checks byte length but not non-empty | Integration crates may assume `validate_room_id` rejects empty IDs and silently accept impossible room state | `finitechat-hermes` adds its own non-empty ingress check and has an invalid empty-room test | Tighten proto ID validators or add explicit non-empty ID helpers, then remove duplicate bridge checks |
| Verbose v1 invite URI on human copy/paste surfaces | `InviteCodeV1::encode` percent-encodes the full room-server URL plus the room id, invite id, 32-byte invite token, and 64-hex inviter account id, so the pairing URLs served by the hosted agent `/invite` endpoint and `hermes invite` run to several hundred characters | Long URLs get truncated or mangled on copy/paste, SMS, and manual-entry surfaces, pushing users toward QR-only pairing and tempting ad hoc URL shorteners that would add an accidental trust surface in front of invite tokens | v1 stays the only invite encoding: hosted and CLI surfaces serve the URI verbatim (plus a QR render), and no shortener or lossy re-encoding sits in front of it | A compact invite encoding lands in `finitechat-proto` — default-server elision and/or a packed v2 form of the binary fields — with encode/parse and cross-version tests, and invite-serving surfaces emit it; this row then deletes |
| Echo-handler Hermes harness overclaimed as real Hermes | The CLI, simulator, and device Hermes media E2E tests load the adapter path but install `set_message_handler` callbacks that send `agent text echo` / `agent media echo` replies; the local simulator demo used `.state/hermes-demo/demo_agent.py` and replied with `Hermes local demo reply: ...` | Echo replies can pass transport/media tests while hiding that real Hermes gateway/model behavior is unproven or broken | `docs/oops-i-faked-it-audit.md` labels each echo path; `scripts/hermes-real-gateway-demo.sh` runs `hermes gateway run` without a test handler; the app projection now surfaces Hermes `working`/`thinking` activity as a live indicator | Simulator and then phone product harnesses show a non-echo back-and-forth conversation with real Hermes through the finitechat plugin, including media, and docs/tests no longer present echo-handler coverage as real Hermes behavior |

## Client Projection Debt

| Debt | Observed Source | Why It Is Risky | First Proof | Delete Condition |
| --- | --- | --- | --- | --- |
| Product app state can be polluted by non-product launch paths | RMP, Xcode, Home Screen, phone, launch automation, and unit tests have historically used different combinations of bundle id, server URL, device id, app support directory, config storage path, and transient data root; stale simulator rows showed room state that did not match local MLS membership, and tests once risked writing fake server/device config into a stable app support path | A malformed dev state can masquerade as a protocol/client failure, making it impossible to tell whether `UnavailableOnDevice`, missing transcripts, or read-only rooms are real product bugs or test pollution; this undermines confidence in the core protocol | `RuntimeConfigTests` cover stable relaunch identity and explicit transient diagnostics; `AppModelPersistenceTests/testInjectedApplicationSupportKeepsRuntimeIdentityConfigLocal` keeps injected support paths from falling back to the real config; `d2bcd99` repairs stale persisted room rows when local MLS exists and historically kept rooms without local MLS read-only instead of pretending they are active; target semantics now hard-cut normal read-only cached room behavior so missing or unusable local MLS is `UnavailableOnDevice`; if this rare repair state occurs the known room stays visible as informational state instead of being hidden or offering rejoin/rescan in v1; stale pre-release rows that claim connected without usable local MLS are corrupted fixtures to hard reset or cover only in named repair tests; delivery projection must hard-cut to `outbound_delivery` and room lifecycle must hard-cut away `Offline`/`NeedsAttention` before the product matrix so tests do not preserve legacy `pending`/`sent`/`failed` or room-state semantics; `docs/real-state-offline-plan.md` defines the required product matrix | One canonical product-state harness exists for simulator-first runs and the same-matrix physical phone gate before first users; every test uses an isolated app support/config path or explicit transient store; online/offline E2E toggles only reachability of the same configured server URL while preserving the same scenario account, device identity, bundle id, config path, app container, and client SQLite path; wrong-server-URL behavior is diagnostics/misconfiguration coverage, not the offline product matrix; product tests build rooms through real create/invite/join/open flows except for explicitly named corrupted-state repair fixtures; cleanup uses one documented reset command that deletes whole explicit test stores at scenario boundaries, requires scenario/device identity, refuses the default product app support path, and avoids mutating room rows, running targeted SQL, or partially clearing state; hidden Developer settings expose the active store/config and redacted debug logs through explicit local copy/share export only, with no automatic upload before first release and no plaintext message bodies, attachment bytes, plaintext filenames, or plaintext media metadata |
| Transport failure conflated with room lifecycle | Invite creation failure used to mutate an existing connected `AppRoomSummary` to `NeedsAttention`/`UnavailableOnDevice`, and the chat list rendered every connected row with a green online-style status dot and `Connected` subtitle when no message preview existed | A transient server outage, wrong local server URL, failed invite action, or message send rejection can make an otherwise readable local chat look broken; users learn that every chat is online/offline instead of trusting local-first messaging semantics | `app_invite_failure_keeps_existing_chat_readable` proves a stale/wrong server failure leaves the existing room connected and durable across reopen; the iOS room row now renders a neutral room avatar for connected chats, uses `No messages yet` instead of `Connected`, and reserves colored state badges for non-connected admission/repair states | Runtime connectivity is projected separately from room lifecycle, with send/invite failures attached to the failed action/outbox and hidden Developer diagnostics; invite/profile/device-list actions remain online-only in v1 with transient feedback and no durable offline queue; composer availability follows usable local MLS membership rather than the most recent server operation result; queued offline sends keep the normal room UI quiet except for message-level delivery marks, with no persistent room-level/global offline or reconnecting banner; hidden Developer diagnostics include bounded redacted copy/export debug logs for runtime, transport, persistence, and repair events, excluding plaintext message bodies, attachment bytes, plaintext filenames, and plaintext media metadata; debug-log export is explicit local copy/share only before first release, not automatic upload or telemetry; wrong-server-URL behavior stays diagnostics/misconfiguration coverage while the product offline path toggles reachability of the same configured URL; auth/admission/room-not-found style send failures project as failed outbound messages plus diagnostics/repair hooks, not direct room-state transitions; automatic outbox drain excludes failed rows, which require explicit user retry or a named repair flow over the same outbox row, visible message identity, and idempotency material; chat list rows never imply per-room online/offline status for ordinary connected rooms |
| Chat transcript UI ahead of product projection commands | The iOS transcript renders projected reply ids, media references, outbound delivery state, read receipts, verified local attachment paths, Rust-owned attachment download activity, and bounded load-older windows, but byte-level download/upload progress is still unavailable behind the current blocking HTTP boundary | UI affordances can drift into dead controls, duplicate local/server bubbles, retry-created duplicate messages, read-state-driven delivery changes, sent-looking staged media, offline attachment queues that v1 does not support, attachment-cache corruption mislabeled as delivery failure, delivered attachment cache misses mislabeled as failed sends, fake Swift transfer progress, mock-only blob proof, media crash/relaunch gaps, or local-only behavior if Swift starts inventing state outside the room log | The first Pika-style transcript slices are protocol-backed for reactions, receipts, outbound attachments, attachment downloads, attachment open/preview, save-to-Photos, and transcript windowing: `ChatTranscriptView` is collection-backed, uses stable row ids, reconfigures visible rows for same-id message content changes, owns keyboard/accessory scroll geometry through a UIKit input accessory host, `AppState.messages` is the selected room's bounded Rust-owned window, `LoadOlderMessages` expands that window using the current oldest visible message as an anchor, reactions go through durable Rust `chat.reaction` actions, read receipts go through durable non-notifying Rust `chat.receipt` actions, outbound status should render as bottom-right checkmarks over Rust-projected `outbound_delivery` and read receipts, with filled checks when `read_count > 0` only after delivery, accepted sends should promote the undelivered local bubble in place with stable visible message identity, explicit retry should reuse the existing failed outbox row, local message id, visible bubble, and idempotency material, v1 attachment sends are upload-required and fail immediately when upload is unreachable before creating a sent bubble, `outbound_delivery`, or durable outbox row, online attachment sends go through Rust-owned encrypt/upload/send projection using `finitechat-blob` plus server-backed `/upload` and `/blobs/{sha256}` ciphertext persistence, product E2E uses the real finitechat-server blob routes and durable blob storage rather than `MemoryBlobStore` or mocks if attachment UI ships in v1, product E2E proves offline attachment fail-fast and restored delivered blob-reference messages after acceptance, missing/corrupt local attachment staging projects attachment-unavailable/composition failure instead of delivery failure or room-state changes, delivered blob-reference attachments with missing plaintext project cache-miss state and wait for explicit tap/download while the message remains delivered, `BeginDownloadAttachment` and `DownloadAttachment` project Rust-owned in-flight download activity after explicit user action, `DownloadAttachment` verifies ciphertext/plaintext before projecting a local cache path to Swift, Swift image/video/file previews and PhotoKit saves are OS presentation/capability bridges over that verified local path, and `AppModelPersistenceTests/testRawRuntimeDiagnosticsStayOutOfNormalChatSurfaces` proves raw transport diagnostics stay in the hidden Developer settings instead of the normal chat/empty-state surfaces | Blob upload/download transport reports byte progress through Rust projection; Swift controls remain thin calls into Rust actions or OS preview bridges over Rust-projected paths and may show only coarse Rust-owned in-flight activity until real byte progress exists; read receipts remain display-only and never affect delivery, failure, retry, or outbox drain; local-to-server send promotion and explicit retry preserve stable visible row identity without duplicates or flicker; staged media cannot appear sent before upload/server acceptance; offline attachment sends fail immediately without sent bubbles or durable outbox rows; delivered attachment cache misses never change message delivery or room state and wait for explicit tap/download; product attachment E2E passes through server-backed blob routes instead of mock-only storage when attachment UI ships, and the debt row is removed |
| Swift-owned People relay/profile cache | `ios/Sources/NostrPeople.swift` fetches Nostr kind 3 contact lists and kind 0 profiles directly, persists rows through `NostrPeopleCache`, and uses `CachedRemoteImage` as a Swift memory cache for profile/avatar image redraws | The native app can drift into owning relay policy, invalidation, avatar caching, and profile truth outside the Rust client store that should eventually back chat, sites, and brain identity surfaces | `NostrPeopleTests` prove cached follows render immediately, a background relay refresh replaces stale rows, invite availability survives cache writes, and relay failure keeps cached people visible with transient refresh status; full iOS unit tests compile the shared cached-avatar view path | Rust projects follows, profiles, durable avatar-cache file URLs, and stale-while-revalidate state from the client store; Swift renders that projection and this direct relay/cache path is deleted or reduced to a thin Rust call |
| Rebuilt selected-room media gallery | `AppState.media_gallery` is Rust-owned and all-history for the selected room, but it is rebuilt from the retained encrypted app-message projection rather than an indexed media table like Pika's media DB | Very media-heavy rooms may pay repeated attachment-cache verification work when selected-room state changes; the projection is still capped by `MAX_APP_MESSAGES` until a dedicated media index lands | `app_runtime_media_gallery_is_all_history_and_downloads_outside_transcript_window` proves older remote media outside the selected transcript window remains visible, downloadable, and durable across offline reopen; Swift renders `AppState.mediaGallery` directly with stable item ids and Rust `DownloadAttachment` actions | Client SQLite has an indexed media-gallery table/projection with source-of-truth invalidation from room-log app messages and attachment cache writes, so selected-room gallery state no longer scans retained app messages |
| Pika composer media parity is incomplete | Pika's iOS composer supports staged multi-image media, pasted images, file picking, polls, and voice recording with optional local speech transcript captions; FiniteChat now has text, replies, pasted image/GIF staging, multi-photo/video staging, multi-file staging, Rust-backed polls/votes, voice recording/playback through `.voiceNote` attachments, native speech recognition that sends the transcript as the Rust-owned voice-message caption, captioned batch send through Rust `sendAttachments`, and remove-before-send, but still lacks live incremental outbound upload progress | Porting the remaining controls as local Swift state without Rust-owned outbound attachment/progress projection would create sends the protocol cannot recover or explain after restart; v1 must also avoid pretending attachments are offline-queueable when upload is required | Rust exposes `sendAttachments`, `sendPoll`, `votePoll`, and `RetryMessage`; poll creation is a typed `chat.message` payload and poll voting is a durable non-notifying `chat.poll.vote.v1` namespaced event; `app_runtime_polls_are_durable_and_votes_are_non_notifying` proves vote projection survives offline reopen; the iOS transcript can render multi-attachment messages, inline voice notes, and poll option tallies; the composer lives in the transcript input accessory; staged file, paste, voice attachment, voice transcript caption, and poll projection tests prove protocol-backed dispatch before UI state updates; offline attachment send must fail before a sent bubble/outbox row when upload is unreachable; online attachment send promotes only after encrypted upload and accepted blob-reference send; `hermes_cli_round_trips_media_blob_references_with_app_runtime` proves human-to-agent and agent-to-human Hermes media events carry encrypted blob references without leaking typed wrapper JSON into chat text | Live outbound upload progress survives force-close through Rust-projected real transfer progress, not Swift-synthesized timers; offline attachment sends fail fast because upload is required |
| iOS and product-harness identity not yet on the shared Finite identity | CLI/agent account keys hard-cut to `finite-identity` (`$FINITE_HOME/identity/identity.json`), and core no longer writes or reads per-store `account-secret.hex`; but iOS still holds the account secret in its keychain and passes it explicitly, `AppModel.hasRecoverableStableStore` still probes the now-never-written `account-secret.hex` (store-level recovery is inert for new stores), and `finitechat-rmp` `product_harness` opens owner/peer stores with `account_secret_hex: None`, which now resolves one shared identity for both roles instead of the per-store secrets the profile-dm scenario assumes | The simulator/product-harness scenarios can silently degrade (same account for owner and peer, or undecryptable app-created stores), and iOS retains a secret copy outside the shared location, re-opening per-tool identity drift on the one platform the contract cannot reach via `~/.finite` | `crates/finitechat-core/tests/shared_identity.rs` pins fresh-start mint, pre-existing pickup, and the absence of `account-secret.hex`; pinned HKDF vectors in `finitechat-mls`/`finitechat-client` prove derivation math is unchanged for a given secret | iOS harness launches provide an explicit per-role identity (launch-arg secret or a FINITE_HOME-equivalent inside the app container), product-harness peer flows pass explicit per-role secrets, the dead `hasRecoverableStableStore` probe is removed, and this row deletes |

## 2026-06-17 Hard-Cut Progress

Landed before and during the first product-state harness slice:

- Rust projection now exposes optional `outbound_delivery` only for local
  outbound messages. Inbound messages do not carry delivery state.
- Text sends persist the prepared append request/idempotency material in the
  local outbox before transport. No-response delivery leaves the bubble
  locally sent and undelivered; accepted delivery promotes the same visible
  message id in place; failed rows are excluded from automatic drain.
- iOS renders outbound delivery from Rust-owned state as bubble checkmarks and
  keeps normal offline queued-send UI quiet.
- Attachment sends no longer create durable failed media outbox rows when upload
  is unavailable; they fail with transient feedback before a sent bubble exists.
  `app_offline_attachment_send_fails_fast_without_outbox_or_bubble` now asserts
  the room stays connected, no visible attachment bubble appears, and the Rust
  outbox is empty immediately and after force-close reopen.
- `app_runtime_downloads_attachment_blob_to_verified_local_cache` now proves a
  delivered blob-reference attachment cache miss remains delivered and
  unfetched across force-close and same-server reopen; only explicit
  `BeginDownloadAttachment`/`DownloadAttachment` projects in-flight state and
  writes the verified plaintext cache path.
- `app_profile_scan_offline_without_cache_is_transient_only`,
  `app_invite_pin_offline_is_transient_and_keeps_scanned_invite`, and
  `app_device_actions_offline_are_transient_only` now prove online-only
  profile, invite PIN, and device-list/revoke failures stay transient, do not
  create app outbox work, and do not mutate room/message delivery state.
- iOS `RuntimeConfig` and `RuntimeDataStore` no longer recover or migrate
  pre-release `FiniteChat/<device>` app-support stores. The updated
  `RuntimeConfigTests` assert legacy directories are ignored and stable launch
  uses only `FiniteChatStore` or explicit `FiniteChatTransient/<device>` roots.
- Core startup no longer imports pre-release `app-messages.json`, and
  `SqliteClientStore::open` no longer migrates legacy app projection schemas.
  Old `client_app_messages`/`client_app_events` tables with plaintext rows or
  missing timestamp/nonce/ciphertext columns are reset-only and fail closed at
  open; app projection timestamp columns with old `DEFAULT 0` schema defaults
  are reset-only too, and extra columns on encrypted app projection tables are
  unsupported. Legacy unencrypted client-store tables such as
  `client_openmls_storage`, `client_rooms`, and `client_profiles` are also
  reset-only even when empty; store open no longer drops them as cleanup.
  Encrypted app-room metadata must carry the current room lifecycle shape:
  missing `state`/`status`/`local_read_seq` fields or legacy `Offline` /
  `NeedsAttention` enum payloads fail closed instead of defaulting to a
  connected room. Encrypted app-outbox metadata must carry the current
  `local_state` plus `server_delivery_state` shape and timestamp; old
  one-axis `delivery_state` payloads fail closed instead of being promoted into
  v1 outbound delivery rows. Encrypted app-state/profile metadata must also
  carry the current selected-room, revoked-device, and stale-profile fields;
  missing fields fail closed rather than being defaulted into product state.
- `Offline` room lifecycle state was removed and `NeedsAttention` was renamed
  to `UnavailableOnDevice`; the v1 repair surface is informational only.
- `finitechat-rmp reset-product-store ios --scenario <scenario> --device
  <device>` now deletes only the whole explicit harness root under
  `.state/product-harness/ios/<scenario>/<device>` and refuses unsafe path
  components, symlinked harness/platform/scenario/device store roots, harness
  roots outside the workspace, or resolved targets outside that root. The reset
  guardrail tests now cover symlinked `.state`, product-harness, platform,
  scenario, and device roots on Unix.
- Product-harness `--dry-run` now uses read-only path resolution: it prints the
  canonical harness paths/phases without creating `.state`, resetting stores,
  building, launching, or writing config, while still applying the same path and
  symlink guardrails.
- Product-harness `--no-reset` now fails closed if an existing harness config's
  server URL or device id differs from the requested run. Changing either
  identity requires the documented whole-store reset path instead of silently
  overwriting config inside a reused store.
- iOS accepts `--finitechat-product-harness-root <path>` for harness launches,
  loads config from that explicit support root, writes `FiniteChatStore` there,
  and fails closed instead of falling back to the default app store when the
  argument is malformed.
- `finitechat-rmp product-harness ios-simulator --scenario text-offline
  --device <device> --server-url <url>` now owns the simulator text-offline
  phases: reset the explicit root, write config, install without first opening
  the default app store, launch online create/send, stop the server, launch
  offline send against the same configured URL, restart the same URL, and launch
  for bounded outbox drain. The command logs config, store, server SQLite, and
  server log paths. A local run on 2026-06-17 with `--device codex-sim-v1`
  against `http://127.0.0.1:18987` produced the explicit harness store and
  asserted the server-side chat delivery shape: one application-delivery effect
  after the online phase, still one after the offline phase and offline
  attachment attempt while the server was stopped, and two after same-URL
  restart/drain. Protocol publish/idempotency rows are allowed to include
  peer-admission traffic; the exact chat proof is the delivery-effect count, one
  delivered room, sender device `codex-sim-v1`, and server delivery message ids
  that match the local visible message ids only in the phases where delivery
  should have happened.
  The same run terminates the simulator app between phases and opens the same
  explicit `FiniteChatStore` through the Rust runtime to assert local
  projection state: one delivered local outbound message after online send, two
  local outbound messages after offline force-close with exactly one
  undelivered id and the online delivered id still present, and two delivered
  local outbound messages after same-URL restart with both the original online
  id and that same formerly-undelivered message id present in place. The harness
  now also reads the explicit client SQLite store directly to assert
  `client_app_outbox` has zero rows after online send, exactly one row keyed by
  the visible offline message id after force-close while unreachable, and zero
  rows after same-URL drain. For the offline row it also reads the encrypted
  metadata through Rust and requires local state `sent`, server delivery state
  `undelivered`, append-request message id matching the visible bubble, and
  retained idempotency material. The harness outbox assertion helper now rejects
  underspecified expectations: non-empty phases must declare room identity plus
  local/server delivery states, wrong-room rows are rejected, and empty phases
  must not carry stale row identity or state criteria. Before restarting the
  server it launches the app
  with a synthetic in-memory attachment send and asserts upload-required
  fail-fast behavior: no new visible outbound bubble, no additional durable
  outbox row, and no new server delivery effect. It also asserts the offline
  visible message id is absent from the server delivery log while the server is
  stopped and present after same-URL drain. iOS transcript media tiles, file
  rows, voice rows, and media-gallery items no longer start delivered
  attachment cache-miss downloads from lifecycle hooks; downloads dispatch only
  from explicit tap/open actions, and Rust `0` transfer progress renders as
  coarse in-flight activity rather than a fake percentage. The harness also
  creates a peer through the real invite/PIN/admission flow before the offline
  phase and asserts the
  restarted-server drain reaches that peer exactly once as an inbound message
  with no `outbound_delivery`; the `codex-sim-v1` run asserted that peer receipt
  for the formerly-undelivered visible message id.
- iOS outbound delivery marks now expose stable accessibility descriptors and
  identifiers derived from Rust-projected `outbound_delivery`: one-check
  `sent-undelivered`, two-check `delivered-unread`, filled two-check
  `delivered-read`, and `failed`. The simulator transcript was launched from
  the product harness store on 2026-06-17 and visually captured with the two
  delivered outbound bubbles showing bottom-right double checks and no
  offline/reconnecting banner. A follow-up XcodeBuildMCP `snapshot_ui` pass
  found the two product-harness message bubbles by stable `ChatMessageBubble-*`
  accessibility identifiers, each with `AXValue = "two checks"`, and no normal
  offline/reconnecting banner element. `OutboundDeliveryAccessibilityTests`
  now include repeatable simulator CI assertions for the combined
  `ChatMessageBubble-*` accessibility label/value over undelivered, delivered,
  read, failed, and inbound messages. A repeat XcodeBuildMCP `snapshot_ui`
  spot-check after that change still found the two product-harness transcript
  bubbles with `AXValue = "two checks"` and no normal offline/reconnecting
  banner element. `AppModelPersistenceTests` now also prove queued offline text
  state does not request a normal-surface `NoticeBar`, while non-empty notice
  text still maps to a stable `NoticeBar` accessibility identifier.
  `testConnectedSavedRoomCanSendWhileRuntimeStatusIsOffline` proves a connected
  Rust-owned room can still dispatch `SendMessage` while runtime status is
  offline, and `testUnavailableSavedRoomKeepsCachedMessagesButCannotSend`
  proves the model does not dispatch text, poll, attachment, invite creation,
  or invite PIN submission from the rare `UnavailableOnDevice` repair state,
  and the Swift model no longer exposes a room retry wrapper. The
  Rust `app_corrupted_state_unavailable_room_create_invite_is_transient_only`
  test also proves `CreateInvite`, `SubmitInvitePin`, startup ticks, and stale
  pending-invite metadata do not leave the informational repair state or create
  durable side effects when local MLS is missing.
  `testProductHarnessDeliveredTranscriptPresentationHasNoNormalOfflineBanner`
  adds the committed transcript assertion over the product-harness delivered
  state: two selected transcript bubbles, both with `AXValue = "two checks"`,
  and no normal `NoticeBar` even while runtime status is offline.
  `testDownloadAttachmentDispatchesOnlyForExplicitCacheMissCandidate` proves
  the Swift app model refuses direct download dispatch for local,
  missing-reference, uploading, or already-downloading attachment projections,
  keeping delivered cache-miss downloads explicit at the action boundary.
  `OutboundDeliveryAccessibilityTests` also assert the explicit retry
  affordance is visible for every failed local outbound bubble and absent for
  inbound or merely undelivered messages.
  `testRetryMessageDispatchesOnlyForFailedLocalOutbound` proves the Swift model
  also refuses to dispatch `RetryMessage` for inbound, delivered, or merely
  undelivered messages, and dispatches it only for a failed local outbound
  projection.
- `finitechat-rmp product-harness ios-device --scenario text-offline
  --device <device> --udid <phone-udid-or-coredevice-id> --ios-development-team <team-id>
  --server-url http://<mac-lan-ip-or-hostname>:<port>` now owns the
  physical-phone version of the same matrix. It rejects loopback configured
  server URLs, requires origin-only `http://host:port` configured URLs, defaults
  the local bind address to `0.0.0.0:<server-url-port>`, rejects loopback bind
  address overrides, maps unspecified bind readiness probes to Mac loopback,
  refuses to run against an already reachable server probe address, builds the
  `aarch64-apple-ios` Rust/XCFramework
  slice, resets the phone by uninstalling the app for non-`--no-reset` runs,
  force-closes through `devicectl`, pulls the phone's
  `Library/Application Support/FiniteChatStore` into the explicit harness root
  for the same Rust projection assertions, and pushes the host-side
  peer-admission store update back before the offline phase. The pull path now
  accepts either a nested `FiniteChatStore` directory or direct store-root
  contents with `account-secret.hex` and `client.sqlite3`, and rejects ambiguous
  `devicectl` output instead of guessing. Dry-run and build/install paths now
  fail before Xcode if the physical-phone identifier/UDID or signing team is
  missing; build/install accepts either the Xcode hardware UDID or the
  CoreDevice identifier printed by `xcrun devicectl list devices`, then
  normalizes to the hardware UDID used by `xcodebuild`. The team may come from
  `--ios-development-team` or `RMP_IOS_DEVELOPMENT_TEAM` and is passed as
  Xcode's `DEVELOPMENT_TEAM`; on the current Mac the signing certificate label
  contains `Apple Development: Paul Miller (Y392XZ3MST)`, while the generated
  entitlements and embedded profile use provisioning team identifier
  `JBLHZ83X6T`, so the harness must pass `JBLHZ83X6T`. A 2026-06-17 run
  normalized the attached phone's CoreDevice identifier to hardware UDID
  `00008150-0010149A26F0401C` and proved command-line `xcodebuild` succeeds for
  bundle id `computer.finite.finitechat` with `DEVELOPMENT_TEAM=JBLHZ83X6T`.
  `finitechat-rmp` unit tests now exercise the physical-phone dry-run and
  identifier-normalization command paths directly, proving that good dry-runs do
  not create `.state`, `RMP_IOS_DEVELOPMENT_TEAM` is accepted as the signing-team
  source, CoreDevice identifiers normalize to hardware UDIDs, and loopback URLs,
  loopback binds, missing UDIDs, and missing signing teams fail before
  build/install work.
- Hidden iOS Developer diagnostics now keep a bounded in-memory redacted log
  for runtime, transport, persistence, and repair events. The normal chat UI
  does not render it; the Developer disclosure exposes explicit local Copy Logs
  and Share Logs actions only. Tests prove the export redacts URLs, filesystem
  paths, long hex material, and does not include message text from send actions.
- `app_server_rejected_text_send_requires_explicit_retry_with_same_outbox_identity`
  opens a real local room against an empty live server to get an actual HTTP
  non-success response, then proves the failed outbound row survives force close,
  is excluded from automatic drain, and explicit retry on the original server
  reuses the same visible message id, append request, and idempotency material.
- `scripts/hermes-agent-media-e2e.sh` now runs the real upstream
  `hermes-agent` package with the Finite Chat plugin against a live
  `finitechat-server`, pairs through invite URL plus PIN, sends image media from
  the user side, and requires the user to receive both agent text and image
  media replies. The Rust Hermes regression also covers back-to-back text and
  media sends from separate bridge processes.
- `scripts/ios-hermes-agent-media-e2e.sh` now runs the same real Hermes media
  path through the iOS Simulator app. The app joins the agent invite, sends an
  image attachment with a caption using Rust `sendAttachments`, the real Hermes
  adapter receives it as photo/image media, the agent replies with text plus
  image media, and the harness verifies those replies in the simulator app's
  persisted Rust-projected state.
- `scripts/ios-device-hermes-agent-media-e2e.sh` now owns the physical-phone
  Hermes media acceptance path. It starts a live `finitechat-server` on a Mac
  LAN address, joins the phone app to the real Hermes agent invite, sends a
  base64 launch-automation image attachment from the phone app, requires the
  agent to observe photo/image media, requires agent text plus image replies,
  then pulls the phone's `FiniteChatStore` and verifies those replies through
  the Rust app-state projection. The script is not yet a passing release proof:
  on 2026-06-17 the app built, signed, and installed on `Paulphone Air`, but
  `devicectl launch` was denied because the phone was locked.
- iOS Simulator product-harness visual proof now covers the software-keyboard
  overlap regression: with the product-harness delivered transcript open and
  the on-screen keyboard visible, XcodeBuildMCP `snapshot_ui` found the two
  delivered message bubbles at `y=287.7` and `y=340.3`, the composer text area
  at `y=463`, and the keyboard starting at `y=517`, so the latest bubble is not
  covered by the composer or keyboard.
- Native iOS now has a Nostr sign-in/create gate backed by Rust-owned
  `nsec`/`npub` material, a keychain identity store, destructive sign-out that
  deletes the local product store/config, and a Home-first shell with a
  floating intention composer. Chats, People, Agents, and New are native
  `TabView` items; selecting New routes to the Home surface instead of using a
  custom tab bar. The People tab fetches follows/profile metadata from
  the same Nostr relays Pika used, checks batched server-backed Invite
  Availability from Finite Chat KeyPackage inventory, dims follows without
  available invite material, and exposes profile-code copy/share/lookup
  surfaces; the Agents tab exposes Hermes invite scanning and "Create New
  Finite Agent". The bounded shortcut is that People "Create Chat Room" still
  creates an ordinary FiniteChat room rather than a Rust-owned npub-addressed
  direct chat.

Remaining delete conditions for the product-state debt rows:

- Add physical-phone execution of the same matrix before first users after the
  local Xcode account/provisioning prerequisite is satisfied and the phone is
  unlocked/awake for `devicectl launch`. The command path now fails early if
  the identifier/signing inputs are absent, normalizes CoreDevice identifiers to
  hardware UDIDs, and can build/sign/install the app for the attached
  provisioned phone; the remaining blocker is an actual full phone harness run
  past launch.
- Repeat the Hermes media round trip on the physical phone before first users,
  after the product harness phone matrix is passing on the same configured
  server URL.
- Replace the People-tab ordinary-room fallback with a Rust-owned contact/chat
  action that accepts an npub/account id, reuses an existing 1:1 room when one
  exists, creates/provisions a new one through the product invite/contact flow
  when none exists, and is covered by a spouse-profile E2E using real Nostr
  profiles/follows.

## Review Rule

Before each finitecomputer integration checkpoint, review this ledger and answer:

- Did this checkpoint add a new shortcut?
- Did an existing shortcut gain a smaller delete condition?
- Did tests prove the shortcut is still bounded?
- Did any user-facing copy become less honest about hosted web versus true
  end-to-end encryption?
