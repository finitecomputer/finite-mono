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
Hosted Web Device, Electron, or a native client starts the room independently.

## Native Hermes specialization profiles

Finite Chat conveys authenticated attachments to Hermes without choosing a
model, rewriting the channel prompt, or registering Finite-specific agent
tools. Specializations are runtime configuration behind Hermes's existing
tools. For example, an `auxiliary.vision` profile can route Hermes's built-in
`vision_analyze` and `video_analyze` tools to the AEON worker while the main
model remains responsible for deciding whether those tools are useful.

```yaml
auxiliary:
  vision:
    base_url: https://inference.example/v1
    api_key: ${AEON_API_KEY}
    model: aeon-gemma-4-12b-k4-nvfp4-unified-fast
    timeout: 120
```

The same rule applies to other specialization families: prefer a model or
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
scripts/hermes-adapter-regression-report.py
scripts/hermes-sidecar-smoke.sh
scripts/hermes-agent-media-e2e.sh
scripts/ios-hermes-agent-media-e2e.sh
```

The adapter regression command writes
`target/hermes-adapter-regressions/report.json` and proves focused Python
adapter behavior for plain messages, redelivery, ack retry, poll recovery,
sidecar startup/fallback/serialization, media, edits, typing activity, room
filters, group sender identity, receipt/control stream filtering, and stream
fallback.
The script writes `target/hermes-sidecar-smoke/report.json` with timings for
server startup, Welcome-first room admission, sidecar readiness, inbound
delivery, ack/drain, agent reply, and user decrypt.
The media E2E writes `target/hermes-agent-media-e2e/report.json` and runs the
real `hermes-agent` package against the Finite plugin with the sidecar inbound
stream enabled. It proves an image sent by a Finite Chat user reaches Hermes as
media and that the user decrypts both text and image replies from the agent.
Agent-local reply paths are never written into the room log: the Rust sidecar
uses the contract above and appends only the durable encrypted blob reference.
It installs an echo callback, so it is adapter transport coverage, not a real
Hermes model or gateway acceptance gate.
The canonical real-gateway acceptance is the monorepo
`just dev saas-smoke` path. It packages Hermes 0.18.2 and this plugin in the
one Runtime image and requires model-backed replies across independent
chat-server, Hosted Web Device, and Runtime restarts.
The iOS Simulator E2E writes
`target/ios-hermes-agent-media-e2e/report.json`, drives the native app through
the product harness, and proves that the app's encrypted local store contains
the adapter text and image replies. It is still echo-handler transport coverage
and requires a booted simulator or `IOS_SIMULATOR_UDID`.
The physical-device variant is `scripts/ios-device-hermes-agent-media-e2e.sh`;
it writes `target/ios-device-hermes-agent-media-e2e/report.json` after pulling
the app's store from an installed, unlocked phone.

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

For the remote Docker human-handoff canary on `finite-lat-2`:

```bash
scripts/hermes-remote-docker-canary.py --keep-running
```

That script is the remote-Docker equivalent of
`scripts/hermes-phone-canary.py`: it requires a passed local phone report by
default, builds the real image on the remote Docker daemon, runs against
`https://chat.finite.computer`, proves invite/PIN admission and real Hermes
model replies before and after entrypoint backup/restore, then prints the
stable invite URL plus current rotating PIN only after the restored container
is alive. The report lands under
`target/hermes-phone-canary/remote-docker/<run-id>/report.json`.

By default the restic repo is a local bind mount under
`target/hermes-docker-smoke/restic-repo`. To run the same smoke against
S3-compatible storage such as Latitude, provide an isolated repository prefix
and AWS-style credentials:

```bash
FINITE_DOCKER_RESTIC_BACKEND=s3 \
FINITE_DOCKER_RESTIC_REPOSITORY=s3:https://objects.nyc.storage.sh/YOUR_BUCKET/agents/finite-agent-tinfoil-user-canary/state \
FINITE_DOCKER_RESTIC_PASSWORD='temporary-canary-backup-secret' \
AWS_ACCESS_KEY_ID='...' \
AWS_SECRET_ACCESS_KEY='...' \
scripts/hermes-sidecar-docker-smoke.sh
```
To exercise the same restic S3 code path before Latitude credentials are wired,
run `scripts/hermes-sidecar-docker-s3-emulator-smoke.sh`. It starts a local
MinIO service, writes `target/hermes-docker-s3-emulator-smoke/report.json`, and
marks the report as `s3_endpoint_kind=local_emulator`. The hardening audit
accepts that as local S3-compatible rehearsal evidence but rejects it as the
real Latitude/GitHub S3 gate.
`FINITE_DOCKER_RESTIC_PASSWORD` must be explicitly set for the S3 backend and
must not be the disposable local smoke default. For this canary it is a
temporary backup encryption secret. The product path should derive or unwrap a
per-agent backup key from user-controlled key material so object storage and
operators cannot decrypt agent state without user-mediated key release.
For local runs, copy `.env.example` to `.env`; the smoke wrapper sources it
before preflight and promotes `FINITE_DOCKER_RESTIC_AWS_*` values to the
`AWS_*` names restic expects. If those values are still unset, it reads the
standard AWS shared credentials/config files using `AWS_PROFILE` or the
`default` profile. Set `FINITE_DOCKER_RESTIC_USE_AWS_SHARED_CONFIG=0` to
disable that fallback. If `FINITE_DOCKER_RESTIC_REPOSITORY` is empty, the
wrapper can derive it from `FINITE_LATITUDE_STORAGE_BUCKET`,
`FINITE_LATITUDE_OBJECT_ENDPOINT`, and `FINITE_DOCKER_RESTIC_PREFIX`.

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
  --preflight-report target/hermes-docker-smoke/restic-preflight.json \
  --publish-report target/hermes-docker-smoke/image-publish.json \
  --handoff-report target/hermes-docker-smoke/tinfoil-handoff.json
```

It fails unless the smoke used `restic_backend=s3`, the image was actually
published, and the published source image id matches the image proven by the
Docker smoke.
The handoff's restore section uses the runtime env names consumed by
`/opt/agent-entrypoint.sh`: `FINITE_AGENT_RESTORE_ON_START=1`,
`FINITE_AGENT_RESTORE_LATEST=1`, `FINITE_AGENT_BACKUP_ON_EXIT=1`,
`FINITE_AGENT_RESTIC_REPOSITORY`, `FINITE_AGENT_RESTIC_BACKUP_TAG`, and
`FINITE_AGENT_RESTIC_PASSWORD`. The generated Tinfoil config must point at the
same per-agent restic repository proven by the S3-backed Docker smoke; it must
not point at emulator buckets or local artifact paths.

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
durable cursors, and storage. The service surface covers inbound stream,
acknowledge, send/edit, activity, recovery, and explicit home-channel state;
strict hosted mode never falls back to Python polling or per-message CLI
subprocesses.
