# finitecomputer-v2

Hard-cut self-serve SaaS codebase for Finite Computer.

This repo is for the product we are building now:

1. A user signs in through Account Auth, names their agent, selects an icon,
   and chooses a Runner class.
2. Core creates a Project and Finite Private grant state.
3. The selected Runner launches a real Hermes Agent Runtime.
4. A newly initialized Runtime copies the Product Release's bundled Finite
   Skills baseline once and exposes it before the first user turn. Existing
   agents keep that baseline until they explicitly run `finite skills sync`.
5. Core declares first-slice readiness from real Runtime/application health and
   preserved provider-durable state. Full Recovery Snapshot, key-backup, and
   empty-target restore support is an explicit post-MVP TODO, not a launch gate.
6. A Finite Chat Hosted Web Device gives the dashboard the proven web-chat
   experience; Electron and native apps can enroll later as additional Devices.
7. Product features ship through their owning services, UI, stable CLIs, and
   skills. They do not expand Runtime Management into a feature control plane.
8. The agent uses Finite Sites, Finite Brain, `finite-skills`, and Finite
   Private as one compatibility-tested product release.

The original `finitecomputer` repo remains the product already shipped to box1
and TRF while those users are unmigrated.

## Security And Recovery Posture

User data availability is the first security invariant. The trusted first
cohort targets O1: normal product paths minimize operator access, while an
explicit and audited Finite-Assisted Recovery path may expose restored data to
prevent permanent loss. Kata isolation and later Phala/TEE evidence improve the
normal privacy boundary; neither substitutes for key recovery, off-host
snapshots, usable exports, or empty-target restore drills.

See the system
[recoverability ADR](../docs/adr/0001-recoverability-precedes-operator-blindness.md)
and the active
[runtime recovery plan](docs/runtime-recovery-and-observability-plan.md).

## What This Repo Owns

- `apps/dashboard`: WorkOS/SaaS dashboard code for Project creation, agent
  overview, Hosted Web Device chat, external connections, Sites/Brain,
  runtime lifecycle, plaintext-safe ops, the skills catalog, and Finite Private
  administration.
- `crates/finite-saas-core`: Core service for Project/runtime/entitlement state.
- `crates/finite-saas-runner`: runner worker for launching Agent Runtimes.
- `crates/finite-private-limiter`: private inference limiter service. TODO:
  extract this to its own repo/deploy boundary after the v2 Core grant contract
  and image release path are stable. Keep it owned and deployed from v2 for now.
- `deploy/finite-computer`: copied deployment/runtime files for the v2 product
  stack (image Dockerfiles and runtime template; the k8s manifests and systemd
  units moved to `../infra/hosts/lat1/`). These need renaming and pruning as
  the split hardens.
- `../infra/hosts/lat1/scripts/deploy-finitechat-server.sh` and
  `deploy/finite-chat/lat1`: hosted Finite Chat server deployment lane for the
  SaaS stack.

## External Product Dependencies

These stay separate repos:

- `finite-sites`: Finite Sites and the `fsite` CLI.
- `finitechat`: Finite Chat server, protocol, native clients, CLI, and Hermes
  plugin.
- `finite-skills`: Finite-managed agent skills.
- `finite-brain`: dashboard and runtime knowledgebase sharing.

v2 deploys and integrates those services. Since the finite-mono cutover they
are sibling directories in this repo — consume them through the root Cargo
workspace and `infra/`, never by copying their code into `finitecomputer-v2/`.
For skills specifically, the monorepo tree is the only editable source;
the Runtime image bundles a tested revision for one-time installation into new
agents. Existing agents update only when they explicitly run
`finite skills sync`. Core, Runner, and the Runtime Management Pipe do
not select, poll, push, or activate a skills revision.

## Hard-Cut Rules

Do not add new v2 dependencies on:

- OpenCode
- a second dashboard-only chat transport outside Finite Chat
- dashboard-to-runtime shell, filesystem, Kubernetes, or provider APIs
- product feature commands or feature-specific status on the Runtime Management
  Pipe
- legacy dashboard-managed Published Apps in place of Finite Sites
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
Hermes local/Docker/Kata/Phala proof ladder.

## Local SaaS

The default local product path is the real Apple Container Runner, not a
dashboard stub or Docker-only canary. From the repository root on Apple
silicon and macOS 26 or newer:

```bash
container system start
export FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY=<operator-held-finite-private-key>
just dev up
```

Open <http://127.0.0.1:13002/dashboard>, name an agent, launch it, and use the
Hosted Web Device. This creates a real Core Project and lease, builds the
canonical Hermes 0.18.2 Runtime image, starts it through the generic Apple
Container provider, and preserves its bind-mounted `/data` across restarts and
image replacement. The runtime publishes generic `/healthz` readiness and an
Agent Principal `/contact` document; Finite Chat Devices own
KeyPackage/Add/Welcome admission.

Run the credential-gated end-to-end acceptance with `just dev saas-smoke`.
It requires real Hermes replies and proves chat-server, Hosted Web Device, and
Agent Runtime restart recovery. See
[`docs/local-integration-harness.md`](../docs/local-integration-harness.md) for
networking, key handling, services-only CI, and recovery details.

The copied code intentionally started slightly too large so we did not lose work
from the SaaS branch. Legacy dashboard chat and Connections implementations were
cut because they depended on the old relay, Kubernetes, and runtime filesystem;
their proven UX can return over Finite Chat and product-owned APIs or skills,
without turning Runtime Management into a feature plane.
OpenRouter fallback and machine publish/repo APIs remain cut. The remaining
cleanup is to reduce `finite-core`/`fc-dashboard` legacy model dependencies and
rename deployment paths from `finite-computer` to v2.

## SaaS Runner

Docker is the local preflight backend. Kata is the first production Runner;
Phala is the confidential fast-follow candidate. The current process-wide
backend env is transitional scaffolding while Project-selected placement lands:

```text
RuntimeSpec.runner_class = "kata"
```

Do not add `FC_RUNNER_BACKEND=kata` as another global branch; Kata must enter
through the generic Runner Contract and the same conformance suite as Phala.

Enclavia can be selected for a single pre-created enclave evaluation target:

```text
FC_RUNNER_BACKEND=enclavia
FC_RUNNER_ENCLAVIA_ENCLAVE_ID=<enclave-uuid>
```

See [docs/finite-stack-deployment.md](docs/finite-stack-deployment.md) and
[../infra/hosts/lat1/systemd/runner.env.example](../infra/hosts/lat1/systemd/runner.env.example)
for the live runner env, provider CLI prerequisites, and acceptance criteria.
