# Finitecomputer Integration

## Existing Seam

Do not replace the current outbound relay shape. It is the right boundary for
hosted runtimes and for agents running elsewhere:

```text
Dashboard route
  -> finited admin API
  -> machine relay event
  -> finitec relay run inside runtime
  -> ChatRuntime / Hermes gateway
  -> finitec gateway send/edit
  -> relay snapshot / stream
  -> Dashboard route
```

Finite Chat should first replace the semantics behind "chat event" and "chat
message", not the dashboard shell. For the canary path, this should be a hard
cut to Finite Chat as the live transcript: the legacy `ChatRuntime` transcript
can be imported or rendered as read-only archive, but should not be dual-written
as another live source of truth.

The long-term product boundary is not the host control plane. A `finite` or
`finitec` daemon should be deployable inside any agent, hosted anywhere, and
should connect outward to Finite. The dashboard and Finite Chat app read Finite
relay/projection state and send typed commands through that outbound agent
connection. Hosted-runner admin remains only for infrastructure Finite operates,
such as hostname reservation, auth policy, route rendering, and runner image
rollouts.

The hosted finitecomputer web path is a trusted-server-client mode, not true
end-to-end encryption. A server-side Rust Finite Chat client may decrypt room
state and expose the existing dashboard DTOs so the web frontend changes as
little as possible. Only local daemon, Electron, native mobile, or other clients
that keep Finite Chat device secrets on the user's device should be described
as end-to-end encrypted.

Hosted dashboard product copy should call this "web chat" or "topics", not
"encrypted chat". Finite Chat can be the internal protocol and storage upgrade;
E2EE language starts only when users run clients that keep keys off the hosted
server.

## Command/RPC Mapping

Dashboard and Hermes commands ride inside Finite Chat durable application
events, not a separate dashboard-to-runtime backdoor. The dashboard sends
`runtime.command.request` as an encrypted durable event in the Project runtime
room. The runtime device syncs ordered room entries, decrypts the request,
validates sender and target policy locally, persists a request ledger entry, and
then schedules execution.

Chat payloads and management commands stay separate at the application-kind
level. Chat messages, receipts, attachments, and topic updates are not shoved
through a generic command queue; management commands are typed, allowlisted
runtime requests with idempotent handlers.

Command payloads should use the Finite Chat v1 typed runtime command shape:
`runtime.command.request` with `request_id`, command name, encrypted target
account/device, optional resource key, schema id, and bounded JSON body.
finitecomputer command bodies should use namespaced schema ids such as
`finitecomputer.runtime.inference.apply.v1`, not ad hoc unversioned maps.

Intermediate states such as thinking, working, tool-running, upload progress,
or runtime presence use ephemeral activity events with `push_policy = never`.
User-visible output, durable checkpoints, terminal success, terminal failure,
and cancellation results are durable application events.

Status reads should not become durable command spam. The runtime should publish
encrypted `runtime.state.snapshot` events when state changes and on a slow
bounded refresh cadence. The dashboard normally reads its local decrypted latest
projection. An explicit refresh button may send a `runtime.command.request`, but
page load should not append request/result events just to ask what the runtime
already knows.

The current relay shell can still wake/poll the runtime. A wake only triggers
sync; it must not directly execute work from an external event callback. Optional
cleartext wake hints may wake a specific runtime device, but the decrypted
command target and local policy remain authoritative.

Transport should start with HTTP mutations, cursor-based pull sync, and SSE
hints. WebSockets are not needed for v1. SSE and relay wakes only trigger sync;
durable ordered room state remains the source of truth.

## Dashboard Command Audit

The current finitecomputer relay surface on `main` has four categories:

- Projection reads: `chat.bootstrap`, `chat.list_threads`,
  `chat.list_messages`, `chat.list_slash_commands`, and `chat.get_attachment`.
  These should become local reads against the trusted-server Finite Chat
  projection or encrypted blob store. They should not be modeled as runtime
  commands.
- Chat mutations: `chat.create_thread` and `chat.send_message`. These map to
  durable Finite Chat events: `conversation.create` for creating a topic and
  `chat.message` for user input. Runtime inbox work is derived only after the
  runtime syncs and decrypts ordered room events.
- Runtime commands: `runtime.inference.validate`, `runtime.inference.apply`,
  `runtime.inference.rollback`, `runtime.gateway.restart`, and Matrix
  connection commands. These should use `runtime.command.request` and
  `runtime.command.result` with namespaced encrypted command names such as
  `finitecomputer.runtime.inference.apply`.
- Decoupling targets: published-app inventory, runtime-owned connection status,
  Telegram/Matrix/Hermes config, Codex status, local skills sync, and other
  portable agent capabilities should move behind the finitec daemon boundary.
  The practical test is whether the feature can work for an agent outside
  Finite's k3s cluster with only `finite` installed.
- Hosted-runner admin: hostname reservation, hosted auth policy, route
  rendering, runner image updates, and emergency pod/runner operations can stay
  outside Finite Chat because they are infrastructure Finite operates. These
  paths should not become product-level dependencies for self-hosted or
  bring-your-own-agent users.

Config-editing commands must be serialized per target runtime resource. For
example, inference profile apply, rollback, and gateway restart all touch Hermes
config or gateway lifecycle and should share a runtime-side command ledger key
such as `hermes.config`. Command retries reuse the same encrypted envelope and
idempotency key. Results include the post-mutation status snapshot but should
use `push_policy = never` unless the user explicitly asked to be notified.

Bridge and connection commands should use the same rule for physical isolation.
Telegram, Matrix, and other bridge adapters get distinct resource keys such as
`connection.telegram` or `connection.matrix`. Commands for one bridge serialize
against that bridge's local resource, while a different bridge on the same
runtime device can keep making progress. If a bridge later moves to a separate
physical process or host, target it as a separate Finite Chat device instead of
special-casing the UI.

## Runtime State Snapshots

Finitecomputer should model bot-visible status like structured AIM status
messages: compact, current, non-notifying, and useful to both humans and agents.
The finitec daemon publishes encrypted `runtime.state.snapshot` events keyed by
the runtime device and a stable `state_key`. The dashboard projects the latest
snapshot for each key instead of asking the runtime for status on every page
load.

Initial state keys:

- `runtime.inference`: active model/provider/profile, backup availability, and
  restart support;
- `runtime.gateway`: gateway health, connected platforms, and restartability;
- `runtime.connection.matrix`: Matrix configured/connected state;
- `runtime.connection.telegram`: Telegram token/access/topic status and pairing
  state;
- `runtime.connection.google_workspace`: Google Workspace credential and tool
  availability;
- `runtime.connection.codex`: Codex auth and device-code state;
- `runtime.published_apps`: runtime-owned app inventory and observed process
  state;
- `runtime.capabilities`: finitec/agent capabilities and schema versions.

Snapshots are not presence. Runtime liveness stays in the small server-visible
heartbeat path. Snapshots are encrypted app state with `push_policy = never`.
They include `observed_at`, `expires_at`, `revision`, `schema`, and a typed
status object. When a command mutates state, its result should include or be
followed by the corresponding snapshot so every client converges without an
extra read command.

## Daemon Survival Requirement

Finite Chat is the recovery control surface when the agent stack is unhealthy.
Hermes can be absent, hung, misconfigured, or unable to reach inference.
Finite Chat still needs to sync rooms, publish runtime state, and accept
allowlisted recovery commands while the host and daemon are online.

The daemon must not depend on Hermes or inference for:

- room sync and projection;
- `runtime.gateway` or `runtime.inference` state snapshots;
- recording `runtime.command.request` in the local command ledger;
- restarting the Hermes gateway;
- reporting terminal failure when an assistant reply cannot be produced.

The integration canary should include the survival gate in
`docs/daemon-survival-testing.md`: Hermes down at startup, inference down,
gateway restart while Hermes is down, daemon restart during the command, and
SSE loss repaired by pull sync.

## Topics And New Chat

Finitecomputer should ship first-class topics as the product surface for Finite
Chat conversations. One project or agent runtime gets one encrypted room, and
that room can contain many topics. Creating a new topic appends
`conversation.create` with encrypted topic metadata such as title, description,
model/skill binding, and external bridge metadata.

The app shell's "New chat" action should create a new topic. A `/new` command
inside an existing topic should not create another topic; it should append
`conversation.segment.start` inside the same `conversation_id`. The UI can render
that event as a divider. Hermes owns the actual prompt/session reset behavior;
Finite Chat only preserves the ordered boundary all clients can agree on.

Hermes `thread_id` and Telegram `message_thread_id` map naturally to topic
`conversation_id`. Finite Chat should store external platform identifiers and
topic names in encrypted conversation metadata. The cleartext `conversation_id`
exists for routing and indexing only; it does not authorize access and it does
not need to expose the external platform's raw identifier.

Topic names are mutable display metadata, not identity. A future Finite Chat
client may render them like Signal group names: editable by members with admin
permission, stored in encrypted conversation metadata, and resolved by the
ordered room log when concurrent renames happen. The stable link between
Hermes, Telegram topics, and Finite Chat remains `conversation_id`.

Existing plaintext chats do not need an indefinite compatibility path. The
finitecomputer hard cut may import them once as archived/read-only
conversations, then require new work to happen in explicit Finite Chat topics or
untopic'd conversations.

## Proposed Landing Shape

The first finitecomputer canary should embed Finite Chat Rust crates directly in
`finited` and `finitec`, while keeping the API shaped so a standalone daemon can
be extracted later. Do not require a separate `finitechatd` process for the
first canary; that would add a deployment, auth, logging, and upgrade surface
before the protocol boundary has proved itself in the product.

Add a Finite Chat-backed mode in finitecomputer with five immediate layers:

1. `finitechat-proto`: shared DTOs used by dashboard server routes, finited,
   finitec, and tests.
2. `finitechat-engine`: reducer/store used by `finited` in local/dev and by a
   future canary room server.
3. `finitechat-hermes`: shared JSON bridge contract for the Hermes platform
   plugin. finitecomputer should import this contract instead of defining its
   own gateway event shape.
4. `finite`/`finitec` agent daemon: the portable process installed inside any
   agent runtime. It owns the outbound connection, runtime device state,
   management command handlers, Hermes/agent adapters, heartbeat, and local
   capability reporting.
5. `finitec encrypted-chat`: runtime/client commands that manage device state,
   KeyPackages, Welcome claim/ack, room sync, and Hermes gateway bridge.

Later, extract `finitechatd` as the local user daemon that owns device secrets,
MLS state, sync, projections, command ledger, and attachment download/upload.
That extraction should be a packaging decision over an already-tested boundary,
not a prerequisite for the first finitecomputer canary.

The dashboard should keep the current `FiniteChat` component contract as long as
possible. The server route can translate encrypted room state into the existing
render model while the encrypted transcript becomes canonical.

The standalone Finite Chat product should grow from the CLI/daemon first. The
finitecomputer integration should consume the same daemon surface that a
self-hosted or third-party agent can use. Electron and native apps can reuse the
same Rust core when the local true-E2EE clients are ready.

## Mapping To Current Files

`crates/finite-core/src/chat.rs`

- Keep current render DTOs for the UI.
- Add encrypted room DTOs separately. Do not overload plaintext message structs
  with MLS envelope fields.
- Model receipts as encrypted durable `chat.receipt` events with
  `push_policy = never`, not as user-notifying messages.
- Replace current attachment semantics with encrypted Blossom-compatible blob
  references in decrypted message DTOs.

`crates/finite-core/src/chat_runtime.rs`

- Treat as read-only archive/import input for canary Projects.
- Add a Finite Chat runtime store instead of mutating `messages` in place.
- The gateway inbox can be fed from decrypted application messages after the
  runtime device processes room sync.
- Drive encrypted device maintenance through `finitechat_client::run_runtime_sync_tick`:
  it replenishes KeyPackages, persists replayable pending uploads with local
  MLS state before upload, claims and activates Welcomes, retries pending
  Welcome acks, and applies bounded ordered room pages into the encrypted client
  store.
- Drive later-device room fanout through `finitechat_client::run_link_fanout_tick`:
  after a target device is registered and replenished, an existing device pages
  the account's rooms, claims one target-device KeyPackage per room, persists
  the room plan and prepared MLS Commit, submits idempotently, and completes
  from the ordered room log.

`crates/finite-core/src/relay.rs` and `crates/finited/src/main.rs`

- Current relay events are short-lived file-backed commands.
- Encrypted room logs need durable ordered storage, not only event files.
- Add a room-log store with transaction semantics before production use.
- Keep machine polling for command/result flow.
- Add SSE as a hint channel for local/dev and hosted routes, but keep pull sync
  as the repair path.

`crates/fc/src/main.rs`

- Add commands under a separate namespace first:
  - `finitec encrypted-chat device register`
  - `finitec encrypted-chat keypackages upload`
  - `finitec encrypted-chat rooms sync`
  - `finitec encrypted-chat gateway poll`
  - `finitec encrypted-chat gateway send`
- Once stable, route existing `finitec gateway` through encrypted chat for
  canary Projects.

`integrations/hermes/finitechat/adapter.py`

- Keep the adapter CLI-only.
- It should not import finitechat internals or start a local HTTP service.
- The command it shells out to can change from `gateway` to encrypted gateway
  subcommands after the finitec surface is stable.

`apps/dashboard/src/lib/finite-relay-client.ts`

- Keep the relay client shape for dashboard calls.
- Add feature-gated encrypted endpoints only in server-side routes, not in the
  client component, until the render model needs new states such as
  `NeedsRepair` or device linking.

## Migration Strategy

Start with new canary rooms only.

- Do not import old Pika rooms.
- Do not transparently convert existing finitecomputer plaintext threads.
- For existing users, create a fresh encrypted Project chat and expose old
  finitecomputer plaintext threads as read-only archived chats until a separate
  migration plan exists.

## Runtime State

Suggested runtime directories:

```text
$HOME/.finite/chat/plaintext-archive/ # read-only imported legacy chats
$HOME/.finite/chat/encrypted/
  device.json                       # account/device identity metadata
  client.sqlite3                    # local MLS and sync state, encrypted at rest
  attachments/
  keypackages/
```

Server/control-plane state should not live in the runtime directory. Room logs,
Welcome delivery records, and idempotency results belong to the room server or
`finited` local dev store.

## Feature Flags

Use explicit flags during integration:

- `FINITE_CHAT_MODE=plaintext|encrypted|dual`
- `FINITE_CHAT_ROOM_SERVER_URL`
- `FINITE_CHAT_DEVICE_ID`
- `FINITE_CHAT_STATE_DIR`

`dual` is for local validation only: write encrypted rooms while preserving the
current plaintext render path.

## First Finitecomputer PR

Keep it boring:

1. add crates and docs;
2. wire compile/test only;
3. add no dashboard behavior change;
4. add one local CLI smoke that creates a fake-MLS room and proves Commit
   ordering.

After that, add the room server API and dashboard local-loop feature flag.

## Debt Hygiene

Observed integration debt is tracked in
`docs/technical-debt-ledger.md`. Before changing finitecomputer, update that
ledger when a shortcut is introduced, narrowed, or deleted. The ledger is
deliberately loud: every tolerated shortcut needs a source, risk, first proof,
and delete condition.
