# Hermes Runtime CI

## Problem Statement

The Hermes runtime testing loop needs to prove the same image through Docker and
the current confidential runner lane without rebuilding the image inside each
test layer or scheduling Docker builds on `finite-lat-2`.

## Acceptance Criteria

- The Docker runtime smoke runs in native Depot CI from GitHub events.
- The runtime image is built once before the Docker smoke.
- The Docker smoke uses that prebuilt image instead of rebuilding inside the
  test.
- The GHCR publish step pushes the same image ID proven by Docker smoke.
- Phala durable-home publish uses the same image ID proven by the durable
  `/home/node` smoke.
- Tinfoil handoff artifacts are generated from the published image digest only
  for explicit Tinfoil dispatches.

## Current Runner

Current runtime image proof lives in `finite-mono`:

- Build-only source preflight:
  `.depot/workflows/hermes-runtime-smoke.yml`
- Build-once promotion path:
  `.depot/workflows/runtime-image.yml`
- Runner: `depot-ubuntu-24.04`
- Builder: `depot build` through `depot/setup-action` OIDC
- Project id: `DEPOT_RUNTIME_IMAGE_PROJECT_ID` or shared `DEPOT_PROJECT_ID`

The old `finite-lat-2-finitechat-hermes-runtime` self-hosted runner path is
retired for mono Docker image CI. Keep lat2 runner references only as
historical evidence or operator inventory until the legacy repo runners are
removed.

## Image Flow

1. `finitecomputer-v2/scripts/build_runtime_image.py` builds the canonical
   monorepo runtime image with `FC_RUNTIME_IMAGE_ENGINE=depot`.
2. The smoke workflow uses `depot build --load` so the exact built image is
   available in the runner's local Docker daemon for optional Docker smokes.
3. The publish workflow builds the canonical image once, validates and smokes
   that local image ID, then pushes the same bytes to GHCR.
4. Tinfoil handoff artifacts consume the published image digest only after an
   explicit Tinfoil dispatch.

## Dispatch

Dispatch the build-once runtime image canary at an explicit GitHub revision:

```sh
depot ci dispatch \
  --org scthc5h66g \
  --repo finitecomputer/finite-mono \
  --workflow runtime-image.yml \
  --ref <github-branch-or-sha> \
  --input version=<date-based-version> \
  --input publish_production=false
```

Use the Phala durable-home gate when proving the current hosted-agent runtime:

```sh
depot ci dispatch \
  --org scthc5h66g \
  --repo finitecomputer/finite-mono \
  --workflow hermes-runtime-smoke.yml \
  --ref <github-branch-or-sha> \
  --input durable_home_docker_smoke=true \
  --input chat_interruption_smoke=true
```

## Current Caveat

The Tinfoil backup/restore Docker smoke is not a current CI lane. Its old
GitHub Actions dispatch helpers were removed during the Depot CI migration. If
that work resumes, it must consume the canonical digest proved by the Depot
runtime-image workflow rather than restoring a parallel publication path. The
current hosted-agent lane is the Phala-shaped durable `/home/node` smoke, matching the
runner contract proven in finitecomputer's Phala runtime spike.
