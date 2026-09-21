Latest latency investigation: [measurements, changes, and remaining production gates](latency/README.md).

# Substrate / Hermes local spike

Two stub owners each own one persistent Hermes actor. Hermes serves its native
HTTP/JSON-RPC WebSocket interface. Substrate owns placement, full-state
suspend/resume, snapshots, and request-triggered wake. Core has no changes.

See [measured results and remaining acceptance](EVIDENCE.md).

## Scope and acceptance

- Alice and Bob have separate homes, native credentials, and actor identities.
- Chat through real `hermes serve` and the TUI JSON-RPC WebSocket protocol.
- Explicitly suspend one actor, then wake it by requesting its same URL.
- Retain chat history and home files; verify the other actor stays independent.
- Run Xvfb + Openbox + a sample GUI window and take a real screenshot from inside the actor.
- Run the pinned SimpleX daemon and Hermes gateway alongside native web chat.

The current accepted direction is [shared notification-only wake](notifications/README.md):
one shared pod wakes both agents without receiving chat contents or holding
message-decryption keys. Each agent's SimpleX daemon sleeps with its actor.
The earlier per-agent external transport below is retained as historical proof,
but was rejected because it required an always-on plaintext-capable daemon per
agent. See the notification variant for current results and remaining gates.

## Architecture

```
owner row -> stable actor name -> Substrate lifecycle API
client URL -> fixed ingress mapping -> atenet router -> Hermes :80
                                              |-> resume if suspended
actor: Hermes serve + gateway + SimpleX + X11 desktop
       /home/agent durable directory + full process snapshot
```

`owners.json` is the stub database. `agent.py --user alice sleep` demonstrates
the small provider boundary. The user flag is trusted fixture input, not real
account authentication. Native Hermes credentials prove data-access isolation;
Core account authentication would supply the owner ID in an integration.

`render.py` emits a two-worker pool, per-owner templates and an NGINX ingress.
The ingress overwrites `ate-target-actor` from fixed host configuration and
forwards the complete Hermes route surface and WebSocket upgrades. It has no
route allowlist or worker-address knowledge. GKE Gateway HTTPRoute header
modification is a candidate replacement for the local NGINX projection; it
has not been qualified here. Caddy is not required.

Each owner has a separate template because Substrate constructs a golden
snapshot when creating a template. Sharing a preinitialized Hermes/SimpleX
snapshot would clone connector identities and credentials across owners.
Production bootstrap should happen after per-actor identity is known if a
shared template is desired. Do not call template cloning an ownership boundary.

## Upstream choices

- Substrate: `bb0effed188e06a44e03862cb6ea993e58f86893`.
- Hermes: root flake pin `29112bef099274229cadff79cdff7bf7b99c4b77` (0.21.0).
- SimpleX: root `simplex-chat` package (Nix version 7.0.2; binary reports 7.0.0.12).
- AX `d8ed0fe38bceb7842d3c47817d53d16ccdfcb601` was inspected, not deployed. Its runner contract adds a task/workspace
  controller and Redis and currently uses DATA snapshots, restarting the process
  tree on resume. Direct Substrate actors are the smaller fit for existing Hermes
  and full desktop/process suspension.

References: [Substrate API](https://github.com/agent-substrate/substrate/blob/bb0effed188e06a44e03862cb6ea993e58f86893/docs/api-guide.md),
[AX runner](https://github.com/google/ax/blob/main/docs/runner.md),
[Fly lifecycle comparison](https://linear.app/finitecomputer/issue/FIN-21/engineering-plan-hermes-runtime-and-lifecycle).

## Required upstream patch

`substrate-readonly-home.patch` fixes a reproduced failure after a successful
full snapshot: atelet uses plain `RemoveAll` on the durable home, but Hermes
copies read-only skill directories there. With all Linux capabilities dropped,
cleanup returns permission denied. Retrying then checkpoints an already-stopped
sandbox and gets stuck. The patch reuses upstream `RemoveAllWritable`, already
used for image directories, and includes a regression run with `--cap-drop=ALL`.
Apply it to the pinned Substrate checkout before building/deploying. This is a
local patch, not a claim that upstream main works unmodified.

## Local setup

Use a separate kubeconfig and the upstream Kind setup, never the current cluster
context. This run uses `/private/tmp/finite-substrate-state/kubeconfig`, cluster
`finite-hermes-spike`, and loopback registry `localhost:5017`. The upstream setup
script deletes an existing cluster of the selected name: inspect before rerunning.
The macOS development tool environment is `nix-shell --impure tools.nix` from this
directory (or pass its full path from the worktree root). It does not install tools
system-wide. It currently targets Apple Silicon.

`build-image.sh` automates the existing local Linux-builder path using
`SPIKE_STATE_DIR`, `SPIKE_IMAGE`, and optional `SPIKE_NIX_BUILDER`.

Build the root flake's `packages.aarch64-linux.hermes-agent-minimal`,
`hermes-agent-minimal-runtime`, and `simplex-chat` with a Linux Nix builder.
Export all three closures into a Docker build context under `nix/store`, copy
`runtime.py` and `Dockerfile`, then pass their store paths as `HERMES_PATH`,
`PYTHON_PATH`, and `SIMPLEX_PATH`. No credentials enter the build context/image.

Actors also need an egress policy: the default denies outbound traffic. The
upstream CLI has no policy verb at this pin. Copy `egress-fixture.go` into
`cmd/finite-spike-egress/main.go` in the pinned Substrate checkout, then run
`go run ./cmd/finite-spike-egress alice-agent bob-agent` in the task Nix shell
with the isolated `KUBECONFIG`. This grants all destinations to these named
disposable actors only. It is not a production network-isolation policy.

Build Substrate's `cmd/ateom-gvisor` through `hack/run-tool.sh ko build`, and build
`cmd/kubectl-ate`. Run `hack/install-ate-kind.sh --deploy-ate-system` with the isolated
kubeconfig and matching registry. Pin the resulting image digests for rendering.

Render fixtures with `render.py --image IMAGE --worker-image WORKER_IMAGE
--version NODE_VERSION_LABEL --snapshot-location gs://ate-snapshots/finite-hermes-spike/
--state-dir /private/tmp/finite-substrate-state/fixtures`. If a Finite Private key
is supplied, load it into `FINITE_PRIVATE_API_KEY` from its authorized local file;
never put it in an argument, checked-in manifest, transcript, or image.

Apply `pool.json` and `ingress.json` with kubectl. Create the `finite-hermes-spike`
atespace and both `*.template.json` with kubectl-ate. Wait for golden tags before
creating the actors with `agent.py`. Credential-bearing manifests and native
password files are mode 0600 in a mode 0700 directory outside the repository.
This is disposable local secret handling, not a production Core custody design.

Forward `svc/ingress` in `finite-hermes-spike` to loopback port 18080. `probe.py`
sets the correct host while connecting to loopback and exercises native login,
anonymous rejection, cross-owner rejection, header override, and WebSocket RPC.
Use `--prompt` for a real model turn once the authorized inference key is present.
For a browser, resolve `alice.agents.test` and `bob.agents.test` to loopback and
use port 18080. Public DNS/TLS is a separate GKE qualification step.

## Operational fit and limits

GKE Standard is the candidate cloud target. The current atelet mounts writable
host paths, whereas [Autopilot denies these](https://docs.cloud.google.com/kubernetes-engine/docs/concepts/autopilot-security).
The upstream bootstrap also requires specific Kubernetes certificate APIs at
cluster creation. This simplifies Finite's per-agent Core logic but still leaves
us operating Substrate controllers, a worker pool, routing, metadata storage and
snapshot storage. It is not currently equivalent to a managed Fly service.

Substrate explicitly describes itself as early development, without stable APIs
or production guarantees. Snapshot success is not independent backup proof.
Deleting an actor can delete its owned snapshot; ordinary sleep must use suspend,
not delete. No production agents, chat databases, rollout topology or Core schema
are changed by this spike. No migration/compatibility or empty-target recovery
claim is made from synthetic two-agent tests.

## Message-triggered SimpleX variant

```
SimpleX human -> encrypted SMP network -> per-owner SimpleX daemon (PVC)
                                            |
                                single WS reader + SQLite event queue
                                            |
                                  HTTP request wakes the actor
                                            |
                    authenticated relay WS -> Hermes SimpleX adapter
                                            |
                                  model reply -> SimpleX human
```

`simplex-relay.py` keeps exactly one reader on the daemon. SimpleX's upstream
[WebSocket server](https://github.com/simplex-chat/simplex-chat/blob/stable/apps/simplex-chat/Server.hs)
uses a shared event queue, so multiple independent readers steal events from
one another. The relay keeps the authenticated administration channel separate
from the Hermes event channel, persists events before forwarding, and accepts
adapter acknowledgements. Network-status chatter and outbound messages do not
wake the actor. Pending inbound events trigger the standard Substrate HTTP path;
there is no new Core scheduler, placement table, or IP bookkeeping.

The small `hermes-simplex-relay.patch` adds an optional bearer token, acknowledges
relay deliveries, handles the pinned daemon's `receivedContactRequest` event,
and uses `/_accept` without waiting on a response from inside the receive loop.
The last two changes repair reproduced contact-setup incompatibilities. Ordinary
Hermes owner pairing still authorizes the sender; no allow-all is enabled.
This experimental extension only qualifies text messages. Attachments would
need a deliberate file-transfer boundary between the transport and actor.

Build with `build-relay-image.sh` using `SPIKE_STATE_DIR`, `SPIKE_HERMES_SOURCE`
(the pinned checkout), `SPIKE_BASE_IMAGE` (baseline image digest), and
`SPIKE_IMAGE` (local output tag). The script was exercised and reproduced the
exact accepted image digest. The Docker overlay deliberately targets the exact
ARM64 Nix store paths in this spike; a production package should build the
patch into a new Nix derivation.

Run `render-simplex.py --state-dir STATE --image DIGEST --key-file KEY_FILE`.
Apply its `simplex-transports.json`, create its two `*-simplex.template.json`
templates, wait for golden tags, and create `alice-simplex` and `bob-simplex`.
Run the egress fixture for those names. Apply `simplex-ingress.json` and restart
only the local ingress deployment. `owners-simplex.json` selects this variant
for `agent.py --owners ...` and `qualify-chat.py --owners ...`.

Each transport is a two-container Deployment with one per-owner PVC and token.
`gateway-start.py` waits for the external transport after resume, so golden
snapshot construction does not require external networking or prematurely
connect an owner's transport. `/home/agent` stays the actor's durable home.

For test users, run the same image with `--entrypoint simplex-chat`, a fresh
`-d` database prefix, `-p 5225`, `--mute`, and a unique `--user-display-name`.
Copy `simplex-command.py` into `/home/agent/` in each disposable user container.
It only talks to that container's loopback daemon. Forward Alice's relay service
to loopback 18765 and Bob's to 18766. `qualify-simplex.py` takes `--state-dir`,
`--output`, and optionally `--user bob --relay-port 18766
--human-container finite-simplex-test-bob`. Repeated runs after native pairing
use `--paired`. No actor HTTP request is made by the harness during its measured
message-wake cycles. `qualify-transport-restart.py` additionally tests an offline
transport and restored identity while the actor stays asleep.

## State ownership and unproven failure edges

| State | Writer / reader | Persistence in this spike |
| --- | --- | --- |
| User-to-actor ownership | trusted fixture / `agent.py` | checked-in stub rows |
| Chat history, pairing grants, agent files | Hermes serve and gateway | actor home in Substrate snapshots |
| SimpleX identity and message database | SimpleX daemon | per-owner transport PVC |
| Undelivered adapter events | relay / adapter ACK | SQLite on the same transport PVC |

The recovery set now spans **both** the actor snapshot and transport PVC. No
empty-target restoration of that whole set was performed. Local PVC restart
survival is not a backup, and transport PVC deletion must never follow compute
teardown. Relay ACK currently means the adapter accepted the event, **not** that
the resulting model turn committed durably. Lost ACKs can replay a delivery;
process crashes after early ACK can require recovery from SimpleX history.
Deduplication, durable turn acceptance, admission/rate limiting, cross-tenant
network restrictions, file attachments, capacity exhaustion, and idle-based
suspend arbitration remain production work. Tests suspend only after replies
complete; they do not establish safe automatic suspension during a live turn.

The baseline supervisor also shuts down the actor if a required child exits.
A deliberate SimpleX `/_stop` diagnostic exits this CLI build; that experiment
required reverting a disposable actor to its previous snapshot. Do not use
that command as a production reconnect strategy. The external variant avoids
coupling SimpleX daemon failure to the Hermes process supervisor.
