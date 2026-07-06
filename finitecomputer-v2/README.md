# finitecomputer-v2

Hard-cut self-serve SaaS codebase for Finite Computer.

This repo is for the product we are building now:

1. A user signs in with WorkOS.
2. Core creates a Project and Finite Private grant state.
3. A runner launches a real Hermes Agent Runtime.
4. The dashboard shows a Finite Chat invite with no PIN.
5. The user chats from the native Finite Chat client.
6. The agent uses `fsite`, `finite-skills`, Finite Private, and focused Finite
   tools instead of legacy `finitec` monolith commands.

The original `finitecomputer` repo remains the product already shipped to box1
and TRF while those users are unmigrated.

## What This Repo Owns

- `apps/dashboard`: WorkOS/SaaS dashboard code for Project creation, agent
  overview, runtime lifecycle, plaintext-safe ops, skills, and Finite Private
  administration.
- `crates/finite-saas-core`: Core service for Project/runtime/entitlement state.
- `crates/finite-saas-runner`: runner worker for launching Agent Runtimes.
- `crates/finite-private-limiter`: private inference limiter service. TODO:
  extract this to its own repo/deploy boundary after the v2 Core grant contract
  and image release path are stable. Keep it owned and deployed from v2 for now.
- `deploy/finite-computer`: copied deployment/runtime files for the v2 product
  stack. These need renaming and pruning as the split hardens.
- `scripts/deploy_finitechat_server_lat1.sh` and
  `deploy/finite-chat/lat1`: hosted Finite Chat server deployment lane for the
  SaaS stack.

## External Product Dependencies

These stay separate repos:

- `finite-sites`: Finite Sites and the `fsite` CLI.
- `finitechat`: Finite Chat server, protocol, native clients, CLI, and Hermes
  plugin.
- `finite-skills`: Finite-managed agent skills.
- `finite-brain`: future brain/knowledgebase sharing.

v2 should deploy and integrate those services, not vendor their code.

## Hard-Cut Rules

Do not add new v2 dependencies on:

- dashboard chat
- OpenCode
- dashboard-managed Published Apps
- `finitec publish`
- `finitec repo`
- `finitec gateway`
- `finitec hermes`
- `finitec finitechat`
- legacy `machine` control-plane operations

If a migrated existing user needs one of those surfaces temporarily, document it
as bridge code with a delete condition.

The only old Core state that must survive into v2 is Finite Private limiter
state: issued API-key token hashes, grants, reservations, usage, and audit
history. Old deploy lanes, runtime records, and machine-control state are not
compatibility targets.

## First Cleanup Targets

See [docs/carry-over-manifest.md](docs/carry-over-manifest.md).
See [docs/finite-stack-deployment.md](docs/finite-stack-deployment.md) for the
current deploy ownership split, and
[docs/hermes-runtime-test-matrix.md](docs/hermes-runtime-test-matrix.md) for the
Hermes local/Docker/Phala proof ladder.

## Local Create-Agent Canary

Run the v2 product-shaped local proof from the repo root with a valid deployed
Finite Private API key:

```bash
export FC_LOCAL_CANARY_FINITE_PRIVATE_API_KEY=<valid deployed fpk_... key>
./scripts/local_create_agent_canary.sh
```

The script starts local Postgres, `finite-saas-core`, the dashboard with a dev
WorkOS identity, submits the dashboard create-agent form, runs the Docker runner
backend once, and verifies the runtime `/healthz` and `/invite` endpoints. It
uses the hosted Finite Chat server by default and a mounted Docker `/data`
runtime state path, matching the shape we climb to remote Docker and Phala.
Because this is a local throwaway Core pointed at the live Finite Private
limiter, the script requires `FC_LOCAL_CANARY_FINITE_PRIVATE_API_KEY` by
default. A key minted by local Core is useful for launch wiring but will fail
real model calls with `401 invalid_api_key` at the live limiter. Set
`FC_LOCAL_CANARY_REQUIRE_FINITE_PRIVATE_KEY=0` only for an intentional
launch-only check that must not be handed to a human as a chat canary.
By default the script builds the v2-owned runtime image
`finitecomputer-v2-agent-runtime:local` before launching the canary. Set
`FC_LOCAL_CANARY_BUILD_RUNTIME_IMAGE=0` to reuse an already-built local image.

Useful overrides:

```bash
FC_LOCAL_AGENT_IMAGE=ghcr.io/finitecomputer/finite-agent-runtime:<tag-or-digest> \
FC_LOCAL_CANARY_BUILD_RUNTIME_IMAGE=0 \
FC_LOCAL_CANARY_KEEP_SERVICES=1 \
./scripts/local_create_agent_canary.sh
```

The copied code intentionally started slightly too large so we did not lose work
from the SaaS branch. Dashboard chat, dashboard-managed Published Apps,
dashboard-managed Connections, OpenRouter fallback, and machine publish/repo API
routes have been cut from the v2 product surface. The remaining cleanup is to
reduce `finite-core`/`fc-dashboard` legacy model dependencies and rename
deployment paths from `finite-computer` to v2.

## SaaS Runner

Docker is the local/remote preflight backend. Phala is the default confidential
SaaS backend:

```text
FC_RUNNER_BACKEND=phala
```

See [docs/finite-stack-deployment.md](docs/finite-stack-deployment.md) and
[deploy/finite-computer/systemd/runner.env.example](deploy/finite-computer/systemd/runner.env.example)
for the live runner env, Phala CLI prerequisite, and acceptance criteria.
