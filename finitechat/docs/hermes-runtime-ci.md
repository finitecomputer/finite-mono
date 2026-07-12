# Hermes Runtime CI

## Problem Statement

The Hermes runtime testing loop needs to prove the same image through Docker and
the current confidential runner lane without waiting on slow GitHub-hosted
Docker builds or rebuilding the image inside each test layer.

## Acceptance Criteria

- The Docker runtime smoke runs on a Finite-controlled x86_64 Docker host.
- The runtime image is built once before the Docker smoke.
- The Docker smoke uses that prebuilt image instead of rebuilding inside the
  test.
- The GHCR publish step pushes the same image ID proven by Docker smoke.
- Phala durable-home publish uses the same image ID proven by the durable
  `/home/node` smoke.
- Tinfoil handoff artifacts are generated from the published image digest only
  for explicit Tinfoil dispatches.

## Current Runner

- Host: `finite-lat-2`
- GitHub runner name: `finite-lat-2-finitechat-hermes-runtime`
- Repo: `finitecomputer/finitechat`
- Labels: `self-hosted`, `Linux`, `X64`, `finite-lat-2`, `docker`,
  `hermes-runtime`
- Systemd unit:
  `actions.runner.finitecomputer-finitechat.finite-lat-2-finitechat-hermes-runtime.service`
- Runner directory: `/srv/github-runner/finitechat-hermes-runtime`

The repository is public, so self-hosted execution is intentionally scoped to the
Docker runtime job. Pull requests run the normal GitHub-hosted test job but do
not run the self-hosted Docker runtime smoke.

## Image Flow

1. `scripts/hermes-build-runtime-image.py` stages the Docker build context and
   builds `ghcr.io/<owner>/finite-chat-hermes-runtime:<sha>` locally on the
   self-hosted runner.
2. Pushes to `main` run `scripts/hermes-durable-home-docker-smoke.py` against
   the prebuilt image. This is the default Phala-shaped `/home/node` gate.
3. Explicit Tinfoil dispatches run `scripts/hermes-sidecar-docker-smoke.sh` with
   `FINITE_DOCKER_IMAGE=<that image>` and `FINITE_DOCKER_SKIP_IMAGE_BUILD=1`.
4. `scripts/hermes-publish-proven-image.py` inspects the selected smoke report
   and pushes the same local image ID to GHCR when the matching publish input is
   set.
5. `scripts/hermes-tinfoil-handoff.py` and
   `scripts/hermes-tinfoil-canary-artifacts.py` consume the publish report.

## Dispatch

Use the publish gate when proving a Tinfoil-bound image:

```sh
scripts/hermes-github-publish-gate.py \
  --ref <branch> \
  --branch <branch> \
  --restic-prefix agent-runtimes/<canary>/ci-smoke/restic \
  --tinfoil-release-tag <tag>
```

For a raw workflow dispatch:

```sh
gh workflow run ci.yml \
  -R finitecomputer/finitechat \
  --ref <branch> \
  -f docker_smoke=true \
  -f publish_runtime_image=true \
  -f restic_backend=s3 \
  -f restic_prefix=agent-runtimes/<canary>/ci-smoke/restic
```

Use the Phala durable-home gate when proving the current hosted-agent runtime:

```sh
gh workflow run ci.yml \
  -R finitecomputer/finitechat \
  --ref <branch> \
  -f durable_home_docker_smoke=true \
  -f publish_phala_runtime_image=true
```

## Current Caveat

The Tinfoil backup/restore Docker smoke is no longer the default `main` gate.
It remains available for explicit Tinfoil dispatches only. The current
hosted-agent lane is the Phala-shaped durable `/home/node` smoke, matching the
runner contract proven in finitecomputer's Phala runtime spike.
