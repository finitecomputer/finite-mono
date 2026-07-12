# Hermes Sidecar Hardening Plan

## Problem Statement

The Finite Chat Hermes adapter should be boring in production: a human can join
from the iOS app, send messages and attachments, receive replies, survive local
process restarts, and avoid duplicate agent turns when acks or local bridge
calls are flaky.

This track is independent of Tinfoil. Tinfoil should only add deployment and
persistence constraints, not hide basic adapter defects.

For the repeated human canary flow "fresh Hermes instance -> Paul's physical
phone -> real multi-turn chat", use
[../../docs/hermes-phone-canary-loop.md](../../docs/hermes-phone-canary-loop.md).
That runbook is the promotion policy for local Mac, remote Docker, and Tinfoil.

## Acceptance Criteria

- Hermes 0.17 gateway can chat with Finite Chat iOS through the `finite`
  platform plugin.
- The same flow works through `finitechat hermes serve`, not only CLI-per-call.
- A handled inbound event is not dispatched twice in one adapter process even
  if the first ack fails.
- Poll failures back off and recover without disconnecting the gateway.
- Sidecar bridge calls are serialized against the local Finite Chat store.
- Restart/restore keeps the same agent npub, invite room, user membership, and
  decryptable outbound messages.
- Attachments, edits, typing/activity, receipts, room filters, and group rooms
  have focused tests. Agent-local attachments are bounded and promoted to
  encrypted durable blob references before append; path/read/upload failure
  appends no media row, and no local path survives in the room log.
- Docker acceptance proves the real runtime image can chat with iOS before any
  Tinfoil deployment attempt.

## Constraints

- Keep the Python adapter thin. Rust owns identity, MLS, cursors, encrypted
  storage, invite verification, attachment materialization, and bridge JSON.
- Keep one-shot polling only as a diagnostic/test tool; Finite Computer
  production requires the resident streaming sidecar and cursor-based reconnect.
- Do not require SaaS or Tinfoil to test chat correctness.
- Preserve CLI fallback only for compatibility environments that leave strict
  inbound streaming disabled.
- Prefer machine-readable JSON contracts over logs as the test oracle.

## RCA: 2026-06-26 Tinfoil Join Pending Failure

Symptom: the iOS app reached "Waiting for approval / waiting for room
admission" for `room-39a1162b6b515f8e`, and a throwaway CLI client reproduced
the same state against the live Tinfoil canary invite. That means scan/PIN
submission worked; the failing boundary was the owner-side agent admission loop.

Facts established:

- The live canary `finite-agent-tinfoil-user-canary-v022` was running Tinfoil
  package tag `v0.1.10` even though the container name suggested a later v022
  run.
- A local Hermes 0.17 gateway using the current `finitechat` plugin,
  `finitechat hermes serve`, and the same Tinfoil-shaped plugin install path
  (`plugins/finitechat` with `plugins.enabled: finitechat`) admitted a
  throwaway client in 3.7s via
  `scripts/hermes-real-gateway-admission-smoke.py`.
- The previous "Hermes media" and Docker smokes did not prove the exact
  Tinfoil path. The media smoke imported `adapter.py` directly and installed an
  echo handler. The Docker smoke ran `containers/agent/echo_agent.py`, not
  `hermes gateway run`. Those were useful bridge/runtime tests, but they were
  not a real Hermes gateway admission gate.
- The runtime `/status` endpoint exposed only coarse process readiness and
  invite presence. It did not expose image tag, finitechat/Hermes versions,
  plugin load status, gateway platform status, or an admission probe result.
- A simulator run through the visible app UI, with no launch auto-join flags,
  could scan/type an invite URL, enter a fresh PIN, reach the chat composer, and
  receive Hermes messages. With no provider env, Hermes returned a clear "No LLM
  provider configured" error through chat; with provider env loaded, the same
  path received a model reply.
- The iOS UI responsiveness issue had a separate root: several fire-and-forget
  actions, especially `setTyping` and composer sends, synchronously called
  `runtime.dispatch` on the main actor. These actions now use background
  dispatch so typing, taps, sends, and room read markers do not wait behind
  Rust/network work.

Final Tinfoil root cause, found after the first live canary probes still
failed: the supervised sidecar was launched as the `node` user with
`--ready-file /run/finite-agent/finitechat-hermes-ready.json`. The runtime
directory is root-owned, so `finitechat hermes serve` exited with
`Permission denied (os error 13)` after binding. Hermes was then not connected
to a live sidecar/admission loop. A token-gated `/finitechat-poll-once`
diagnostic proved the underlying Finite Chat owner state was healthy by
admitting the same pending join manually inside the container.

The fix in `finitecomputer` moved the ready file into the node-owned agent home
(`/home/node/.finite/agent/hermes-service-ready.json`) and added token-gated
diagnostics plus Hermes/sidecar log capture. Tinfoil canary release `v0.1.14`
then showed:

- `finitechat hermes serve` and `hermes gateway run --replace` both running in
  the Tinfoil container.
- `scripts/tinfoil_agent_admission_probe.sh --json --timeout-ms 60000`
  admitted a throwaway client in 2.1s.
- A CLI chat smoke joined the same live invite, ran `/sethome`, sent a message,
  and received the model reply `tinfoil chat ok`.
- A visible iOS Simulator run with no launch automation pasted the live
  `finite://join` URL, entered the current PIN, reached the composer, sent
  `Reply with exactly: simulator chat ok`, and received `simulator chat ok`.
  After relaunching the updated app, the same joined room persisted and a second
  visible composer send received `updated app ok`.

Before asking a human to try iOS again, run the live canary admission probe
from finitecomputer:

```bash
scripts/tinfoil_agent_admission_probe.sh --json --timeout-ms 30000
```

That probe fetches the running canary invite, creates a throwaway local
`finitechat` client, submits the current PIN, and fails unless the agent admits
the join. It would have caught this failure without involving the iOS app.

## RCA: 2026-06-26 Phase 5 Real Hermes/iOS Product Flow

Goal: prove the whole product surface locally before another Tinfoil attempt:
visible iOS Simulator UI, hosted Finite Chat server, a real Hermes 0.17 gateway
running on the Mac, the real `finitechat` plugin, a real invite/PIN join,
and multiple model-backed AI response turns. CLI state reads are allowed for
diagnostics only; they are not the UI oracle because they can open the same
store and perform sync work.

Root causes found:

- Invite admission could claim a stale KeyPackage for the same device instead
  of the exact KeyPackage uploaded with the current PIN-bound join request.
  The client now uploads the inline joiner KeyPackage and the owner claims by
  exact id/ref/hash/payload match before creating the Welcome.
- A permanently bad pending Welcome could poison later invite attempts. The
  client now clears permanent pending Welcome activation failures
  (`ParseWelcome`, `StageWelcome`, `ActivateWelcome`, `GroupAlreadyExists`) so
  a later valid Welcome can be processed.
- Swift command completion treated "any later runtime revision" as success
  after enqueueing a runtime action. Typing, read receipts, or sends could win
  that race before the requested `.startRuntime` sync had actually completed.
  Native callers now use exported `dispatchAndWait(action:)` on background
  tasks when they need completed action state.
- The runtime could persist a newly synced decrypted message to SQLite without
  reflecting that durable row in live `AppState.messages`. `runtime_tick` now
  reloads the durable chat projection after sync and reapplies visible outbox
  rows, so failed or undelivered local messages remain visible.
- Swift timeline groups used an id derived only from the first message in a
  group. Consecutive incoming messages from Hermes could update data without a
  row identity change that forced the native transcript to render. Group row ids
  now include first id, last id, and count.
- Long device IDs can break the Hermes inbound stream recipient limit
  (`recipient length ... exceeds max 128`). The local canary used a short
  device id (`p5h`) for the agent. Production should validate or derive bounded
  device IDs before runtime launch.
- The local proof used `GATEWAY_ALLOW_ALL_USERS=true` /
  `FINITECHAT_ALLOW_ALL_USERS=true` so the invited iOS user could drive Hermes.
  This is canary-only. Production needs finitechat room membership or an
  explicit per-user allowlist as the Hermes authorization source.

Final local proof:

- Runtime directory:
  `target/phase5-real-hermes-hosted/run-20260626-130008`
- Hosted server: `https://chat.finite.computer`
- Room: `room-07a72ff0b88b9819`
- Invite: `invite-fbc27d8906ce7ef7`
- Agent device: `p5h`
- iOS flow: Home -> Chats -> room -> composer send, with no launch automation
  flags and no CLI join.
- Hermes gateway log shows real model turns through
  `anthropic/claude-sonnet-4.6` via OpenRouter for:
  `phase five actual turn one ok`,
  `phase five actual turn two ok`,
  `phase five dispatch wait turn ok`, and
  `phase five durable projection ok`.
- The simulator UI rendered the incoming model bubbles for the final two
  post-fix turns, proving Rust state, encrypted local projection, Swift
  timeline grouping, and visible product UI agree.

## Phases

Each phase should run both vertically through the stack and horizontally across
quality attributes. Vertically means protocol/store -> Rust sidecar -> Python
adapter -> Hermes gateway -> iOS/CLI clients -> packaged runtime -> Tinfoil.
Horizontally means quality, reliability, simplicity, performance, and
understanding.

The operating rule is: whenever a phase changes behavior at one layer, run down
to the lower contract that should make it true, run up to the human workflow
that should benefit from it, and run across the five quality attributes before
calling the phase done. This keeps Tinfoil as the last validation environment
instead of the first place basic chat/runtime defects appear.

### Stack-Walk Operating Model

Every phase uses the same loop:

1. Run down the stack to the lowest contract that should make the behavior
   inevitable: protocol shape, durable store transition, sidecar JSON, adapter
   event mapping, runtime env, or object-storage state.
2. Run up the stack to the human workflow that should work because of that
   contract: local CLI chat, Hermes gateway chat, iOS chat, Docker packaged
   runtime, CI release artifact, or Tinfoil canary.
3. Run across the quality bar before closing the phase:
   quality, reliability, simplicity, performance, and understanding.

The phase is not done when one happy-path command works. It is done when the
lowest contract is tested, the highest relevant workflow is proven, the runtime
or release artifact emits machine-readable evidence, and the remaining risks
are written down with a clear owner.

### Phase Ladder

| Phase | Main question | Evidence required before moving on |
| --- | --- | --- |
| 1. Current polling bridge | Can the existing adapter be made correct and boring? | Adapter/unit regressions for redelivery, ack retry, poll failure, restart, media, edits, and plain messages. |
| 2. Rust sidecar as normal path | Can `finitechat hermes serve` own the bridge contract? | `/healthz`, `/readyz`, serialized bridge mutations, structured errors, and service fallback tests. |
| 3. Photon-style inbound stream | Can we reduce polling without losing correctness? | Feature-flagged NDJSON/SSE inbound path, reconnect/backoff tests, ack-after-dispatch semantics, poll fallback. |
| 4. Human E2E matrix | Does real chat work end to end for humans? | CLI and iOS smoke evidence for invite/PIN, first reply, media, edits, receipts, group identity, restart, and restore. |
| 5. Real Docker runtime | Does the packaged Linux image behave like the future Tinfoil runtime? | Docker smoke proving Hermes/plugin/binaries, encrypted backup, wipe, restore, same npub, `/healthz`, and post-restore chat. |
| 6. CI and release gates | Can we publish only what was proven? | CI artifacts for tests/smokes, S3-backed smoke, digest-pinned proven image, and fail-closed handoff report. |
| 7. Tinfoil canary last | Does Tinfoil add only TEE/runtime constraints? | Manual canary: restore from empty disk, chat, backup, restart, restore same npub, chat again, and classify any failure. |
| 8. Product convergence | Can hosted agents and docs stay in sync with shipped capability? | Skills/docs/update manifests for finitechat, Hermes, fsite, fbrain, runtime state, repair, rollback, and operator support. |

### Phase 1: Lock Down The Current Polling Bridge

- Quality: keep the existing polling adapter behavior green while adding
  focused regressions for redelivery, ack retry, transient poll failure,
  sidecar startup, service fallback, plain iOS messages, media, edits, and
  restart/restore.
- Reliability: prove a handled inbound event is not dispatched twice in one
  adapter process even when ack fails and the event is redelivered.
- Simplicity: keep the adapter thin and keep Rust responsible for crypto,
  cursors, local state, and bridge JSON.
- Performance: keep active-turn polling short and long-poll idle rooms instead
  of busy-looping.
- Understanding: document every failure mode as bridge input, expected JSON,
  observed output, and likely owner.

Current adapter regressions cover media payload mapping, inbound attachment
materialization into Hermes media fields, room filtering, redelivery dedupe,
ack retry without duplicate dispatch, transient poll recovery, service fallback,
NDJSON inbound stream consumption/fallback, outbound edit thread-route
preservation, transient service transport retry before CLI fallback, and
thread-scoped working activity set/clear routing. They also cover group-room
sender identity mapping and fail-closed handling for typed non-message stream
records such as receipts.

Protocol-hardening regression added after the missed second-turn incident:
`hermes_poll_recovers_messages_already_applied_by_runtime_sync` proves the
adapter does not treat the live bridge callback as authoritative. After Hermes
has acknowledged one user message and recorded its own cursor, the test sends
two later user messages, lets a separate Rust app runtime sync and persist them
first, then requires `finitechat hermes poll` to recover both ordered messages
from durable `client_app_events`, redeliver them until ack, and never replay
them after ack. This directly exercises the v1 protocol rule that streams and
pushes are only hints; ordered durable sync and local cursors are the
consistency boundary.

### Phase 2: Make The Rust Sidecar The Normal Runtime Path

- Quality: treat `finitechat hermes serve` as the primary path and CLI-per-call
  as fallback.
- Reliability: expose process health, readiness, version metadata, structured
  errors, and serialized bridge mutations against the local store.
- Simplicity: keep one loopback JSON contract and avoid leaking storage details
  into Python.
- Performance: avoid repeated CLI startup and repeated Python/Rust process
  churn during normal gateway operation.
- Understanding: make `/healthz`, `/readyz`, and bridge responses useful enough
  for Docker, Tinfoil, and human debugging.

Current service regressions cover ready-file startup, `/healthz`, `/readyz`,
serialized bridge actions behind the loopback service, home-channel actions,
NDJSON inbound failure handling, and machine-classifiable structured error
bodies for Hermes and usage failures.
The adapter also requires `/healthz` to pass after the ready file appears
before sending bridge actions to the service, closing the startup race that the
live media smoke exposed.

### Phase 3: Add A Photon-Style Inbound Stream

- Quality: use `GET /v1/hermes/inbound?room_id=...` behind a feature flag using
  NDJSON or SSE; keep `poll` only when that strict stream flag is disabled.
- Reliability: stream reconnects with backoff, preserves ack-after-dispatch
  semantics, and never drops events on sidecar restart.
- Simplicity: Rust owns one sync loop and Python consumes inbound events instead
  of scheduling its own polling loop.
- Performance: reduce idle polling overhead and lower message latency.
- Understanding: compare the design directly against Hermes Photon's sidecar
  pattern and record where Finite differs because of MLS/state constraints.

The current stream shape is an NDJSON endpoint guarded by
`FINITECHAT_HERMES_INBOUND_STREAM=1`. It emits `joined` and `event` records from
the Rust runtime's durable cursor machinery. For the Finite Computer production
profile this is no longer an optional intermediate mode: remove automatic
stream-to-poll and service-to-CLI fallback, reconnect with bounded backoff, and
prove cursor catch-up without duplicate dispatch.

### Phase 4: Build The Human E2E Matrix

- Quality: prove local CLI, Python adapter, Rust service, Hermes gateway, and
  Finite Chat iOS all agree on the same behavior.
- Reliability: cover invite/PIN, first chat, reply, edit, activity, attachment,
  read receipt, group sender identity, restart, restore, and second chat.
- Simplicity: keep each smoke command copy-pastable and make failures obvious
  from JSON or local stores, not screenshots.
- Performance: record latency for join, first inbound delivery, first reply,
  restore, and post-restore reply.
- Understanding: every E2E test should say which layer it proves and which
  lower layers it assumes are already green.

Current local smoke:

```bash
cargo run -q -p finitechat-rmp -- test ios-simulator --json
scripts/hermes-adapter-regression-report.py
scripts/hermes-sidecar-smoke.sh
scripts/hermes-agent-media-e2e.sh
scripts/hermes-real-gateway-admission-smoke.py
scripts/ios-hermes-agent-media-e2e.sh
```

The RMP simulator test command erases the dedicated simulator, runs the full
native `FiniteChat` test scheme with `.state/xcode-derived-data`, replaces its
explicit `.state/xcode-results/FiniteChatTests.xcresult`, and shuts the
simulator down after the run. This is the default unit-suite gate before any
visible app or Hermes E2E proof.
The smoke commands write `target/hermes-adapter-regressions/report.json`,
`target/hermes-sidecar-smoke/report.json`, and
`target/hermes-agent-media-e2e/report.json`.
`scripts/hermes-real-gateway-admission-smoke.py` writes
`target/hermes-real-gateway-admission-smoke/report.json`; its pass condition is
that Hermes 0.17 `gateway run --replace` admits a normal invite/PIN join through
the installed `finitechat` plugin with no direct adapter import and no echo
handler.
The iOS Simulator script writes
`target/ios-hermes-agent-media-e2e/report.json`; it requires a booted
simulator or `IOS_SIMULATOR_UDID`.
Together they prove finitechat-server, `finitechat hermes` CLI, encrypted
client stores, `finitechat hermes serve`, `/v1/hermes/inbound` NDJSON,
ack/drain, adapter redelivery/ack/fallback/filter/group/receipt regressions,
and native iOS app runtime plumbing. They do not, by themselves, prove the
production `hermes gateway run` path: the Hermes media smoke imports the
adapter directly and installs a test handler. The required additional local
gate is a real Hermes 0.17 gateway admission smoke where a user joins through
the invite/PIN flow and the agent's platform adapter admits the join without a
test handler.

### Phase 5: Prove The Real Runtime Image In Docker

- Quality: use the actual packaged image with the actual Hermes version,
  plugin layout, finitechat binary, fsite binary, env, and state directories.
- Reliability: backup encrypted agent state, wipe local container state,
  restore, unlock, and chat again with the same npub and room.
- Simplicity: keep the Docker harness close to the future Tinfoil entrypoint and
  avoid test-only bootstrap behavior.
- Performance: measure image startup, restore time, sidecar readiness time, and
  first reply after restore.
- Understanding: differences between local CLI, Docker, and future Tinfoil must
  be explicit in the runbook.

Current Docker smoke:

```bash
scripts/hermes-restic-preflight.py --report target/hermes-docker-smoke/restic-preflight.json
scripts/hermes-sidecar-docker-smoke.sh
scripts/hermes-sidecar-docker-s3-emulator-smoke.sh
```

It builds `containers/agent/Dockerfile` with `hermes-agent==0.18.2`, starts the
real Hermes gateway in Docker, drives `finitechat` CLI users through invite/PIN
admission before and after restore, and writes
`target/hermes-docker-smoke/report.json`. This proves the packaged Linux image
has the plugin files, binary, Hermes runtime dependency, real gateway command,
invite/PIN admission flow, restic encrypted repository init, entrypoint-owned
encrypted recovery-root backup on controlled shutdown, repository check, local
`/data` volume wipe, latest-by-tag snapshot restore into a fresh
volume/container, restored `/data/workspace` probe, same agent npub, same room,
runtime `/healthz`, and restored gateway admission. Echo replies are not
accepted as Docker runtime proof.
The smoke defaults to a local bind-mounted restic repository for CI, and can
point the same restic contract at S3-compatible object storage with
`FINITE_DOCKER_RESTIC_BACKEND=s3` plus a per-agent repository such as
`FINITE_DOCKER_RESTIC_REPOSITORY=s3:https://endpoint/bucket/agents/<agent-id>/state`,
`FINITE_DOCKER_RESTIC_PASSWORD`, and AWS-style credentials. The remaining
Phase 5 storage gap is running that S3 path against the actual Latitude bucket
and carrying the same env contract into the Tinfoil canary/runbook.
`scripts/hermes-sidecar-docker-s3-emulator-smoke.sh` starts a local MinIO
endpoint and runs the existing Docker smoke with `FINITE_DOCKER_RESTIC_BACKEND=s3`
against it. This proves the runtime/restic S3 path without real object-storage
credentials, but the audit marks it separately as `docker_runtime_s3_emulator_smoke`
and still requires a non-emulated S3 report for the actual Latitude/GitHub gate.
The restic preflight fails before the image build when S3 env is incomplete,
requires an explicit non-default backup encryption secret for remote repos, and writes a JSON
report that is uploaded alongside the Docker smoke report. The hardening audit
does not accept a status-only S3/preflight report: the Docker report must show
the entrypoint-created encrypted restic backup, matching repository metadata,
a latest-tagged snapshot rooted at the full `/data` recovery root, a restored
`/data/workspace` probe, and a non-emulated S3 backend; the
preflight report must show the derived `s3:` repository plus required password
and AWS credential env presence.
Passing this Hermes hardening audit is deliberately narrower than Agent Runtime
Recovery Readiness. Every generated smoke, handoff, canary, and audit report
keeps the application-consistent snapshot barrier, independently recoverable
key authority, and Core-owned service-consistent empty-target restore marked
`unproved`.
For local S3 runs, `scripts/hermes-sidecar-docker-smoke.sh` sources `.env` when
present, promotes `FINITE_DOCKER_RESTIC_AWS_*` values to the `AWS_*` names used
by restic, falls back to the standard AWS shared credentials/config profile
when the AWS env is still unset, and can derive
`FINITE_DOCKER_RESTIC_REPOSITORY` from `FINITE_LATITUDE_STORAGE_BUCKET`,
`FINITE_LATITUDE_OBJECT_ENDPOINT`, and `FINITE_DOCKER_RESTIC_PREFIX`.
For the current canary, the default prefix is
`agents/finite-agent-tinfoil-user-canary/state`; `target/hermes-docker-smoke`
is only a local/CI artifact path, not a storage prefix to deploy.
`.env.example` documents those fields without shipping secrets.
`scripts/hermes-publish-proven-image.py` then turns a passing smoke report into
a publish artifact by validating that the local Docker image id still matches
the proven `facts.image_id`, tagging that exact image, and optionally pushing
it to GHCR.

### Phase 6: Promote To CI And Release Gates

- Quality: make adapter unit tests, Rust integration tests, service contract
  tests, and runtime-image smoke checks part of the package/release story.
- Reliability: CI should fail before publishing an image that cannot chat,
  restore, or report readiness.
- Simplicity: keep CI outputs as artifacts and small JSON summaries instead of
  forcing humans to scrape logs.
- Performance: track build time and smoke latency so self-hosted runner work is
  driven by data.
- Understanding: each release should answer what changed, what was tested, and
  which runtime image digest was proven.

Current CI shape:

- `.github/workflows/ci.yml` pins the adapter test environment and runtime
  image build arg to `hermes-agent==0.18.2`.
- Every PR and `main`/`codex/**` push runs Rust fmt, clippy, workspace tests,
  the local Hermes sidecar smoke, Ruff, BasedPyright, and Python adapter tests.
- The local smoke uploads `target/hermes-sidecar-smoke/report.json` as a CI
  artifact so humans can inspect the exact invite/join/readiness/reply timings.
- The Docker runtime smoke runs on `main`, tags, or manual dispatch with
  `docker_smoke=true`; it uploads `target/hermes-docker-smoke/report.json` and
  `target/hermes-docker-smoke/restic-preflight.json`, plus the local encrypted
  restic repository when the default local backend is used. The report includes
  the local Docker image ID, image metadata, restic backend, restic snapshot
  metadata, repository metadata, encrypted backup flag, snapshot tag, and
  backup source for the image it proved. This is the current release-gate
  stand-in. Before generating the combined hardening audit, the Docker job
  downloads the sidecar smoke artifact from the Rust/Hermes job so
  `target/hermes-hardening-audit.json` reflects the full CI evidence chain
  instead of only the files produced inside the Docker job.
- Manual workflow dispatch with `publish_runtime_image=true` runs the Docker
  smoke first and requires `restic_backend=s3`. The workflow accepts either a
  full `restic_repository` input or derives the repository from
  `latitude_storage_bucket`, `latitude_object_endpoint`, and `restic_prefix`
  inputs/repository variables. It passes `FINITE_DOCKER_RESTIC_PASSWORD`,
  `FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID`,
  `FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY`, and optional
  `FINITE_DOCKER_RESTIC_AWS_SESSION_TOKEN` and
  `FINITE_DOCKER_RESTIC_AWS_REGION` repository secrets into the Docker smoke.
  `scripts/hermes-github-secrets-setup.py` can install those names from `.env`,
  process env, or the standard AWS shared credentials/config files without
  printing secret values. `scripts/hermes-github-ci-preflight.py` checks the
  GitHub secret and variable names before the slow workflow is started without
  reading secret values. `scripts/hermes-branch-publication-readiness.py`
  checks the local worktree before that slow path by classifying publishable
  source changes, blocking obvious generated or sensitive paths, reporting clean
  already-pushed worktrees distinctly from blocked ones, and printing the exact
  stage/commit/push commands when there are source changes to publish without
  mutating git.
  `scripts/hermes-github-publish-gate.py` then dispatches the manual workflow,
  watches it, downloads artifacts, and writes a local report so the S3
  smoke/publish gate can be driven from the repo rather than the Actions UI. On
  success it imports the downloaded reports into the canonical local
  `target/...` paths and refreshes the hardening audit. It refuses to dispatch
  from a dirty local worktree or a missing remote branch because GitHub Actions
  can only prove the pushed ref.
  The hardening audit rejects a placeholder publish-gate `{"status":"passed"}`
  report: it requires the dispatched run id/URL, successful watch/download
  exits, a local audit refresh, and copied canonical artifacts for the Docker
  smoke, restic preflight, image publish report, Tinfoil handoff, and canary
  summary.
  After the S3-backed smoke passes, it logs into GHCR, tags the exact
  `facts.image_id` from the passing smoke report as
  `ghcr.io/<owner>/finite-chat-hermes-runtime:<commit-sha>`, pushes it, and
  uploads `target/hermes-docker-smoke/image-publish.json`. The hardening audit
  rejects a placeholder image-publish `{"status":"published"}` report: it
  requires the source smoke report path, source image name/id, target ref,
  pushed flag, sha256 repo digest, and runtime proof tying the published image
  back to the S3-backed Hermes 0.17 Docker smoke.
- The same manual publish workflow then runs
  `scripts/hermes-tinfoil-handoff.py` to produce
  `target/hermes-docker-smoke/tinfoil-handoff.json`. That report fails closed
  unless the Docker smoke was S3-backed, the publish report is `published`, the
  source image id matches the proven smoke image id, and a registry digest is
  present. The hardening audit rejects a placeholder handoff
  `{"status":"ready"}`: it requires source report paths, the digest-pinned
  image, Hermes/restic runtime flags, S3 repository proof, latest restore
  selector, `finite-agent-state` restore tag, required secret names, and runtime
  container env.
- A ready handoff can be turned into a digest-pinned `tinfoil-config.yml`,
  Markdown runbook, and summary JSON with
  `scripts/hermes-tinfoil-canary-artifacts.py`. The generator refuses local,
  dry-run, non-S3, or non-digest-pinned reports.
- Unittest discovery covers the Tinfoil handoff fail-closed provenance checks
  plus the generated config/runbook contract consumed by the runtime entrypoint.
  The hardening audit rejects a placeholder `{"status":"ready"}` canary summary:
  it requires readable generated config/runbook files, a digest-pinned image,
  required restore/backup env, required secret names, and handoff digest match.
- `scripts/hermes-hardening-audit.py` reads the adapter-regression, sidecar,
  Hermes-agent media, iOS Simulator media, Docker, GitHub setup, GitHub
  publish-gate, preflight, publish, handoff, canary-artifact, and live-canary
  reports and emits a single evidence matrix. Use `--require-complete` only for
  the final gate where all native-client, S3, publish, and Tinfoil evidence is
  expected to exist.

### Phase 7: Tinfoil Canary Last

- Quality: Tinfoil should validate TEE/container integration, not discover
  basic chat correctness.
- Reliability: acceptance is start, unlock, restore, connect outbound via
  finitechat, chat once, backup, full restart, restore, and chat again.
- Simplicity: keep the first Tinfoil canary manually controlled from scripts and
  runbooks before adding SaaS/dashboard product flow.
- Performance: measure cold start, restore, readiness, and chat latency against
  the Docker baseline.
- Understanding: every Tinfoil-specific failure gets classified as image,
  runtime state, storage, network, attestation/secrets, or Tinfoil control
  plane.

Tinfoil canary runbook draft:

1. Run the local Docker smoke and keep `target/hermes-docker-smoke/report.json`
   as the baseline for image id, Hermes version, restic version, and latency.
2. Run the Docker smoke again with `FINITE_DOCKER_RESTIC_BACKEND=s3` against an
   isolated per-agent Latitude/restic prefix and an explicit canary
   `FINITE_DOCKER_RESTIC_PASSWORD`.
3. Publish only the image digest that passed the Docker S3 smoke, using
   `scripts/hermes-publish-proven-image.py --require-restic-backend s3` or the
   manual CI `publish_runtime_image=true` gate.
4. Use `target/hermes-docker-smoke/tinfoil-handoff.json` as the canary input:
   generate `target/hermes-docker-smoke/tinfoil-canary/tinfoil-config.yml` and
   `tinfoil-canary-runbook.md` with
   `scripts/hermes-tinfoil-canary-artifacts.py`.
5. Create one manually controlled Tinfoil container from the public config repo
   and release tag. The config should pin `image.digest`, expose `/healthz` on
   port 8080, set `FINITE_AGENT_RESTORE_ON_START=1`,
   `FINITE_AGENT_RESTORE_LATEST=1`, `FINITE_AGENT_BACKUP_ON_EXIT=1`,
   `FINITE_AGENT_RESTIC_REPOSITORY`, `FINITE_AGENT_RESTIC_BACKUP_TAG`,
   `FINITE_SERVER_URL=https://chat.finite.computer`,
   `FINITECHAT_HERMES_INBOUND_STREAM=1`, and AWS-style object-storage secrets.
6. Start from empty local disk, let the runtime entrypoint restore the latest
   restic snapshot tagged `finite-agent-state`, and fetch invite URL/PIN from
   the runtime endpoint. Before handing the invite to a human, run the
   owner-side admission probe:

   ```bash
   finitecomputer/scripts/tinfoil_agent_admission_probe.sh --json --timeout-ms 30000
   ```

   Only after that passes should a human join from Finite Chat, chat once and
   record the event ID, stop cleanly so the entrypoint writes a fresh backup,
   restart the container, restore latest by tag again, verify the same npub,
   and chat again with a second recorded event ID.
7. Write those observations to
   `target/hermes-docker-smoke/tinfoil-canary/container.json` and
   `target/hermes-docker-smoke/tinfoil-canary/health.json`, then build
   `target/hermes-docker-smoke/tinfoil-canary-evidence.json` with
   `scripts/hermes-tinfoil-canary-evidence.py`. Then run
   `scripts/hermes-tinfoil-canary-result.py --evidence-json target/hermes-docker-smoke/tinfoil-canary-evidence.json --report target/hermes-docker-smoke/tinfoil-canary-result.json`.
   The evidence builder records the source artifact paths plus the generated
   handoff/config expectations for container name, image digest, storage
   backend, and restore tag. The observed image digest and storage fields must
   come from container/health JSON or explicit operator observations; expected
   handoff values are not reused as observed runtime facts. The validator must
   pass before
   `scripts/hermes-hardening-audit.py --require-complete` can pass, and it
   rejects missing source artifacts, mismatched expectations, or chat claims
   without concrete event IDs.
8. Treat `FINITE_AGENT_RESTIC_PASSWORD` as a temporary canary secret, not the
   production privacy posture. Today the entrypoint passes this value directly
   to restic as `RESTIC_PASSWORD`; no user-key derivation happens in the
   container. The product path should derive or unwrap a per-agent backup key
   from user-controlled key material, with domain separation from finitechat
   identity keys, so object storage and operators cannot decrypt agent state
   without user-mediated or attestation-gated key release.
9. If Tinfoil fails after the Docker S3 smoke is green, classify the failure as
   image pull, runtime state, object storage, network, secrets/unlock,
   attestation, or Tinfoil control plane before changing application code.

### Phase 8: Turn Learning Into Product Shape

- Quality: update shipped skills/docs so hosted agents know exactly what they
  can do in the runtime they are actually running.
- Reliability: define update, rollback, and repair paths for finitechat,
  Hermes, fsite, fbrain, skills, and runtime state.
- Simplicity: converge `fsite`, `fchat`, and `fbrain` on common command shapes,
  JSON output, idempotency, and auth setup.
- Performance: avoid making every hosted agent carry services or binaries it
  does not need.
- Understanding: keep the live curriculum and operator docs synced to the
  product we have, not the product we hope to have later.

## Evaluation Design

- Unit tests assert JSON boundaries: Hermes `MessageEvent` fields, bridge
  payloads, retryability, acks, and service readiness.
- Rust integration tests use a live local Finite Chat server and real encrypted
  client stores.
- iOS simulator tests prove the native client can decrypt agent replies.
- `scripts/hermes-real-gateway-admission-smoke.py` proves real Hermes 0.17
  owner-side invite admission before Docker or Tinfoil.
- Docker tests prove the packaged runtime has the right Hermes version,
  plugin layout, binaries, env, state directories, and native iOS chat path.
- Restart/restore tests wipe local container state, restore encrypted backup,
  and chat again using the same invite room.
- Tinfoil canary is last. It should validate TEE/container integration, not be
  the first place chat correctness is discovered.
- Live Tinfoil canary evidence must be normalized by
  `scripts/hermes-tinfoil-canary-result.py`; a hand-written
  `{"status":"passed"}` result is not enough. The normalized result must
  preserve raw source artifact references, match the generated handoff
  expectations, include observed image/storage sources, and include concrete
  before/after chat event IDs.
- The hardening audit must report `complete` before this track is considered
  done. Local-only smoke evidence is not enough to satisfy the Tinfoil objective.

## Streaming Sidecar Shape

The target sidecar contract should look like:

- `GET /healthz`: process health and version metadata.
- `GET /readyz`: store open, account loaded, server reachable enough to sync.
- `POST /v1/hermes/send|edit|activity|ack|recover|invite|pin`: serialized
  bridge mutations.
- `GET /v1/hermes/inbound?room_id=...`: newline-delimited JSON or SSE stream
  of `HermesPollEventV1` values plus join notifications.

The adapter reads inbound events from the stream, dispatches to Hermes, acks
after successful dispatch, and reconnects with bounded backoff. With
`FINITECHAT_HERMES_INBOUND_STREAM=1`, a stream failure must never select the
Python polling loop or a per-action CLI subprocess; reconnecting to the Rust
service performs durable-cursor catch-up.
