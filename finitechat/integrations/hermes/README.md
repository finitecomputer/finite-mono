# Hermes ⇄ Finite Chat

The `finitechat` plugin connects a [Hermes agent](https://github.com/NousResearch/hermes-agent)
to end-to-end-encrypted Finite Chat rooms. The dream flow (ADR 0006):

1. The agent prints a QR code, a `finite://join?...` URL, and a rotating
   6-digit PIN when the gateway starts.
2. You scan or paste it into the Finite Chat app and type the PIN.
3. The agent verifies the PIN proof *before* admitting you to the MLS group
   — then you're chatting, end-to-end encrypted, with MLS-authenticated
   sender identities. No public relay, no account registration: the agent's
   npub lives only on its home server.

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

Then `hermes gateway start` prints the invite QR/URL/PIN and the agent is
reachable from the Finite Chat app.

`finitechat hermes install` writes the embedded `finitechat` plugin into
`$HERMES_PLUGINS_DIR/finitechat`, `$HERMES_HOME/plugins/finitechat`, or
`~/.hermes/plugins/finitechat`. It also writes a local `finitechat.env` file with
the Agent Home and binary path. The plugin treats that file as defaults only:
explicit Hermes config and process environment still win.
Pass `--service-url URL` to also write `FINITECHAT_HERMES_SERVICE_URL` for a
supervisor-managed `finitechat hermes serve` process.

For the supervised Rust bridge work, `finitechat hermes serve` starts the
loopback service boundary and exposes `GET /healthz` plus `GET /readyz`. The
current plugin still starts that service itself when no
`FINITECHAT_HERMES_SERVICE_URL` is set, and falls back to the CLI-per-call
bridge when the service is unreachable.
Set `FINITECHAT_HERMES_INBOUND_STREAM=1` to make the adapter consume the
sidecar's experimental `GET /v1/hermes/inbound` NDJSON long-poll endpoint
instead of POSTing `poll`; transport failures fall back to the existing poll
path.
See [HARDENING.md](./HARDENING.md) for the adapter reliability plan and
acceptance matrix.
See [../../docs/hermes-phone-canary-loop.md](../../docs/hermes-phone-canary-loop.md)
for the physical-phone quality loop that promotes local Hermes, remote Docker,
and Tinfoil only after lower-layer evidence is green.

For a local human smoke with JSON evidence:

```bash
scripts/hermes-adapter-regression-report.py
scripts/hermes-sidecar-smoke.sh
scripts/hermes-agent-media-e2e.sh
scripts/hermes-real-gateway-admission-smoke.py
scripts/ios-hermes-agent-media-e2e.sh
```

The adapter regression command writes
`target/hermes-adapter-regressions/report.json` and proves focused Python
adapter behavior for plain messages, redelivery, ack retry, poll recovery,
sidecar startup/fallback/serialization, media, edits, typing activity, room
filters, group sender identity, receipt/control stream filtering, and stream
fallback.
The script writes `target/hermes-sidecar-smoke/report.json` with timings for
server startup, invite/join, sidecar readiness, inbound delivery, ack/drain,
agent reply, and user decrypt.
The media E2E writes `target/hermes-agent-media-e2e/report.json` and runs the
real `hermes-agent` package against the Finite plugin with the sidecar inbound
stream enabled. It proves an image sent by a Finite Chat user reaches Hermes as
media and that the user decrypts both text and image replies from the agent.
It installs an echo callback, so it is adapter transport coverage, not a real
Hermes model or gateway acceptance gate.
The real gateway admission smoke writes
`target/hermes-real-gateway-admission-smoke/report.json` and proves Hermes 0.17
`gateway run --replace` loads the installed `finitechat` plugin and admits
a normal invite/PIN join without a test callback.
The iOS Simulator E2E writes
`target/ios-hermes-agent-media-e2e/report.json`, drives the native app through
the product harness, and proves that the app's encrypted local store contains
the adapter text and image replies. It is still echo-handler transport coverage
and requires a booted simulator or `IOS_SIMULATOR_UDID`.
The physical-device variant is `scripts/ios-device-hermes-agent-media-e2e.sh`;
it writes `target/ios-device-hermes-agent-media-e2e/report.json` after pulling
the app's store from an installed, unlocked phone.

Validate the restic backup environment before the longer Docker smoke:

```bash
scripts/hermes-restic-preflight.py --report target/hermes-docker-smoke/restic-preflight.json
```

The preflight writes JSON, fails before any expensive image build when required
S3 env is missing, and redacts URL userinfo from the repository field.

For the Docker runtime smoke:

```bash
scripts/hermes-sidecar-docker-smoke.sh
scripts/hermes-sidecar-docker-s3-emulator-smoke.sh
```

That builds `containers/agent/Dockerfile` with `hermes-agent==0.17.0`, starts
the real Hermes gateway in Docker, drives `finitechat` CLI users through
invite/PIN admission before and after restore, and writes
`target/hermes-docker-smoke/report.json`. It also stops the first agent
container cleanly so the runtime entrypoint snapshots agent state to an
encrypted restic repository, checks the repository, wipes the local agent
volume, starts a fresh container whose entrypoint restores the latest tagged
snapshot before the gateway starts, verifies the same npub, verifies the
runtime `/healthz` endpoint, and admits a second user through the restored
gateway. Echo replies are not accepted as Docker runtime proof.

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

The Python adapter stays thin: it shells to `finitechat hermes
<action> --json` and translates JSON to Hermes `MessageEvent`s. The Rust
binary owns identity, MLS encryption, invite verification, durable cursors,
and storage. The bridge actions are `init`, `invite`, `pin`, `poll`,
`send`, `edit`, and `activity`.
