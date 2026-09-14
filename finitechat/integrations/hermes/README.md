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

Plugin 0.4.2 requires this monorepo's pinned Hermes runtime with delivery
contract v1. An unmodified upstream Hermes install does not provide that
contract. From the monorepo root, `nix run .#hermes-agent -- gateway start`
runs the supported runtime; the full and minimal packages and CI all compile
the same checked-in `delivery-contract.patch` into the Hermes wheel.

Select the patched Hermes runtime first, then update the embedded plugin.
Before replacing a working plugin, preflight that exact runtime's interpreter
from the repository root:

```bash
nix shell .#hermes-agent-runtime-python -c python3 -c \
  'from gateway.platforms.base import BasePlatformAdapter; assert BasePlatformAdapter.DELIVERY_CONTRACT_VERSION == 1'
```

`finitechat hermes install`
only copies the plugin; it does not upgrade Hermes. A new plugin on an old
runtime fails clearly before recovery or inbox consumption. Upgrade the
runtime to restore service, or reinstall the previous plugin with the previous
FiniteChat binary. `install --force` can replace a working old plugin without
checking which interpreter the next gateway boot will use, so doing that
before the runtime upgrade can interrupt availability. Do not reset the Agent
Home or chat stores. An old
plugin remains compatible with the patched runtime, retaining its old behavior.

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

# 2. The plugin (requires the paired Finite Hermes runtime above)
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
set. Compatibility mode can use the CLI-per-call bridge when no service is
configured. Once a service request has been attempted it never switches
transports or resubmits the mutation.
The Finite Computer production runtime sets `FINITECHAT_HERMES_INBOUND_STREAM=1`
and treats the resident `GET /v1/hermes/inbound` NDJSON path as mandatory.
In that strict mode, stream failures reconnect with bounded backoff and resume
from the Rust service's durable cursor. They never fall into Python timer
polling or CLI-per-message subprocess calls. One-shot polling and CLI fallback
remain available only when inbound streaming is disabled. CLI-only polling is
capped at one second so its serialization lock cannot delay outgoing replies
for a long poll; this trades more idle CLI calls for bounded reply latency.
Resident service/stream timing is unchanged.

## Inbox in-flight state and reply routing live in Rust

The Rust sidecar owns the chat delivery contract end to end (ownership audit
O1/O2); the Python adapter keeps no route table, dedup set, or SQLite state of
its own.

- **In-flight state (O1).** Each inbox entry carries a lease: `Pending` or
  `Leased`. The stream / `poll` / `inbound` deliver only deliverable entries and
  flip them to `Leased`, so a leased entry is not re-emitted on the next tick.
  The adapter settles the lease from the turn: the completion hook `ack`s on
  success or failure, and a cancelled turn calls `release`, which returns the
  entry to `Pending` for redelivery. A lease older than the TTL (config,
  generous default) is swept back to `Pending`, so a crashed turn cannot strand
  an entry. The sidecar keeps a bounded recently-acked ring, so a post-restart
  duplicate ack is a no-op and an already-acked entry is never redelivered —
  idempotency the adapter no longer has to provide. Existing `hermes-inbox.json`
  entries load as `Pending` (`#[serde(default)]`), so the on-disk format is
  unchanged.
- **Busy-session admission.** Hermes returns an explicit receipt: `started`
  leaves settlement to its completion hook; `consumed` means an inline control
  completed; `rejected` means a terminal refusal; `deferred` has accepted
  nothing. A deferred receipt includes a readiness awaitable owned by Hermes.
  The adapter holds one ephemeral head per session to preserve queue order,
  with later messages released back to the Rust inbox. The head remains
  unaccepted durable inbox state, never a second acceptance record. Cancelling
  its wait cannot cancel the running turn. Hermes owns command, approval and
  clarification recognition; a previously deferred head cannot be reclassified
  as an answer to a prompt created later. The adapter never reads Hermes's
  private session/queue dictionaries or polls a task every 50ms.
- **Reply/edit routing (O2).** Every inbound event already carries its
  conversation and segment ids, and the sidecar mints `thread_id` from them. On
  send/edit/activity the adapter passes that `thread_id` back, and the sidecar
  resolves it against its own agent store into the concrete route. An `edit`
  with no route fields is resolved by looking the original message up by
  `(room_id, message_id)`. An explicit Topic/Chat route still wins as an
  override; an unknown thread id falls back to the Home default with a loud
  warning (an archived topic must never silently consume a message). There is
  no policy switch: the fallback is the only behaviour.
- **Outbound attempts.** The adapter makes one request per send/edit/activity;
  it never retries a lost response or falls back to another transport after
  an attempt. Hermes honors the typed delivery disposition and cannot retry,
  rewrite the response as a formatting fallback, or emit a second media
  delivery notice. Error kind and retryability from a service refusal remain
  available for diagnostics, but do not authorize an automatic resend. A lost,
  malformed or empty send response is `unknown`, never success. Even a service
  error after mutation can follow acceptance, so the adapter conservatively
  reports `unknown` unless it rejected the operation locally before sending.
- **No second outbound queue.** Hermes does not register FiniteChat responses
  in its delivery-obligation ledger. Existing ledger rows remain inspectable
  under its normal retention rules but are not automatically replayed through
  FiniteChat. Completed input is acknowledged even when response delivery is
  unknown: re-running tools would not recover that response safely. An operator
  can inspect Hermes's transcript and any already delivered chat messages to
  decide whether an explicit new send is appropriate. This does not promise
  exactly-once delivery or automatic response recovery after a crash.

The Rust inbox format, service/CLI requests, Room bindings and admission
policies are unchanged. Existing pending/leased entries remain readable;
interrupted or abandoned input still recovers through the existing release/
lease-expiry contract. A crash after processing but before inbox acknowledgement
can still replay input, and a lease expiring during a very long turn can still
redeliver it; this change introduces no new database or exactly-once claim.
Token streaming remains disabled for the append-only transport; separate
interim commentary remains supported. Other Hermes platforms retain their
existing queue, retry, streaming and delivery-ledger behavior. No Hosted Web
Device component gains state or authority.

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
second reply. The older restic/remote-Docker scripts are historical recovery
experiments, not current promotion gates; Recovery Snapshot design remains an
explicit TODO.

### Parked Recovery Experiments

Everything below this heading is retained for recovery/Tinfoil archaeology.
The commands use the retired invite/PIN flow and no longer match the current
workflow inputs or release path. Do not use them as a product canary or publish
gate until they are rewritten for Agent Principal + Welcome-first admission
and the Recovery Snapshot design is explicitly resumed.

The remote Docker canary (`scripts/hermes-remote-docker-canary.py`) and the
sidecar Docker smoke wrappers (`scripts/hermes-sidecar-docker-smoke.sh`,
`scripts/hermes-sidecar-docker-s3-emulator-smoke.sh`,
`scripts/hermes-build-runtime-image.py`) were deleted in ownership audit O12.
They built a drifting `containers/agent/Dockerfile` test fixture that was
never shipped and drove the removed `finitechat hermes invite`/`join`
admission commands, so they could not pass against any image. The restic
entrypoint contract they exercised is still unit-tested in
`tests/container/test_agent_entrypoint.py`; a Docker-level restic proof, if
resumed, must run against the canonical image built by
`finitecomputer-v2/scripts/build_runtime_image.py` with Welcome-first
admission.

To publish the exact local image proven by a passing Docker smoke report:

```bash
scripts/hermes-publish-proven-image.py \
  --report target/hermes-docker-smoke/report.json \
  --image-ref ghcr.io/finitecomputer/finite-chat-hermes-runtime:canary \
  --publish-report target/hermes-docker-smoke/image-publish.json
```

Add `--push` only after logging in to the registry. In GitHub Actions this is
available as the manual `publish_runtime_image` input, which pushes the image
ID recorded in the passing smoke report and uploads `image-publish.json`.
Publishing requires the manual `restic_backend=s3` path. The workflow can use a
full `restic_repository` dispatch input, or derive it from
`latitude_storage_bucket`, `latitude_object_endpoint`, and `restic_prefix`.
For hands-free dispatches, set these repository variables or secrets:

- `FINITE_LATITUDE_STORAGE_BUCKET`
- `FINITE_LATITUDE_OBJECT_ENDPOINT`, optional when using the default
  `https://objects.nyc.storage.sh`
- `FINITE_DOCKER_RESTIC_PREFIX`, optional when using the default
  `agents/finite-agent-tinfoil-user-canary/state`

Configure these repository secrets before using the publish gate:

- `FINITE_DOCKER_RESTIC_PASSWORD`
- `FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID`
- `FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY`
- `FINITE_DOCKER_RESTIC_AWS_SESSION_TOKEN` if using temporary credentials
- `FINITE_DOCKER_RESTIC_AWS_REGION` if the provider requires one

If the values are present in `.env`, the current process environment, or the
standard AWS shared credentials/config files, install the GitHub
secrets/variables without printing secret values:

```bash
scripts/hermes-github-secrets-setup.py \
  --repo finitecomputer/finitechat \
  --env-file .env \
  --apply
```

Pass `--aws-profile PROFILE` when the object-storage key lives outside the
default AWS profile.
Omit `--apply` for a redacted dry run. Existing GitHub secret or variable names
are preserved, so the script only requires local values for names that are not
already configured remotely.

Check that the GitHub-side names are configured before starting the slow manual
publish workflow:

```bash
scripts/hermes-github-ci-preflight.py --repo finitecomputer/finitechat
```

This writes `target/hermes-github-ci-preflight.json` and reports missing secret
or variable names without reading or printing secret values.

Before dispatching CI, check that the local branch is publishable:

```bash
scripts/hermes-branch-publication-readiness.py \
  --branch codex/hermes-sidecar-hardening
```

That writes `target/hermes-branch-publication-readiness.json`, classifies the
source files that should be staged, blocks obvious generated or sensitive paths
such as `.env`, `target/`, caches, keys, and database files, and prints the
exact `git add`, `git commit`, and `git push` commands when there are source
changes to publish. A clean worktree reports `status: clean` instead of
`blocked`, because that means there is nothing local to stage. It does not
stage, commit, or push anything.

Once preflight is green, run the S3 smoke, publish, handoff, canary-artifact
generation, and artifact download path from one local command:

```bash
scripts/hermes-github-publish-gate.py \
  --repo finitecomputer/finitechat \
  --ref codex/hermes-sidecar-hardening
```

That command dispatches the manual CI workflow with `docker_smoke=true`,
`publish_runtime_image=true`, and `restic_backend=s3`, watches the run, downloads
artifacts into `target/hermes-github-publish-gate/artifacts`, and writes
`target/hermes-github-publish-gate/report.json`. On a passing workflow it also
copies the downloaded reports back into their canonical `target/...` paths and
reruns `scripts/hermes-hardening-audit.py`, so local audit state reflects the CI
evidence. Use `--dry-run` to inspect the exact `gh` commands without starting
the workflow. The non-dry-run path fails before dispatch when the local worktree
has uncommitted changes or the requested branch does not exist on GitHub; CI can
only prove the pushed ref, not local edits.

After publish, build the redacted Tinfoil handoff report:

```bash
scripts/hermes-tinfoil-handoff.py \
  --smoke-report target/hermes-docker-smoke/report.json \
  --publish-report target/hermes-docker-smoke/image-publish.json \
  --handoff-report target/hermes-docker-smoke/tinfoil-handoff.json
```

It fails unless the smoke used `restic_backend=s3`, the image was actually
published, and the published source image id matches the image proven by the
Docker smoke.
The handoff's restore section still carries the historical
`FINITE_AGENT_RESTIC_*` env names; the entrypoint restic contract they targeted
has been retired (no production launcher ever set it), and off-host retirement
storage is the Borg lane (`docs/runs/runtime-retirement-readiness.md`). The
generated Tinfoil config must point at the same per-agent restic repository
proven by the S3-backed Docker smoke; it must not point at emulator buckets or
local artifact paths.

After a ready S3/published handoff, generate the Tinfoil canary config and
runbook:

```bash
scripts/hermes-tinfoil-canary-artifacts.py \
  --handoff-report target/hermes-docker-smoke/tinfoil-handoff.json \
  --output-dir target/hermes-docker-smoke/tinfoil-canary \
  --config-repo finitecomputer/tinfoil-agent-runtime-canary \
  --tag v0.1.0
```

The generated `tinfoil-config.yml` pins the published image digest, exposes
`/healthz` on port 8080, and restores the latest restic snapshot tagged
`finite-agent-state` so clean shutdown backups become the next restore point.
This canary still uses Tinfoil secrets for the restic password and storage
credentials; that validates plumbing, not the final user-mediated key-release
privacy posture.

After the live Tinfoil canary has been run, save the observed Tinfoil container
JSON and runtime health JSON, then build a local evidence file:

```bash
scripts/hermes-tinfoil-canary-evidence.py \
  --handoff-report target/hermes-docker-smoke/tinfoil-handoff.json \
  --canary-summary target/hermes-docker-smoke/tinfoil-canary/tinfoil-canary-summary.json \
  --container-json target/hermes-docker-smoke/tinfoil-canary/container.json \
  --health-json target/hermes-docker-smoke/tinfoil-canary/health.json \
  --image-digest '<digest-observed-from-tinfoil-container-json>' \
  --storage-backend s3 \
  --restore-tag finite-agent-state \
  --chat-before-message-id '<finite-chat-event-id-before-restart>' \
  --chat-after-message-id '<finite-chat-event-id-after-restart>' \
  --backup-observed \
  --restore-observed \
  --evidence-json target/hermes-docker-smoke/tinfoil-canary-evidence.json
```

Then normalize it into the only runtime result accepted by the hardening audit:

```bash
scripts/hermes-tinfoil-canary-result.py \
  --evidence-json target/hermes-docker-smoke/tinfoil-canary-evidence.json \
  --report target/hermes-docker-smoke/tinfoil-canary-result.json
```

That validator fails unless the evidence preserves the raw handoff, summary,
container, and health source artifact references; the canary used the generated
handoff expectations for container name, digest-pinned image, S3 restic state,
and restore tag; the observed image digest and storage fields are sourced from
container/health JSON or explicit operator observations; a running Tinfoil
container; `/healthz` readiness with the restored npub; concrete Finite Chat
event IDs before and after restart; an observed clean-stop backup; an observed
latest-by-tag restore; and the same agent npub after restore.

To see exactly which hardening gates are proven by the reports on disk:

```bash
scripts/hermes-hardening-audit.py --report target/hermes-hardening-audit.json
```

The audit also reads `target/hermes-adapter-regressions/report.json`,
`target/hermes-github-secrets-setup.json`, and
`target/hermes-github-publish-gate/report.json` so missing adapter coverage,
GitHub secrets, dirty local worktrees, and missing remote branches show up
before the S3 evidence exists. It also requires
`target/ios-hermes-agent-media-e2e/report.json` for the Phase 4 native-client
gate; this is intentionally manual/local because CI does not currently boot the
Finite Chat iOS harness. In CI, the Docker runtime job downloads the sidecar
smoke and adapter regression artifacts from the Rust/Hermes job before
generating the audit, so the uploaded audit reflects both local adapter/sidecar
contracts and the packaged-runtime proof. Add
`--require-complete` only when the S3-backed smoke, published digest, handoff,
generated canary artifacts, iOS Simulator media E2E report, and live Tinfoil
canary result are all expected to be present.

## How the pieces divide (ADR 0002)

The Python adapter stays thin and talks to the resident loopback Finite Chat
service. The Rust binary owns identity, MLS encryption, Welcome processing,
durable cursors, storage, inbox in-flight state (leases), and reply/edit route
resolution. The service surface covers inbound stream, acknowledge, release,
send/edit, activity, recovery, and explicit home-channel state; strict hosted
mode never falls back to Python polling or per-message CLI subprocesses.
