# Hermes ⇄ Finite Chat

The `finitechat` plugin connects a [Hermes agent](https://github.com/NousResearch/hermes-agent)
to end-to-end-encrypted Finite Chat rooms. The current flow is Welcome-first:

1. The runtime publishes the Agent Principal `npub` through its contact
   document; gateway startup does not invent a room.
2. A user Device publishes a KeyPackage and starts a profile chat with that
   principal.
3. Finite Chat commits the MLS Add, the agent claims the Welcome through its
   generic Device inbox stream, and Hermes receives only MLS-authenticated
   messages.

## Install

The default way to get the binary is the released build: run the install
block at the top of [the repo README](../../README.md), which downloads the
`finitechat` release asset for your platform, verifies its sha256, and
installs it to `~/.local/bin`. Building from source is the alternative for
development checkouts:

```bash
cargo install --path crates/finitechat-cli   # installs `finitechat`
```

Then onboard (one drop-in binary owns all crypto and state):

```bash
# 1. Initialize the agent home (defaults to ~/.finite/agent; override with
#    --agent-home DIR). The account key is the shared Finite identity at
#    ~/.finite/identity/identity.json ($FINITE_HOME/identity in hosted
#    runtimes) — minted here if no Finite tool has run yet, reused if one
#    has. Inspect it with `finitechat auth status`; bring an existing nsec
#    with `finitechat auth import` (stdin or --file). Use
#    --server http://127.0.0.1:8787 for a local development server.
finitechat hermes init --server https://chat.finite.computer

# 2. The plugin (Hermes ≥ 0.16 plugin layout)
finitechat hermes install
```

Enable it in `~/.hermes/config.yaml`:

```yaml
plugins:
  enabled:
    - finitechat

gateway:
  platforms:
    finitechat:
      enabled: true
```

Then `hermes gateway start` makes the Agent Principal reachable. The dashboard
Hosted Web Device starts the room independently.

## Native Hermes capability profiles

The managed Finite Private `glm-5-3-flash` profile declares
`model.supports_vision: true`. Hermes cannot discover capabilities for the
generic `custom` provider. With its default `agent.image_input_mode: auto`,
the pinned Hermes sends attached images directly to the main model and uses
the native image-loading path of `vision_analyze` when no explicit auxiliary
vision backend is configured. An explicit `auxiliary.vision` backend still
takes precedence in this Hermes version. Startup removes only the backend the
deleted AEON specialization writer installed, keeping the replaced config as
`config.yaml.pre-aeon-vision-retirement`; see the runtime-image runbook.
The runtime reconciler
backfills only a missing declaration on the known Finite-owned model/route/key
shape; explicit capability and image-routing settings remain user-owned.
Absence means the managed product default: deleting the declaration causes
startup to restore it. Set an explicit capability or routing override to
change that behavior.
Selecting another inference profile replaces the model block, so the
declaration does not carry over to an unrelated model.

Finite Chat conveys authenticated attachments to Hermes without choosing a
model, rewriting the channel prompt, or registering Finite-specific agent
tools. Auxiliary capabilities are runtime configuration behind Hermes's
existing tools. For example, an `auxiliary.vision` profile can route Hermes's
built-in `vision_analyze` and `video_analyze` tools to a dedicated
OpenAI-compatible vision endpoint while the main model remains responsible
for deciding whether those tools are useful.

```yaml
auxiliary:
  vision:
    base_url: https://inference.example/v1
    api_key: ${VISION_API_KEY}
    model: vision-capable-model-name
    timeout: 120
platform_toolsets:
  finitechat:
    - hermes-cli
    - video
```

`video` is an explicit Hermes opt-in. The `hermes-cli` base preserves the
ordinary Finite Chat tool catalog; a bare `video` list would replace that
catalog rather than extend it. Runtime admission should verify that the
installed Hermes catalog actually contains `video_analyze`, since older Hermes
images may not provide the native tool.

The same rule applies to other capability families: prefer a model or
provider profile behind a Hermes-native capability. Add a new generic Hermes
capability only when Hermes has no suitable surface; do not add product- or
model-named tools to this transport plugin. Semantic audio interpretation is
currently such a Hermes capability gap and is not represented as a custom
Finite Chat tool.

`finitechat hermes install` writes the embedded `finitechat` plugin into
`$HERMES_PLUGINS_DIR/finitechat`, `$HERMES_HOME/plugins/finitechat`, or
`~/.hermes/plugins/finitechat`. It also writes a local `finitechat.env` file with
the Agent Home and binary path. The plugin treats that file as defaults only:
explicit Hermes config and process environment still win.
Pass `--service-url URL` to also write `FINITECHAT_HERMES_SERVICE_URL` for a
supervisor-managed `finitechat hermes serve` process.

For the supervised Rust bridge work, `finitechat hermes serve` starts the
loopback service boundary and exposes `GET /healthz` plus `GET /readyz`. The
plugin starts that service itself when no `FINITECHAT_HERMES_SERVICE_URL` is
set. Compatibility mode can fall back to the CLI-per-call bridge when the
service is unreachable.
The Finite Computer production runtime sets `FINITECHAT_HERMES_INBOUND_STREAM=1`
and treats the resident `GET /v1/hermes/inbound` NDJSON path as mandatory.
In that strict mode, stream failures reconnect with bounded backoff and resume
from the Rust service's durable cursor. They never fall into Python timer
polling or CLI-per-message subprocess calls. One-shot polling and CLI fallback
remain available only when inbound streaming is disabled.

## Inbox in-flight state and reply routing live in Rust

The Rust sidecar owns the chat delivery contract end to end (ownership audit
O1/O2); the Python adapter keeps no route table, dedup set, or SQLite state of
its own.

- **In-flight state (O1).** Each inbox entry carries a lease: `Pending` or
  `Leased`. The stream / `poll` / `inbound` deliver only deliverable entries and
  flip them to `Leased`, so a leased entry is not re-emitted on the next tick.
  The adapter settles the lease from the turn: the completion hook `ack`s on
  success or failure, and a turn cancelled by shutdown or recovery calls
  `release`, which returns the entry to `Pending` for redelivery. It decides
  once per turn. The pinned base adapter runs the hook again, as cancelled,
  when a shutdown cancels the turn after the hook began, and a second decision
  could reverse the first: the sidecar's `ack` removes an entry that a
  `release` has just returned to `Pending`. So the settlement runs as its own
  task, which that cancellation cannot interrupt, and the second run waits for
  it and resends the same decision to any entry the sidecar did not confirm.
  Hermes `stop()` interrupts running turns cooperatively and reports them as
  success, so model work whose gateway handler returns after `stop()` begins
  is released. Work whose handler returned before `stop()` began ran
  uninterrupted, so it is acked even when it settles after `stop()` began,
  unless the stop cancels it before its reply is sent. The gateway still has
  its own work to do after an agent run ends, and that can overlap the start
  of `stop()`: a restart drain calls `stop()` 50 ms after the last run ends.
  Such a turn is released and can run once more after restart. A command
  Hermes carries out itself is acked once even when a stop follows or cancels
  it, so a restart never repeats its effect. Controls such as `/restart`,
  `/undo` and `/yolo`, and replies to a pending prompt, count once the
  gateway's handler has received them; `/goal` (the saved goal) and
  `/blueprint` (a scheduled job) once the handler has returned without
  rewriting them. A stop that cancels a control after the handler received it
  but before it took effect leaves a command the user resends. A command the
  handler never received, or one that starts model work before Hermes ran or
  rewrote it, is released like other model work. While
  Hermes drains to stop or restart it refuses any new model turn with a reply
  that reports success, so the adapter holds delivered work instead and
  releases it on disconnect. Work is anything Hermes would not dispatch as a
  gateway command, by the pinned base adapter's own rule, so path-like text
  such as `/usr/bin/x` waits too, as does `/curator`, which the pinned gateway
  passes to the model as text. So do the commands Hermes turns into a model
  turn for their message: `/queue`, `/steer`, `/plan`, `/learn`, `/init`,
  `/blueprint` and `/moa` rewrite it into the agent's input, `/retry` re-sends
  the last message and `/goal` queues a kickoff turn. A pinned test derives
  that set from the gateway's dispatch. Other commands still answer during a
  drain. A drain can begin after admission but before Hermes checks: the
  adapter re-checks just before handoff, and releases ordinary work, or a
  command Hermes rewrote into the turn's input, that Hermes answered without
  binding a run to the turn's session guard; a redelivery then waits out the
  drain. A message handed over as the drain begins can show Hermes's refusal reply
  and still run after the restart. A `/goal` handed over in that window is
  acked: it has saved the goal, and only its kickoff turn is refused,
  visibly. Its unstarted kickoff is now retained in the child-work journal
  and resumed on restart, separately from inbox foreground replay. A started
  child whose completion is uncertain receives an explicit interrupted outcome
  instead of automatic execution replay. `/retry`
  rewinds the transcript before it re-sends the last message. If a drain
  refuses the re-sent turn or a stop cuts it off first, the adapter puts the
  rewound transcript back and releases the entry, so the same message is
  retried once after the restart. It undoes the rewind only while the
  transcript is exactly what the rewind left, under the store's transcript
  lock; otherwise it acks rather than rewind twice. To see the rewind, the
  adapter wraps the gateway session store's `rewrite_transcript` and
  `rewind_session`, which run unchanged outside a Finite `/retry` turn. A message Hermes
  queues behind a reserved or busy session slot, or a `/queue` it copies
  there, is settled by the turn that runs it. If that is a `/retry`'s
  re-sent message and a stop drops the queue first, the adapter undoes the
  rewind the same way when it disconnects and releases the entry. The adapter
  also holds every non-internal event, commands included, while Hermes's
  startup-restore gate is closed: the gate queues events in memory and
  reports them handled, which would ack them before they run. The inbox is
  the only owner of interrupted-turn recovery, so the adapter declines the
  synthetic auto-resume turn Hermes starts at boot for sessions `stop()` or
  crash recovery marked. The redelivered message runs once, with Hermes's
  recovery note. An interrupted internal event has no inbox redelivery and
  no automatic restored-session notice; its transcript and recovery mark stay
  until the next real message. The default session-reset mode is `none`;
  Agents opting into a reset mode retain Hermes's stale-mark reset behavior.
  After a crash nothing runs until the lease expires. The
  stream never leases for a client that has disconnected, and it releases a
  batch it could not send. A user
  `/stop`, `/new` or `/reset` instead `ack`s the cancelled turn and its held
  queued admissions, before any drain or stop rule, even when the turn
  finishes before Hermes marks it cancelled; earlier undelivered entries are
  acked when delivered in that process. A lease older than the TTL (config, generous default) is swept back
  to `Pending`, so a crashed turn cannot strand
  an entry. The sidecar keeps a bounded recently-acked ring, so a post-restart
  duplicate ack is a no-op and an already-acked entry is never redelivered —
  idempotency the adapter no longer has to provide. Existing `hermes-inbox.json`
  entries load as `Pending` (`#[serde(default)]`), so the on-disk format is
  unchanged.
- **Busy-session admission.** While a Hermes session is busy the adapter holds
  delivered ordinary events in arrival order per session, retaining their
  leases in the Rust inbox. It admits them one at a time. Releasing later
  events while holding only the head would let a partly consumed stream batch
  overtake those released events when the session becomes idle. Renewed leases
  for a queued or running event are coalesced without changing its position.
  These in-memory holders grow with the delivered backlog; the Rust inbox
  remains the only durable queue. Gateway commands, pending approval responses, and pending
  clarification replies still reach the active turn immediately, and one busy
  session does not pause another. The exceptions are `/queue` and a `/steer`
  the running agent cannot take: Hermes would keep either as a new in-memory
  event, acked before it ran and dropped by a stop or drain. They wait with
  ordinary text and run as their own turn, without Hermes's "Queued" reply; a
  media-only `/queue` caption gets Hermes's idle usage reply instead. A
  `/steer` the running agent takes settles with the turn it steered, so a
  stop that interrupts that turn redelivers both. A steer that arrives after
  the agent's last tool call becomes Hermes's in-memory follow-up, which a
  drain still drops. Commands Hermes rejects mid-turn, such as `/plan`, keep
  its visible "can't run mid-turn" reply. After a turn's ack, its background
  task still runs the usage-notice call and cleanup; the base adapter treats
  the session as busy until then while Hermes is idle, and would run a
  command inline outside any background turn. A command that starts model
  work, such as `/retry` or `/plan`, waits for its own turn instead. So do
  `/goal <text>` and `/goal resume`; the `/goal` controls (`status`, `show`,
  `pause`, `clear`, `stop`, `done`, `wait`, `unwait`, `gate`) answer at once,
  so a pause sent between goal turns stops the loop. A control sent while the
  goal judge runs stays in force when its verdict returns (the image's
  goal-judge patch, `infra/images/README.md`). During a drain Hermes also refuses
  clarification and approval text sent to a busy session; that refusal is
  shown and acked, because replaying it later would start a turn without its
  prompt. Text, photos, audio, video, and files each
  enter their own background turn and retain their lease until its completion
  hook settles it. Separate media messages are not merged into Hermes's pending
  slot; multiple attachments on one message still travel together. Graceful
  shutdown releases every queued lease for immediate redelivery; a crash
  leaves the entire held backlog recoverable through the sidecar's normal
  expiry path (45 minutes by default). Settlement uses one RPC per entry, so
  an interrupted shutdown can leave remaining leases waiting for that expiry.
  A failed handoff retries the head with exponential backoff (1–30 seconds)
  before admitting later events. User interruption also clears held work in
  the idle gap between turns. Events consumed inline by a busy
  session never pass through a background turn, so the adapter acks them
  directly (exactly once; the sidecar's ack is idempotent).
- **Reply/edit routing (O2).** Every inbound event already carries its
  conversation and segment ids, and the sidecar mints `thread_id` from them. On
  send/edit/activity the adapter passes that `thread_id` back, and the sidecar
  resolves it against its own agent store into the concrete route. An `edit`
  with no route fields is resolved by looking the original message up by
  `(room_id, message_id)`. An explicit Topic/Chat route still wins as an
  override; an unknown thread id falls back to the Home default with a loud
  warning (an archived topic must never silently consume a message). There is
  no policy switch: the fallback is the only behaviour.
- **Error classification.** Core decides each failure's class and whether the
  same request may be retried (`FiniteChatCoreError::classification`); the
  sidecar's error envelope (`error_kind`, `retryable`, HTTP status) and the
  daemon's status are derived from that one decision, and the adapter reads
  those fields verbatim. Nothing on either side matches on error text.

None of this changes the Rust inbox on-disk format, the CLI/service protocol
(the `release` command and the optional `thread_id` request field are additive),
or the deployment order.

## Pinned Hermes clarification and compaction boundary

Pinned Hermes owns clarification state in `tools.clarify_gateway`. Its gateway
registers the pending question under the exact session key, calls the platform
adapter's `send_clarify`, and resolves typed answers through that same session.
Telegram renders the full question and choices with inline buttons; Discord
renders the full question in content plus an embed and uses buttons for choices.
Both adapters resolve button choices through `resolve_gateway_clarify`; open
answers and typed choice replies use Hermes's session-scoped text interceptor.
Typing is paused while either adapter waits, and their normal whole-turn
processing reactions remain separate from clarification state.

Finite Chat uses that same pending state and ordinary Chat messages. Its
adapter requires the originating Finite topic and chat to resolve before it
delegates prompt formatting to Hermes, pins the send to that exact route, and
explicitly bypasses emoji/prose kind inference. A missing or unknown route
returns a visible adapter failure to Hermes instead of falling back to Home or
whichever Chat is active. Finite does not persist a second clarification state
or add clarification request/answer protocol types.

The pinned Hermes runtime does not expose a semantic compaction start/finish pair to
platform adapters. Compaction emits human-readable status strings, an internal
post-compression `session:compress` hook, and the whole-turn
`on_processing_complete` callback; none gives an adapter both semantic edges.
Telegram and Discord have no separate compaction callback or UI contract.
Consequently, compaction UI is parked until a later Hermes version provides a
clean adapter hook; Finite must not infer it from status prose or markers.

See [HARDENING.md](./HARDENING.md) for the adapter reliability plan and
acceptance matrix.
See
[../../../finitecomputer-v2/docs/hermes-runtime-test-matrix.md](../../../finitecomputer-v2/docs/hermes-runtime-test-matrix.md)
for the current local Apple Container → Kata → Phala proof ladder.

## Agent → user attachment contract

Hermes sends a newly created local file as a typed attachment. The Python
adapter does not read, encode, or upload it:

```json
{
  "kind": "media",
  "status": "complete",
  "attachments": [{
    "kind": "image",
    "name": "site-preview.png",
    "mime_type": "image/png",
    "path": "/data/workspace/site-preview.png",
    "url": null,
    "blob": null
  }]
}
```

Before appending any MLS message, the Rust sidecar validates every local path,
reads regular non-empty files within the 32 MiB per-file and 64 MiB per-send
limits, encrypts/uploads each file through the room's pinned Finite Chat blob
service, and replaces `path` with the returned durable `blob` plus its canonical
`url`. Name, MIME type, and media kind are preserved. A request may contain at
most 16 attachments under the Hermes v1 DTO limit. A bad/unreadable/oversized
path or upload failure returns an error without appending a chat message.

An attachment already carrying a valid `blob` is not re-uploaded. This is the
normal echo/forward case for an inbound blob that Rust materialized for Hermes:
the local `path` is stripped and the blob's canonical URL is retained before
append. A URL-only attachment remains a pass-through external reference; agents
should use `path` for new local output and `blob` for already durable Finite
Chat media. The promotion happens synchronously on `send`; it does not poll,
and agent-local filesystem paths never enter the encrypted room log.

For a local human smoke with JSON evidence:

```bash
just chat-reliability-fast
scripts/hermes-sidecar-smoke.sh
scripts/hermes-agent-media-e2e.sh
scripts/ios-hermes-agent-media-e2e.sh
```

The adapter regression command writes
`target/hermes-adapter-regressions/report.json` and proves the Hermes-internal
behaviour that remains the Python adapter's responsibility: plain messages,
busy-session admission, clarification routing, poll recovery, sidecar
startup/fallback/serialization, media, typing activity, room filters, group
sender identity, receipt/control stream filtering, and strict stream recovery.
It fails if a required test is missing or skipped. Inbox lease/ack/release,
reply/edit route resolution, and delivered-event dedup are proven by the Rust
sidecar tests (`cargo test -p finitechat-cli -p finitechat-hermes`), not here.
The CLI round-trip script writes `target/hermes-sidecar-smoke/report.json` for
server startup, Welcome-first room admission, direct `finitechat hermes poll`,
text/media replies, user decrypt, and invalid-media rejection. Despite its
historical filename, it does not start `finitechat hermes serve`, consume the
NDJSON inbound stream, or prove ack/drain behavior; those exclusions are
recorded in the report.
The media E2E writes `target/hermes-agent-media-e2e/report.json` and runs the
real `hermes-agent` package against the Finite plugin with the sidecar inbound
stream enabled. It proves an image sent by a Finite Chat user reaches Hermes as
media and that the user decrypts both text and image replies from the agent.
Agent-local reply paths are never written into the room log: the Rust sidecar
uses the contract above and appends only the durable encrypted blob reference.
It installs an echo callback, so it is adapter transport coverage, not a real
Hermes model or gateway acceptance gate.
The canonical real-gateway acceptance is the monorepo
`just dev saas-smoke` path. It packages the flake-pinned Nix Hermes runtime and
this plugin in the one Runtime image and requires model-backed replies across independent
chat-server, Hosted Web Device, and Runtime restarts.
For the canonical durable Docker packaging smoke used by the manual workflow:

```bash
scripts/hermes-durable-home-docker-smoke.py \
  --image finite-agent-runtime:<built-tag>
```

It starts the canonical Hermes gateway, creates the room through
KeyPackage/Add/Welcome, requires a real model reply, restarts compute around
the same durable `/home/node`, verifies the same npub and Room, and requires a
second reply.

## How the pieces divide (ADR 0002)

The Python adapter stays thin and talks to the resident loopback Finite Chat
service. The Rust binary owns identity, MLS encryption, Welcome processing,
durable cursors, storage, inbox in-flight state (leases), and reply/edit route
resolution. The service surface covers inbound stream, acknowledge, release,
send/edit, activity, recovery, and explicit home-channel state; strict hosted
mode never falls back to Python polling or per-message CLI subprocesses.


Accepted `/bg`, `/btw`, and goal continuations have a separate Hermes-owned
`hermes-home/finite-child-work.sqlite3` journal. Include it and
`hermes-home/finite-child-results/` in the same Recovery Set as the existing
Hermes state. Acceptance precedes acknowledgement, and execution-start markers
precede effects. Background and goal completion manifests, including retained
local attachment bytes and a receipt cursor for every item, precede result delivery. Delivery retries have at-least-once
semantics: transport success followed by a crash before its durable receipt can
repeat a reply, but never automatically reruns background model/tool effects.
An interrupted execution reports its uncertainty; an unchanged active goal is
paused for review. Saved unstarted goals resume only if their exact persisted
state still matches, the source remains authorized, and the foreground session
and inbox admission queue are free. Generic Hermes resume remains suppressed.

`finite-child-work-v1` accounting includes pending child outcomes. Its status
marker is bound to the writer's PID and process start time; an old-runtime
writer cannot inherit the marker after rollback. This is observability, not an
admission/drain fence, and cannot authorize automated upgrades by itself.

An older Runtime can ignore this journal while replaying foreground work or
advancing a goal outside it. Keeping its bytes unchanged is not safe rollback,
and rolling forward cannot undo effects already performed by the old binary.
Candidate-to-old downgrade is held while obligations are pending or unknown,
or admissions are not fenced; the current protocol has no supported fence.
The hold includes automatic old-image restart after an upgrade failure, which
currently bypasses the rollout wrapper. Preserve the whole Recovery Set and
all newer accepted writes; do not clear journal rows or restore an older
snapshot to make a downgrade pass. See the [Runtime rollback hold](../../../infra/runbooks/runtime-image.md#hold-for-runtime-child-work-recovery-candidates).
