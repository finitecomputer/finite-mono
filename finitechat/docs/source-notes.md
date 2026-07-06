# Source Notes

Research date: 2026-05-21.

Local sources read:

- `https://github.com/justinmoon/finite`, cloned to a temp directory for the
  finite-chat protocol plan.
- `https://github.com/justinmoon/pika`, cloned to a temp directory for Pika,
  Marmot, MDK, OpenClaw, and mobile/runtime lessons.
- `../finitecomputer`, local checkout for the current plaintext chat relay,
  dashboard shape, finitec runtime connector, and Hermes platform adapter.

## Justin's Finite Planning Repo

The finite planning repo is the protocol authority for this seed. Its plan says:

- build a new Rust workspace;
- keep Pika and Cordn as prior art only;
- use MLS with Nostr account identity;
- make one room server the authoritative sequencer for each room;
- start with deterministic fake-MLS simulation before OpenMLS;
- keep Signal-level metadata protection out of v1;
- use explicit per-device identity and device binding from day one;
- treat DMs as MLS rooms;
- require Welcome release to be coupled to durable Commit acceptance.

Important planning files:

- `todos/finite-chat-protocol/plan.md`
- `docs/protocol/00-overview.md`
- `docs/protocol/02-protocol-sketch.md`
- `docs/protocol/03-decisions-and-risks.md`
- `docs/protocol/05-server-api.md`
- `docs/protocol/06-state-machines.md`
- `docs/protocol/07-simulator-plan.md`
- `docs/protocol/08-data-minimization.md`

## Pika And Marmot Lessons

Pika proves the product and client-side MLS shape:

- Rust core owns business state and mobile apps render snapshots.
- MDK/Marmot provides MLS group operations and encrypted messages.
- Nostr relays carry encrypted events without plaintext.
- KeyPackages and Welcomes are working product primitives.
- The OpenClaw/Pikachat sidecar shows a practical JSONL daemon boundary for
  external agent adapters.
- Local encrypted state uses SQLite plus keyring/file-key handling.
- Notification Service Extension code can decrypt push-related media, but
  notification previews should not define the core protocol.

The part to reject for Finite Chat v1 is relay-derived Commit consensus. MLS
needs all clients to process handshake operations in the same epoch order.
Eventually consistent Nostr relay delivery made that the brittle point.

Useful Pika references:

- `README.md`
- `docs/architecture.md`
- `rust/src/mdk_support.rs`
- `crates/pika-marmot-runtime`
- `crates/pikachat-sidecar/src/protocol.rs`
- `pikachat-openclaw/README.md`
- `pikachat-openclaw/todos/ship-marmot.md`

## Finitecomputer Current Shape

Finitecomputer already has the correct high-level transport boundary for hosted
runtime chat:

- dashboard uses Next routes under `apps/dashboard/src/app/api/chat/...`;
- dashboard calls the host relay through `apps/dashboard/src/lib/finite-relay-client.ts`;
- `finited` exposes machine-authenticated outbound polling and admin event APIs;
- `finitec relay run` polls from inside the runtime and handles typed events;
- `finite-core::ChatRuntime` stores plaintext threads/messages under
  `$HOME/.finite/chat`;
- the Hermes plugin talks only through `finitec gateway`, not a runtime-local
  HTTP server.

That is the right integration seam. Finite Chat should replace the chat payload
and room state inside the runtime/control-plane contract without making the
dashboard reach into the machine.

Relevant files:

- `docs/canary-roadmap.md`
- `docs/chat-local-dev.md`
- `crates/finite-core/src/chat.rs`
- `crates/finite-core/src/chat_runtime.rs`
- `crates/finite-core/src/relay.rs`
- `crates/finited/src/main.rs`
- `crates/fc/src/main.rs`
- `integrations/hermes/finitechat/adapter.py`
- `apps/dashboard/src/lib/finite-relay-client.ts`
- `apps/dashboard/src/lib/chat-proxy.ts`

## Hermes And OpenClaw Topics

Hermes already models the distinction Finite Chat needs for first-class topics:

- `gateway/session.py` stores `SessionSource.chat_id` plus optional
  `thread_id`; Telegram forum topics, Discord threads, and Slack threads are
  all represented as thread ids under a parent chat.
- `build_session_key` makes `thread_id` part of the stable session key. For
  group/channel threads, the default is a shared session across participants,
  matching Telegram forum topic UX.
- `gateway/delivery.py` parses delivery targets as
  `platform:chat_id:thread_id` and forwards `thread_id` through send metadata.
- `gateway/platforms/telegram.py` maps metadata `thread_id` to Telegram
  `message_thread_id`, extracts inbound `message.message_thread_id`, resolves
  topic names, and supports topic-specific skill binding.
- Finitecomputer's Hermes adapter already treats finite `thread_id` as the
  inbound/outbound session key for gateway events.

This supports the Finite Chat mapping: a topic is a user-facing conversation
inside a room, backed by `conversation_id`. A `/new` command inside a topic
should start a new segment inside that conversation, not create a new
conversation or room.

OpenClaw/Pikachat has a simpler shape: direct and group conversations map to
session keys, and the plugin keeps a `sessionKey -> group/account` table so
tools can route replies. It does not add a separate topic primitive, which makes
Finite Chat's conversation/topic layer the useful missing concept.

## Finitecomputer Command Audit

Audited finitecomputer `main` on 2026-05-21. The local checkout had an
unrelated modified `docs/canary-roadmap.md`; this audit read only.

The decoupling planning docs change the interpretation of the command split.
The target is not "leave everything host-control-plane-shaped". Finite Computer
wants a `finite` binary or finitec-owned daemon that can run inside any agent,
hosted anywhere, connect outward to Finite, and expose chat plus Finite
Computer capabilities without an inbound dashboard-to-runtime path.

Relevant docs:

- `FINITE_COMPUTER_BRIEF.md`: `finite` runs inside any agent, hosted anywhere,
  bridges that agent to Finite Computer, and exposes capabilities beyond chat.
- `docs/relay-heartbeat-and-coupling-audit.md`: the correct boundary is machine
  connects outward to Finite, and Finite reads relay state instead of reaching
  into a pod.
- `docs/finitec-transport-migration-ledger.md`: surfaces should move toward the
  Finite Chat / finitec boundary; hosted-runner admin is only for runtimes
  Finite hosts; do not add direct dashboard-to-runtime paths.
- `docs/archive/plans/decoupling-release-train-runbook.md`: the practical test
  is whether a feature can work for an agent outside the k3s cluster with only
  finitec installed.

The dashboard relay client posts `{ lane, kind, ttlSecs, payload }` and waits
for a result. Chat calls are restricted to `chat.*`, while other runtime calls
use lanes such as `runtime` and `connection`.

Current relay kinds handled by `finitec relay run`:

- `chat.bootstrap`
- `chat.list_threads`
- `chat.create_thread`
- `chat.list_messages`
- `chat.send_message`
- `chat.list_slash_commands`
- `chat.get_attachment`
- `connection.matrix.status`
- `connection.matrix.configure`
- `connection.matrix.disconnect`
- `runtime.inference.status`
- `runtime.inference.validate`
- `runtime.inference.apply`
- `runtime.inference.rollback`
- `runtime.gateway.restart`

Finite Chat should split these by responsibility. Chat reads become projection
reads, chat writes become durable chat events, and portable runtime/config
mutations become `runtime.command.request` payloads handled by the finitec
daemon. Hosted-runner infrastructure remains outside the generic Finite Chat
protocol, but only because it is about Finite's hosting substrate, not because
the central control plane is the desired product boundary.

The dashboard also has host-side operations that do not currently use relay.
Published-site auth, route rendering, runner image updates, and emergency pod
operations are hosted-runner admin. Published-app inventory, connection status,
Telegram/Matrix/Hermes config, Codex status, local skills sync, and any other
agent-portable capability should move toward finitec-owned commands and status
projections.

Status reads should use daemon-published state rather than page-load commands.
The Finite Chat shape is `runtime.state.snapshot`: a durable, non-notifying,
encrypted status event keyed by runtime device and stable state key. This gives
the dashboard the latest projected state without writing a chat message or
sending a command every time someone opens a page.

## Dependency Audit

OpenMLS credential spike:

- `openmls = 0.8.1`, MIT, used only for `BasicCredential`/`Credential`
  identity-byte integration, KeyPackage generation, Welcome staging, and
  encrypted application-message proof in the OpenMLS spike. OpenMLS SQLite
  storage is intentionally not added; the local proof uses OpenMLS memory
  storage through the RustCrypto provider. Notable transitive dependencies in
  this slice are `openmls_traits`, `tls_codec`, `zeroize`, and `rayon`.
- `openmls_rust_crypto = 0.5.1`, MIT/Apache-2.0, added as a dev dependency for
  the real OpenMLS provider in credential/Welcome proof tests.
- `openmls_basic_credential = 0.5.0`, MIT, added as a dev dependency for
  OpenMLS `SignatureKeyPair` generation in proof tests. Finite Chat still owns
  the account/device credential bytes; this crate only signs MLS leaves.
- `secp256k1 = 0.29.1`, CC0-1.0, used for Nostr-compatible BIP340 Schnorr
  account signatures. The full `nostr-sdk` and bech32 parsing are intentionally
  not added; the protocol boundary stores raw 32-byte Nostr public keys.

Internal dev-only proof dependency:

- `finitechat-engine` is a dev dependency of `finitechat-mls` only for the
  engine-through-real-MLS scenario. This does not make the server depend on MLS;
  it proves the existing server reducer can order opaque MLS bytes.
