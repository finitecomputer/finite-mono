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

## Inference routes, backup, and notices

The dashboard's inference commands, the intent record, and what disconnect
guarantees are specified in the
[runtime control contract](../../../finitecomputer-v2/docs/runtime-control-contract.md#inference-connections).
This section covers the Hermes side.

### Finite Private route and backup

On every normal start (not a recover-known-good boot) with the Runner's
`FINITE_PRIVATE_MODEL` and `FINITE_PRIVATE_BASE_URL` set, the startup
reconciler writes an image-owned named provider and, when no fallback is
configured, a backup entry:

```yaml
providers:
  finite-private:
    name: Finite Private
    base_url: <FINITE_PRIVATE_BASE_URL>
    key_env: FINITE_PRIVATE_API_KEY
    api_mode: chat_completions
    models:
      <FINITE_PRIVATE_MODEL>: {context_length: <FINITE_PRIVATE_CONTEXT_LENGTH>}
fallback_providers:
  - provider: finite-private
    model: <FINITE_PRIVATE_MODEL>
    base_url: <FINITE_PRIVATE_BASE_URL>
    key_env: FINITE_PRIVATE_API_KEY
    api_mode: chat_completions
```

- `providers.finite-private` is replaced on every normal start, so a hand edit
  there does not survive. Other providers are untouched. Its model entry adds
  `supports_vision: true` for `glm-5-3-flash` on the Finite Private product
  URL, as agentd's block does.
- `fallback_providers` is seeded only when neither it nor the legacy
  `fallback_model` exists and the Finite Private key is present (`.env` first,
  then the process environment). A user chain, an explicit `[]`, or a
  `fallback_model` wins. Later starts refresh only entries whose provider is
  `finite-private`, in place.
- `key_env` makes the entry resolve its own key. It never reads
  `OPENAI_API_KEY`. The literal `base_url` lets Hermes skip the entry when the
  primary is already the bare Finite Private block.
- `FINITE_PRIVATE_FALLBACK_MODE=remove` makes the next normal start delete the
  Finite-owned entries. The Runner does not set it.
- Finite Private serves no `/v1/models` list (404). The `models` map supplies
  context length and vision support. It does not validate:
  `/model <any-model> --provider finite-private` succeeds with Hermes's warning
  that it could not validate the model.
- `/model … --global` writes only `{default, provider}` and drops
  `context_length`, `base_url`, `api_mode` and `api_key` from the saved
  block. The reconciler does not put them back, because `model` is
  user-owned. Choosing Finite Private in Connections writes the full block
  again.

The launcher also unsets `OPENAI_API_KEY` when it equals
`FINITE_PRIVATE_API_KEY` (the Runner sets both to the same value), leaves a
different value alone, and sets
`CODEX_HOME=/dev/null/finite-codex-home-disabled` before it runs any Hermes
code.

### The session-route safety patch

`infra/images/patches/hermes-session-route-safety.patch` is applied to pinned
Hermes at image build, after the stop-generation patch. It closes two ways a
route could use another route's credential:

1. **OpenRouter borrowed `OPENAI_API_KEY`.** With no `OPENROUTER_API_KEY`,
   Hermes sent the Finite Private key (the Runner's alias), or a user's own
   OpenAI key, to OpenRouter. The patch removes `OPENAI_API_KEY` from
   OpenRouter's key candidates (`hermes_cli/runtime_provider.py`). An
   OpenRouter runtime with an empty or `${…}` key, as the saved default or a
   session override, is a credential failure: Hermes uses the fallback chain
   before any request, or fails the turn.
2. **A session override ran on another route's key.** A `/model` override
   whose credential was not cached was laid over the saved default's runtime,
   so, for example, a ChatGPT override with no sign-in sent the Finite Private
   key to the ChatGPT endpoint. The patch resolves the override's own provider
   instead (`gateway/run.py`). If that fails, or the resolved endpoint is not
   provably the one the override names, the turn uses the fallback chain or
   fails, and the next turn tries again. The override is never cleared and
   stays the conversation's choice. Endpoints compare scheme and host
   case-insensitively and everything else exactly, ignoring only a trailing
   `/`. An override with only a model still inherits the saved default's
   runtime.

With no key and no usable fallback, the user sees the gateway's generic error:
"⚠️ Provider authentication failed. Check the configured credentials; raw
provider details are in the gateway logs." The specific reason ("No OpenRouter
API key is configured for this agent, and no fallback model is available." for
the saved default, or "credentials for provider 'openrouter' are not
configured" for an override) reaches the gateway log only.

**Re-port the patch on every Hermes pin bump.** The build applies it with
`--fuzz=0`, so a moved hunk fails the build. A re-port that drops a hunk fails
`infra/images/test_hermes_session_route_safety.py` (CI step "Packaged Hermes
session route safety contract"). `OpenRouterBorrowingTests`,
`EndpointIdentityTests`, `SessionBoundaryTests` and the 120-case
`MatrixTests` hold the cases that fail on unpatched Hermes;
`NormalServingTests` guards ordinary routing. Two other tests pin upstream
behavior this integration relies on:
`infra/images/test_finite_inference_helper.py` (`PinnedUpstreamTests`, the helper's Hermes symbols) and
`finitechat/tests/hermes/test_inference_route_notice.py`
(`PinnedUpstreamWordingTests`, the fallback wording below).

### Backup notices

When the backup answers, the chat gets one note after the answer, for example
"Finite Private answered this response because OpenRouter is out of credits or
quota." The plugin's `_InferenceRouteObserver` records, per Finite
conversation and turn, which routes the main agent requested
(`pre_api_request`), which failed and why (`api_request_error`), and which
answered (`post_api_request`). On completion it sends at most one notice, and
only for what it observed:

| Observed in the turn | Notice |
| --- | --- |
| a non-Finite Private request failed, then Finite Private answered | "Finite Private answered this response because {route} {reason}." |
| a request failed, then another non-Finite Private route answered | "Your backup model answered this response because {route} {reason}." |
| the turn failed after a non-Finite Private failure and a Finite Private attempt | "{Route} and Finite Private couldn't answer this message." |
| anything else, including every turn only Finite Private served | none |

The route is OpenRouter, ChatGPT, or "your selected model"; without a known
reason the text says "after {route} returned an error". Finite Private is a
request to provider `custom`, `finite-private` or `custom:finite-private` at
`providers.finite-private.base_url`, so a user's own custom endpoint is never
called Finite Private. A request counts as the main agent's when its
`task_id` equals its `session_id`, it is not inside a delegated child, and the
session platform is Finite Chat or local; delegated subagents and the
background review fork are ignored. The notice is a `kind: message` with
`metadata.finite_notice = {v: 1, type, attempted, served_by, reason}` and no
`notify`, so old clients show it as an ordinary agent message. The adapter's
`send_or_update_status` drops Hermes's own "⚠️ Model fallback: …" and
"✅ Primary model restored: …" status lines, which would otherwise repeat on
every failing turn.

A notice is sent for each failure the plugin observes. **No notice is sent when
Hermes chooses the fallback before any request**, because nothing failed that
the plugin can see:

- a ChatGPT route that is signed out, or whose only credentials are in a
  usage-limit cooldown;
- an OpenRouter route with no key, as the saved default or an override;
- a session override whose endpoint binding the patch refuses;
- the provider cooldown on an agent Hermes keeps cached. After a billing or
  rate-limit error Hermes stops trying the primary for 60 seconds (doubling on
  consecutive errors, up to 4 hours). Hermes keeps the agent cached only when
  the backup's model ID equals the primary's; otherwise it evicts the agent,
  the next turn tries the primary again, and that failure gets its notice.

TODO: close these cases with an observed-facts notice
([FIN-129](https://linear.app/finitecomputer/issue/FIN-129)).

Recovery follows the same rule. An evicted agent tries the primary again on
the next turn; a cached agent tries it again only after the cooldown ends. No
notice is sent on recovery.

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
  `release`, which returns the entry to `Pending` for redelivery. A user
  `/stop`, `/new` or `/reset` instead `ack`s the cancelled turn and its held
  queued admissions; earlier undelivered entries are acked when delivered in that
  process. A lease older than the TTL (config, generous default) is swept back
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
  remains the only durable queue. Slash commands, pending approval responses, and pending
  clarification replies still reach the active turn immediately, and one busy
  session does not pause another. Text, photos, audio, video, and files each
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
