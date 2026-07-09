# Finite Stack Deployment Lanes

Status: v2 deploy authority.

This document describes the deployment shape for the Finite SaaS stack as we
move Core, dashboard, hosted Finite Chat, Finite Sites integration, and Agent
Runtimes into independently deployable services that can still ship together
when a product release needs lockstep.

## Problem Statement

Finite needs these user-facing surfaces to iterate independently:

- the finite.computer Core/dashboard product in this repo;
- the Finite Chat room server from `../finite-chat-darkmatter` / GitHub
  `finitecomputer/finitechat`;
- Agent Runtime images and launchers, including local Docker and Phala;
- the Finite Sites server from `../finite-sites`;
- marketing/static surfaces.

A fix to one surface must not require rolling all of them. A coordinated
release must still be possible when a protocol, auth, or product milestone spans
more than one service.

## Canonical Service Boundaries

| Surface | Canonical host | Owner repo | Runtime lane | Notes |
| --- | --- | --- | --- | --- |
| Core/dashboard | `finite.computer` / future app host | `finitecomputer-v2` | Core host | Account/control plane, WorkOS auth, dashboard, Projects, Finite Private grants, runtime launch records. |
| Chat room server | `chat.finite.computer` | `finitechat` | Core host now; v2 deploy lane | Native clients and Hermes runtimes use this by default. |
| Agent Runtime | Phala CVM by default; Docker for preflight | `finitecomputer-v2` plus `finitechat` plugin and external tools | local Docker, remote Docker, Phala | Runtime image should differ by destination only in provider config and durable volume binding. |
| Finite Sites API | `api.finite.chat` | `finite-sites` | finite-lat-2/Sites host | Existing Sites API route; do not reuse it for chat. |
| Finite Sites serving | `*.finite.chat` | `finite-sites` | finite-lat-2/Sites host | User-published content stays isolated from Core/chat. |

`chat.finite.chat` can be a later brand alias, but `chat.finite.computer` is
the canonical hosted room server because it avoids the existing `finite.chat`
wildcard/API routing and keeps chat with the SaaS account plane.

## Product Target

The desired self-serve user flow is:

1. The user logs in to finite.computer with WorkOS.
2. Core creates a Project, Finite Private grant, and Agent Runtime launch
   request.
3. The runner launches a real Hermes Agent Runtime, defaulting to Phala for
   confidential durable runtime hosting.
4. The dashboard shows a Finite Chat invite with no PIN.
5. The user scans or opens that invite with the native Finite Chat client.
6. The native client chats directly through `chat.finite.computer`.

Dashboard chat is intentionally cut for this release. The dashboard may later
add a trusted web bridge, but native-device Finite Chat is the product target.

## Release Model

Each service owns its source and local checks. v2 owns the SaaS deployment
coordination, environment config, runtime image promotion, and service health
gates.

Single-service deploys:

- dashboard-only changes use the v2 dashboard/Core deploy lane;
- chat server changes use the v2 chat server deploy lane below;
- runtime image changes climb the Hermes runtime matrix before promotion;
- Sites changes deploy from `finite-sites` to the Sites host.

Runtime image artifacts are published to GHCR as:

```text
ghcr.io/finitecomputer/finite-agent-runtime:<tag>
```

Before a runtime image can be promoted to Phala or any SaaS runner that pulls
from outside the GitHub Actions builder, the package must either be public or
the provider/runner must have a `read:packages` pull secret. CI's `GITHUB_TOKEN`
can push the package, but it does not make private GHCR packages anonymously
pullable.

## Phala Runner Lane

Phala is the default confidential runner for the first SaaS cut. The runner uses
the official `phala` CLI as the provider management pipe because it already
handles authenticated deploys, client-side env sealing, `--wait`, and restart.
The v2 Rust runner still owns the product contract: lease from Core, render a
Docker-equivalent compose file, provide a runtime-scoped Finite Private key,
wait for `/healthz` and `/invite`, then complete the Core request.

Host prerequisites:

```sh
npm install -g phala@1.1.19
install -d -m 700 /var/lib/finite/saas-runner
install -d -m 750 /etc/finite-computer
```

Runner env is configured from:

```text
/etc/finite-computer/runner.env
```

Use
`../infra/hosts/lat1/systemd/runner.env.example` as the template. The
production shape is:

- `FC_RUNNER_BACKEND=phala`
- `FC_RUNNER_RUNTIME_ARTIFACT_ID=<promoted OCI runtime artifact in Core>`
- `PHALA_CLOUD_API_KEY=<host-only Phala API key>`
- `FC_RUNNER_PHALA_INSTANCE_TYPE=tdx.small`
- `FC_RUNNER_PHALA_DISK_SIZE=40G`
- `FC_RUNNER_PHALA_PUBLIC_LOGS=false`
- `FC_RUNNER_PHALA_PUBLIC_SYSINFO=false`

For each leased Project, the runner writes a provider work directory under:

```text
/var/lib/finite/saas-runner/phala/<runtime-name>/
```

That directory contains a rendered `docker-compose.yml` and sealed-env input
file. The compose file mounts Phala named volume `agent_state` at `/data`, the
same durable state path used by the Docker runner. It points the runtime at
`https://chat.finite.computer`, exposes port `8080`, and records the public
invite/status endpoint as:

```text
https://<app-id>-8080.dstack-pha-<teepod>.phala.network/invite
```

Do not pass OpenRouter or user-owned model keys through this lane. Core issues
a runtime-scoped Finite Private key and the runner gives that key only to the
new runtime through the Phala env sealing path.

Acceptance for enabling the runner is not "Phala accepted the deploy." A SaaS
launch is accepted only after Core records the runtime, the Phala endpoint
returns `ready=true` from `/healthz`, `/invite` returns a no-PIN Finite Chat
invite, the native client joins, Hermes answers multiple real turns through
Finite Private, and a Phala restart preserves the same agent identity, room,
memory, and workspace state.

Lockstep deploys:

1. add or update a release manifest under a future `ops/releases/`;
2. run service-local validation for every pinned repo/commit;
3. deploy services in dependency order;
4. record deployed release metadata on each host/provider.

## Deployment Order For Coordinated Releases

Use this default order unless a release manifest says otherwise:

1. Chat server, when protocol/API is backward compatible.
2. Core/dashboard, when it consumes the new chat/API behavior.
3. Agent Runtime image, after local Docker and remote Docker prove the same
   Hermes behavior.
4. Phala runtime canary, after Docker proves the image.
5. Finite Sites, when publish entitlement or public serving behavior changes.

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
- Runtime image: local real-Hermes proof, remote Docker proof, Phala proof.
- Sites: health, claim/publish smoke, grant check, wildcard route smoke, app
  wake smoke.

Cross-service gates:

- dashboard/Core can reach `https://chat.finite.computer/health`;
- native client can consume a dashboard-issued pairing invite;
- Agent Runtime can receive a Finite Chat turn and reply through Hermes;
- Agent Runtime can use Finite Private;
- Agent Runtime can publish a site with `fsite`;
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
