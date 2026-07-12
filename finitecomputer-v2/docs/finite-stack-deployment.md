# Finite Stack Deployment Lanes

Status: v2 deploy authority.

This document describes the deployment shape for the Finite SaaS stack as we
move Core, dashboard, hosted Finite Chat, Finite Sites integration, and Agent
Runtimes into independently deployable services that can still ship together
when a product release needs lockstep.

## Problem Statement

Finite needs these user-facing surfaces to iterate independently:

- the finite.computer Core/dashboard product in this repo;
- the Finite Chat room server from `finitechat/`;
- Agent Runtime images and launchers, including local Docker, Kata, and Phala;
- the Finite Sites server from `finite-sites/`;
- immutable Managed Skills Baseline revisions from `finite-skills/`;
- marketing/static surfaces.

A fix to one surface must not require rolling all of them. A coordinated
release must still be possible when a protocol, auth, or product milestone spans
more than one service.

## Canonical Service Boundaries

| Surface | Canonical host | Owner repo | Runtime lane | Notes |
| --- | --- | --- | --- | --- |
| Core/dashboard | `finite.computer` / future app host | `finitecomputer-v2` | Core host | Account/control plane, WorkOS auth, dashboard, Projects, Finite Private grants, runtime launch records. |
| Chat room server | `chat.finite.computer` | `finitechat` | Core host now; v2 deploy lane | Native clients and Hermes runtimes use this by default. |
| Agent Runtime | Kata first; Phala confidential fast follow; Docker for preflight | `finitecomputer-v2` plus `finitechat` plugin and external tools | local Docker, Kata, Phala | Runtime image differs only in provider config and Provider Durable Volume binding. Kata trusts the privileged host operator; Phala earns stronger claims through evidence. |
| Managed Skills Baseline | `git.finite.chat` distribution mirror | `finite-skills` | baked Runtime revision plus event-driven activation | `finite-mono/finite-skills` is the only editable source. Core selects a promoted immutable digest; Runtime verifies/activates it; Runner has no skills role. |
| Finite Sites API | `api.finite.chat` | `finite-sites` | finite-lat-2/Sites host | Existing Sites API route; do not reuse it for chat. |
| Finite Sites serving | `*.finite.chat` | `finite-sites` | finite-lat-2/Sites host | User-published content stays isolated from Core/chat. |

`chat.finite.chat` can be a later brand alias, but `chat.finite.computer` is
the canonical hosted room server because it avoids the existing `finite.chat`
wildcard/API routing and keeps chat with the SaaS account plane.

## Product Target

The desired self-serve user flow is:

1. The user logs in through Account Auth, names their agent, and selects an
   icon. Core assigns the standard Runner class from product policy.
2. Core creates a Project, Finite Private grant, and Agent Runtime launch
   request without leaking provider-specific handles into the product model.
3. The assigned Runner launches the same compatibility-pinned Hermes Agent
   Runtime image; Kata is the first launch lane and Phala follows through the
   same contract. The image exposes its Product Release's baked Finite Skills
   Revision before the first user turn.
4. Core waits for real Runtime/application health before reporting first-slice
   readiness. Full Recovery Snapshot, key-backup, and empty-target restore
   support remains a disclosed post-MVP TODO/open question.
5. The dashboard provisions or opens the user's Finite Chat Hosted Web Device
   and enters the canonical agent Room.
6. The user chats in the proven dashboard UI, connects Telegram and Google,
   publishes/previews/lists Finite Sites, and uses Finite Brain.
7. Electron or a native app may enroll as another Device and sync the same
   Room without depending on the Hosted Web Device's availability.

Hosted web chat is intentionally trusted-server chat, not browser E2EE. It is
the launch surface; Electron and native clients are additive local-custody
Devices, not replacement UX projects.

## Release Model

Each service owns its source and local checks. v2 owns the SaaS deployment
coordination, environment config, runtime image promotion, and service health
gates.

Single-service deploys:

- dashboard-only changes use the v2 dashboard/Core deploy lane;
- chat server changes use the v2 chat server deploy lane below;
- runtime image changes climb the Hermes runtime matrix before promotion;
- Sites changes deploy from `finite-sites` to the Sites host.
- compatible skills-only changes publish a tested baseline for the future
  explicit `finite skills sync` command. Existing agents choose when to sync;
  Core, RMP, and Runner do not roll it out automatically.

Every new Runtime seeds the baseline bundled from this monorepo before its
first turn. Neither Runtime nor dashboard consumes a second editable copy.
Later updates are runtime-local and explicit; no feature-specific management
message or Runner operation is involved.

Runtime image artifacts are published to GHCR as:

```text
ghcr.io/finitecomputer/agent-runtime:<tag>
```

Before a runtime image can be promoted to Phala or any SaaS runner that pulls
from outside the GitHub Actions builder, the package must either be public or
the provider/runner must have a `read:packages` pull secret. CI's `GITHUB_TOKEN`
can push the package, but it does not make private GHCR packages anonymously
pullable.

## Runner Lanes

Kata is the first production runner under the provider-neutral Runner Contract.
It is isolated compute on finite-lat-1, not an operator-blind environment. The
first trusted cohort may launch on provider-durable state before the off-host
Recovery Snapshot design is complete, with honest product/support disclosure.

The NixOS host pins Kata Containers, QEMU, the guest kernel and image, nerdctl,
CNI plugins, and containerd in one closure. No globally installed provider CLI
is required. The runner is rootful only because it owns the containerd socket
and CNI namespaces; its typed contract is deliberately limited to launch,
status, restart, stop, and compute-only destroy.

Runner env is configured from:

```text
/etc/finite/runner.env
```

Use
`../infra/hosts/lat1/systemd/runner.env.example` as the template. The
production shape is:

- `FC_CORE_RUNNER_API_TOKEN` (the bearer assigned to this exact Runner; never
  reuse the shared `FC_CORE_API_TOKEN`)
- `FC_RUNNER_CLASS=kata`
- `FC_RUNNER_RUNTIME_ARTIFACT_ID=<promoted OCI runtime artifact in Core>`
- `FC_RUNNER_KATA_NAMESPACE=finite`
- `FC_RUNNER_KATA_OCI_RUNTIME=io.containerd.kata.v2`
- `FC_RUNNER_KATA_CPUS=4`
- `FC_RUNNER_KATA_MEMORY=8G`

For new versioned leases, Core—not Runner—selects the newest promoted,
non-retired digest-pinned OCI artifact and persists the complete RuntimeSpec
before returning the lease. The checked-in Core service sets
`FC_CORE_RUNTIME_ENV_JSON` to the public Sites and Brain endpoints that are
copied into that spec. The Runner artifact and `FC_RUNNER_RUNTIME_ENV_JSON`
values remain only as an N-1 fallback for already-existing rows without a
RuntimeSpec during expand; a present spec always wins. Do not put credentials
in either JSON map. Each name and value is bounded and validated identically by
Core and Runner.

Core holds the corresponding rotatable keyring. Its non-secret metadata shape
is an array in `FC_CORE_RUNNER_CREDENTIALS_JSON`:

```json
[
  {
    "credentialId": "kata-current",
    "tokenEnv": "FC_CORE_RUNNER_CREDENTIAL_TOKEN_KATA_CURRENT",
    "runnerId": "finite-kata-runner-1",
    "runnerClasses": ["kata"],
    "sourceHostId": "finite-lat-1",
    "revoked": false
  }
]
```

The environment variable named by `tokenEnv` contains the secret outside the
repository. Rotation adds a second entry and secret name with the same exact
binding, moves the Runner to that bearer, then marks or removes the old entry.
Setting `revoked` rejects only that credential. Empty class sets, duplicate
bearers, and bearer reuse across Core route capabilities fail Core startup.

For each leased Project, the runner writes a provider work directory under:

```text
/var/lib/finite-saas-runner/kata/<runtime-name>/
```

That directory is bind-mounted at `/data` and survives restart, image
replacement, stop, and destroy. Runtime compute is a Kata microVM. Agent HTTP
port `8080` is dynamically published on loopback and recorded in Core; only
same-host services can reach it. A transient root-only env file carries launch
credentials to nerdctl and is deleted immediately after container creation.
It never appears in process arguments.

```text
http://127.0.0.1:<allocated-port>/contact
```

`/contact` is a product-facing discovery fact, not a Runner health gate. The
Runtime may keep compatibility routes internally, but Core and new clients do
not publish or canonize them.

### Current production preflight (2026-07-10T19:03Z)

**Ready for the authorized fixed-artifact rollout; this is not Kata readiness
proof.** Read-only inspection found the `finite-lat-1` runner timer enabled and
active, a maximum of 12 Runtimes with 2 active, and 4 CPU / 8G per Runtime; the
observed launch shape has two read-write host binds at `/data`. The selected
`.5` artifact returned `503` because one pending attachment carried a historical
Chat-server loopback blob URL into the Kata guest, where it ended the Hermes
inbound stream. The canonical public blob exists. The repository already makes
the Chat server write its public origin and makes the Runtime reroute historical
loopback blob paths through its trusted server origin before verifying the
encrypted bytes. Publish and promote a fresh post-fix artifact and prove it
with the canary's fresh launch; the old `.5` guest need not be repaired first.
The live environment also contains only `FC_CORE_API_TOKEN`, not the required
`FC_CORE_RUNNER_API_TOKEN`. Do not treat the timer, capacity, artifact
selection, or bind configuration as end-to-end success until the route-scoped
credential and fixed artifact are deployed. No live mutation was made during
this preflight.

Core issues a runtime-scoped Finite Private key and the runner gives that key
only to the new runtime at launch. Later inference changes are explicit typed
agent-local mutations through finite-agentd; the platform never edits a remote
`.env` file.

Acceptance for enabling the runner is not "Phala accepted the deploy." A SaaS
launch is accepted only after Core records the runtime, the Kata endpoint
returns `ready=true` from `/healthz`, the Hosted Web Device creates or resumes
the canonical room using the agent identity from `/contact`, Hermes answers
multiple real turns through Finite Private, and a Kata restart preserves the
same agent identity, room, memory, and workspace state. Native and Electron
Devices may join that room later without changing the Runner contract.

Phala remains the confidential fast-follow lane. A separate worker advertises
`FC_RUNNER_CLASS=phala` and uses the official CLI for authenticated deploy,
sealed environment delivery, and provider lifecycle. It implements the same
Runner Contract and acceptance matrix; it is not part of this rollout.

## Enclavia Evaluation Lane

Enclavia is available as a runner target for testing the same Agent Runtime
image inside a pre-created Enclavia enclave. This lane is deliberately not the
default SaaS backend yet: the current runner points at one configured enclave
ID, pushes one local Docker image into that enclave, injects runtime env through
Enclavia per-enclave secrets, and records the hosted proxy endpoint in Core.

Host prerequisites:

```sh
cargo install enclavia-cli
enclavia auth login
docker info
```

The Enclavia enclave must already exist and should be created with:

- container port `8080`;
- persistent encrypted storage mounted at `/data`;
- outbound egress for the model/chat dependencies the runtime needs;
- upgradable mode only if the operator is ready to handle staged upgrades
  manually.

Runner env:

```text
FC_RUNNER_CLASS=enclavia
FC_RUNNER_RUNTIME_ARTIFACT_ID=<promoted OCI runtime artifact in Core>
FC_RUNNER_ENCLAVIA_BIN=/usr/local/bin/enclavia
FC_RUNNER_ENCLAVIA_ENCLAVE_ID=<pre-created enclave UUID>
FC_RUNNER_ENCLAVIA_PULL_POLICY=missing
FC_RUNNER_MAX_SANDBOXES=1
```

For each leased Project, the runner:

1. ensures the promoted OCI image exists in the local Docker daemon, pulling it
   when `FC_RUNNER_ENCLAVIA_PULL_POLICY=missing` or `always`;
2. writes Docker-equivalent runtime env into Enclavia secrets, sending raw
   API-key values through stdin rather than CLI argv;
3. runs `enclavia push <local-image> <enclave-id> --json`;
4. polls `enclavia enclave status <enclave-id> --json` until `running`;
5. waits for `ready=true` from the generic Runtime health endpoint:

```text
https://<enclave-id>.enclaves.beta.enclavia.io/proxy/healthz
```

After readiness, Core may publish the independent agent contact fact at
`https://<enclave-id>.enclaves.beta.enclavia.io/proxy/contact`; contact
availability is not part of compute lifecycle admission.

Do not point more than one active Core runner at the same Enclavia enclave. A
non-upgradable enclave rejects a second push; an upgradable enclave stages the
second push, which this runner intentionally does not auto-confirm yet.

Lockstep deploys:

1. add or update a release manifest under a future `ops/releases/`;
2. pin the Runtime OCI digest, Hermes and Finite binary/service versions, baked
   and desired Finite Skills Revision digests, and allowed compatibility
   envelope;
3. attach the Recoverability Contract, Recovery Set manifest, current
   Operator-Privacy Level, active-skills evidence, and last empty-target restore
   evidence;
4. run service-local validation for every pinned component and revision;
5. deploy services in dependency order;
6. record deployed release metadata on each host/provider.

## Deployment Order For Coordinated Releases

Use this default order unless a release manifest says otherwise:

1. Chat server, when protocol/API is backward compatible.
2. Core/dashboard, when it consumes the new chat/API behavior.
3. Agent Runtime image, after local Docker proves the same Hermes behavior.
4. Kata runtime canary, including provider-volume restart preservation. Off-host
   Recovery Snapshot and empty-target restore remain a post-MVP TODO.
5. Phala runtime canary, after Kata proves the common contract.
6. Finite Sites, when publish entitlement or public serving behavior changes.
7. Finite Skills Revision after every declared binary/service dependency is
   live, then canary activation and stable promotion without a Runtime restart.

If a database migration is not backward compatible, split the release into an
expand phase and a contract phase instead of doing a single all-at-once push.

## Data Ownership

- Core/dashboard state remains in the v2 Core database.
- The only old Core state that must survive the hard cut is Finite Private
  limiter state: issued API-key token hashes, grants, reservations, usage, and
  audit records. Old runtime records and deploy-lane state are not continuity
  requirements.
- The chat room server owns room-ordering durability and exposes opaque
  encrypted delivery state only.
- Agent Runtime state belongs on the runtime provider's durable mount.
- Finite Skills source belongs only in the monorepo; immutable revision
  artifacts and manifests are release state, while active/last-good caches are
  reproducible Runtime state and user-local skill overrides are recoverable
  user data.
- A Provider Durable Volume is primary state, not a backup. Agent Runtime state
  also needs a provider-independent off-host Recovery Snapshot covering the
  complete `/data` Recovery Set.
- Sites owns the Sites registry, blobs, app state, and public serving state.
- Marketing/static surfaces are stateless.

The hosted room server can use SQLite for the current canary while restart
durability and backup are verified. Before broad multi-user traffic, prefer
Postgres or another operationally managed store for the hosted room server.

## DNS Rules

- Do not route chat through `api.finite.chat`; that name belongs to Sites.
- Do not route Core/dashboard through `*.finite.chat`; that wildcard belongs to
  user-published Sites.
- Keep `finite.computer` for SaaS auth, account, dashboard, and chat product
  surfaces.
- Keep `finite.chat` for marketing and user-published Sites.

## Validation Gates

Per-service gates:

- Dashboard/Core: lint, build, auth/dashboard smoke, route health.
- Chat server: `finitechat-server` route/conformance/persistence tests, HTTP
  health, restart persistence, SSE/pull repair, production `/health` provenance.
- Runtime image: local real-Hermes proof, full-product Docker proof, Kata proof,
  then Phala proof.
- Managed skills: manifest/provenance validation, representative Finite
  workflow tests, offline baked discovery, between-turn activation, crash
  atomicity, in-process reload, rollback, and user-override preservation on the
  same Docker/Kata/Phala image.
- Sites: health, claim/publish smoke, grant check, wildcard route smoke, app
  wake smoke.

Cross-service gates:

- dashboard/Core can reach `https://chat.finite.computer/health`;
- native client can consume a dashboard-issued pairing invite;
- Agent Runtime can receive a Finite Chat turn and reply through Hermes;
- Agent Runtime can use Finite Private;
- Agent Runtime can publish a site with `fsite`;
- first-turn Hermes guidance selects current Finite-specific workflows, and
  dashboard/release/Runtime evidence agree on the active skills digest;
- a compatible canary skills revision activates and rolls back without changing
  the Runtime boot id or Hermes PID;
- every stateful service and Agent Runtime restores its declared Recovery Set
  from a service-consistent off-host snapshot onto an empty replacement target;
- Runtime Retirement preserves recovery material and normal product/billing
  paths cannot invoke Purge User Data;
- `finite.computer`, `chat.finite.computer`, `api.finite.chat`, and a wildcard
  site all route to their intended service.

## Current Chat Server Deploy Lane

This repo owns the hosted Finite Chat production deploy mechanics for the SaaS
product. The `finitechat` repo owns source, protocol compatibility, local
checks, and the release-blocking decision about which commit is safe to deploy.

The deploy script now lives in `infra/hosts/lat1/scripts/` (it resolves the
workspace path relative to the finite-mono root):

```sh
../infra/hosts/lat1/scripts/deploy-finitechat-server.sh \
  finitecomputer-v2/deploy/finite-chat/lat1 \
  <finitechat-full-sha>
```

Operator config lives in:

```text
deploy/finite-chat/lat1/secrets/workspace.env
```

Use `deploy/finite-chat/lat1/workspace.env.example` as the template. Do not
commit filled secrets.

The current deploy script:

1. fetches `https://github.com/finitecomputer/finitechat.git` on lat1 at a
   pinned commit;
2. builds `finitechat-server` with Nix-provided Rust tooling;
3. runs the server on `10.42.0.1:8787` with SQLite state under
   `/var/lib/finite-chat/data/server.sqlite3`;
4. points a k3s Service/Endpoints object at the host bridge address;
5. serves `chat.finite.computer` through Traefik and keeps `chat.finite.vip` as
   a temporary canary alias while needed;
6. writes `/var/lib/finite-chat/last-deploy.json` with the pinned source ref.

This is an interim host-built lane. Replace it with a v2 image/release artifact
after the hosted server shape is stable, but keep the same release gate:
production `/health` must report the expected finitechat `source_commit` and
`source_dirty: false`.
