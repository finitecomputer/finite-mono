# Substrate runner — phase 1 candidate

Under qualification; no production cutover or default admission change has been
made. New agents are intended to run continuously. Automatic sleep and SimpleX
notification wake are outside phase 1.

## Authority and compatibility

Core retains users, ownership, billing, admission, RuntimeSpec, current/target
artifacts, and its provider-operation journal. Substrate owns actors, scheduling,
workers and snapshots. The Rust adapter uses native gRPC, not kubectl; protocol
provenance is in `finite-saas-runner/proto/README.md`. There is no second routing
registry or operation journal.

The integration uses the existing Runner process, one shared ingress, a stateless
gRPC client and a boot-time environment fetch. There is no per-agent transport
pod, Finite worker registry, parallel lifecycle queue or credential-refresh daemon.
The source-of-truth audit does not justify deleting the Core lease/journal fences:
Substrate observations cannot authorize an owner stop reversal or change ownership.

The operational cost is not limited to this Rust adapter. Six checked-in provider
patches affect production behavior; the seventh is local CSI-only. The qualified
cluster also retains spike-specific provider/egress changes. Stock managed egress
does not yet satisfy SimpleX parity. Qualify a reproducible supported provider
build and raw-TCP path before describing this as a low-operations replacement.

Use the existing `FC_CORE_AGENT_CREATION_PLACEMENT_JSON` setting for staging
admission and `FC_RUNNER_DRAIN` for capacity/drain. Existing placements stay on
their existing runner and chat transport. Expand **every Core reader** to support
`substrate` before admitting Substrate rows. Migration 0033's populated-state
proof preserves existing Kata project, creation, runtime and provider-operation rows,
rejects provider writes before expansion, and accepts them after expansion and
replay. This is schema evidence, not old-binary compatibility: a Core binary
that cannot parse `substrate` is not a valid rollback after those rows exist.
Migration 0034 permits `requested_by_user_id: null` only on restart records to
identify Core-authorized recovery. Owner/operator requests still serialize their
existing user ID. Upgrade all Core/Runner readers before admitting these records;
an old binary requiring a non-null requester is not a compatible rollback.
The populated-state Postgres test also proves 0034 preserves owner-attributed
requests through replay, allows existing owner writes afterward, and rejects
unattributed stop/destroy/upgrade/recovery operations. It exercises the previous
non-null SQL reader against a system record and observes its decoding failure;
this establishes the reader boundary, not a complete mixed-version fleet proof.
The existing Core store replay regression also runs for Substrate: full startup
schema replay preserves its pending placement, repairs a mismatched legacy runner
field from that placement, and leases the request to the matching adapter.

The current dashboard also passes the retained-chat browser proof against a Core
response that omits `native_hermes_chat`: history, multi-tab isolation, reconnect
and predecessor-service snapshots remain usable, with zero Hermes-access requests.
The explicit native opt-in browser proof passes separately. Evidence:
`legacy-transport-browser.log` (3 tests) and `native-transport-browser.log` (1 test).
These exercise predecessor wire shapes through the actual dashboard; they do not
establish a mixed-binary production fleet or authorize rollback after new rows exist.

The real-Postgres runtime-control lifecycle test now gives wrong-host and
wrong-provider callers valid restart capabilities. It rejects Phala/Substrate
callers for a retained Kata runtime, then proves its matching Kata runner can
still lease and complete the request (`mixed-provider-lease-qualified.log`).
Previously those negative cases omitted capabilities and could return early
without exercising host/placement selection. Structure and strict Core Clippy
pass. This proves candidate-Core lease isolation, not predecessor-binary parsing.

A real predecessor Runner built from archived revision
`c1b66ba309e9bfd8b26d08a085c23451f69a87c9` also passes alongside current Core
and the Substrate Runner (`predecessor-runner-retry.log`, 150.68 seconds).
The local macOS binary SHA-256 is
`1cdb13b0a8843095692c973643c14dd7cd3d368bc303aeb28e1dd524a816b40a`.
Set `FC_TEST_SUBSTRATE_PREDECESSOR_RUNNER` to that binary and enable
`FC_TEST_SUBSTRATE_AUTO_RECOVERY` with the documented local worker-loss flags.
The old Docker adapter uses its own class-scoped credential on the same host,
reads the current artifact and polling routes, and returns idle while each
Substrate creation is pending and while a null-requester recovery is queued.
The current Runner then completes the work. The full two-owner native chat,
worker-loss recovery, stop/restart, history/files/desktop and isolation checks
pass; both actors are suspended with data retained and final fleet status is
healthy. This is real old-Runner/new-Core queue coexistence, not old-Core rollback,
a deployed Kata/Phala qualification, or a legacy agent launch/chat proof.

## Local node-restart recovery

Restarting OrbStack during qualification changed worker pod IPs without changing
pod UIDs. At provider pin `bb0effe`, `workersync.createOrUpdateWorker` detects the
mismatch but only logs it: registered worker IPs are immutable. A new actor was
reported RUNNING at worker IP `10.244.0.33` while its Kubernetes pod was at
`10.244.0.16`; the agentd ingress returned 503 with a connection timeout. The
second idle worker also retained its pre-restart address. Deployment convergence
alone therefore does not prove usable worker routing.

`requester-dispatch-browser-proof.log` is an interrupted run, not a pass. Its
actor `runtime-e578240cfdfe5e1a3627` was suspended with data retained before the
stale worker pods were replaced. New pod UIDs registered current addresses; the
combined browser retry uses those workers. This is manual local infrastructure
recovery, not automatic whole-node recovery. The canonical status command now compares registered worker/pod UIDs and IPs
using `kubectl ate get workers` and the Kubernetes worker pods; a mismatch,
missing pod or empty worker inventory cannot report healthy. The `kubectl ate`
plugin is required for this read-only probe. Qualify provider-supported
reconciliation before accepting a deployment as a supported recovery boundary.
Do not repair this with a second worker registry in Core or Runner. The provider
patch and later sandbox/IP-drift and whole-node qualifications are recorded below.

## GKE staging boundary

Rechecked against [Google's installation guide](https://docs.cloud.google.com/kubernetes-engine/ai-ml/install-overview-substrate)
on 2026-09-23: evaluation is available, but production support requires the
private-GA allowlist. Autopilot is unsupported; use Standard with the documented
certificate APIs and C3-class nodes. The installer deploys its own PostgreSQL
workload; GKE does not turn that database into a managed backup service. New
nodes also need the installed Substrate version label at the node-pool level.

Project, region/zone, credentials and spending limit must be supplied before
creating staging resources. Use the existing `runtime-image.yml` non-production
CI lane for an amd64 image; the local arm64 digest is not a C3 deployment image.
Pin installer/provider revisions and image digests, retain the independent
Recovery Set, and qualify the existing patch set and egress before admission.
No cloud resources or production placements have been changed by this branch.

## Runtime configuration and processes

Public flags come from Core's existing `FC_CORE_RUNTIME_ENV_JSON`, through
`GET /api/core/v1/runtime/environment` on its dedicated runtime listener. The
no-store response excludes secrets and derives the creation's owner allowlist.
There is no new configuration table. `finite-agentd bootstrap` fetches flags
before invoking the canonical runtime entrypoint. Updates apply on the next
cold boot; a failed fetch never falls back to cached flags. Existing runners
continue consuming their persisted RuntimeSpec environment.

`environment-removal-proof.log` qualifies next-boot updates and removals on the
current image: both agents write the real environment through terminal tools,
and authenticated downloads verify the changed value and unset removed flag.
Installed templates remain unchanged. This is not hot reload.

The initial fetch authenticates the creation credential and live lease; later
boots require the current assignment. A stopped assignment may fetch boot flags
only during its exact live, leased restart/upgrade operation. This does not
activate chat early. Queued/expired operations and revoked credentials remain
unauthorized. Successful completion activates the existing runtime credential.

Each immutable artifact template has three environment entries:
`FINITE_CORE_URL`, `FINITE_CORE_CREDENTIAL`, `FINITE_BOOTSTRAP_ENV_JSON`. These
include private bootstrap material; never log them. Public flags are not copied
into the template. Interrupted creation validates the installed credential,
image and agent identity before reusing the inference key; finding an actor
must not mint another key.

Run the existing `finite-saas-runner serve` with `FC_RUNNER_CLASS=substrate` and:

- `FC_SUBSTRATE_ENDPOINT`, `FC_SUBSTRATE_SERVER_NAME`
- `FC_SUBSTRATE_CA_FILE`, `FC_SUBSTRATE_TOKEN_FILE` (projected Kubernetes identity)
- `FC_SUBSTRATE_ATESPACE`, `FC_SUBSTRATE_BASE_TEMPLATE`
- `FC_SUBSTRATE_EGRESS_HOSTS`, `FC_SUBSTRATE_EGRESS_CIDRS`
- `FC_SUBSTRATE_RUNTIME_ORIGIN` (private ingress), `FC_RUNNER_SOURCE_HOST_ID`

The adapter rereads projected credentials per operation. It installs explicit
egress policy before bootstrap and preserves an existing policy on retry.
The pinned non-MITM gateway applies hostname rules only to cleartext HTTP; TLS
requires destination CIDRs. Local Core/inference and SimpleX relay IPs are
fixture-time allowlists, **not production DNS refresh or internet-tool parity**.
There is no implicit allow-all policy.

Upstream's [GA egress contract](https://github.com/agent-substrate/substrate/blob/6ec93a45/docs/egress-traffic.md)
allows HTTP(S) but explicitly blocks WebSocket, forward-proxy CONNECT and other
TCP. The local SimpleX proof depends on the custom non-MITM egress fixture;
it does not establish SimpleX support in the stock managed installation. GKE
qualification must retain and operate a supported raw-TCP path or resolve this
upstream limitation before admitting agents with the promised transport parity.

The base template requires CSI-backed `/data`, DATA suspension snapshots and
COLD_BOOT resume. An empty durable-dir mount such as `/run/finite-checkpoint`
is also required by pinned gVisor's DATA implementation; it is not a duplicate
agent data store. Core's CPU/memory limits apply to the actor. Golden actors on
older providers are detected through `/run/ate/atespace`: bootstrap exposes
readiness only and never enrolls an identity or starts user chat.

`FINITE_DESKTOP_ENABLED=1` uses agentd's existing supervisor for Xvfb/Openbox.
The canonical image includes `scrot` and Chromium; tools use `DISPLAY=:99`.
There is no public desktop listener.

## Ingress and native chat

`finite-saas-runner substrate-ingress` has separate public Hermes (8642) and
private agentd (8080) listeners. Expose only the public listener externally;
terminate TLS at the cluster edge. Both proxy their service's complete surface.
`/runtimes/runtime_<20 lowercase hex digits>/...` maps deterministically to
`runtime-<same suffix>`. Each request opens an actor-bound CONNECT tunnel;
incoming actor headers cannot change its target. No per-agent Caddy setup is
required. Native Hermes remains the authentication and authorization authority.

Repeat `--allowed-origin https://<dashboard-origin>` for permitted browser
origins. The existing strict origin validator permits HTTPS or explicit loopback
HTTP for development; an empty list disables cross-origin browser access.
The public ingress answers preflights and replaces upstream CORS headers, without
cookie credentials. Origin and Authorization pass through on actual requests;
native authentication responses are preserved. The private listener has no CORS
policy. This is necessary because native auth rejects unauthenticated preflights
before Hermes's localhost CORS middleware.

Core's explicit runtime capability selects native chat. The dashboard obtains
short-lived Core grants and single-use native WebSocket tickets, reusing the
native gateway UI work from PR #845. New-user navigation resolves the authorized
transport before mounting a history provider. The native unscoped/Recents bucket
uses Finite's shared Home topic identifier, so New chat and other shared actions
have a valid target even for an empty agent. This is a UI projection, not a
persisted Hermes topic. Hermes alone owns history,
approvals, clarification and in-flight turn state:

- History reads all 100-row pages in creation order, deduplicates IDs and publishes
  only a complete read. Concurrent insert/delete may still require refresh;
  offset pagination is not a snapshot. Archive/restore uses native durable state.
- Reconnect restores the running turn, accepted user row, reply prefix, pending
  approval and pending clarification. It never replays a prompt or answer.
  Failed turns retain their partial response; unrelated events cannot replace
  the selected conversation's error state.
- Stop response sends native `session.interrupt`; only the terminal event settles
  the turn. Failures remain visible and are not automatically retried. Unsent
  text/files remain local.
- Approval responses name the exact session/request and use offered choices.
  The pending snapshot preserves queued requests. Clarification answers name
  their exact question and rely on native remaining-question state. Single-choice
  and multi-select controls preserve custom answers; JSON encoding keeps commas
  inside a choice intact. Native expiry removes only its matching request.
- Files stage with `file.attach` and the returned path is submitted with the
  caption. Failed staging preserves the draft. Downloads obtain fresh Core
  authorization, keep tokens out of URLs and sandbox documents on the dashboard
  origin. Persisted `@image:`/`@file:` references and native generated `MEDIA:`
  directives render as attachment cards. Inline/standalone generated references
  and quoted filenames use the same authenticated file route; fenced/inline
  code, remote URLs and invalid paths remain literal text. Browser fixtures
  cover generated documents in restored history and live replies.
  Images use the same staging call and a turn-scoped `@image:` reference;
  Hermes invokes its vision tool. The client never fills the native session-wide
  pending-image queue or implements model-routing policy. Live browser upload,
  vision reply, rendered image and page-reload history have passed locally.

Hermes's steering-history patch preserves accepted corrections through the
existing transcript writer. The upstream tool handoff mutates a row already
marked persisted; Hard Stop also clears an accepted model redirect before saving
it. `hermes-steer-history.patch` appends tool corrections as user rows instead,
and transfers canceled redirects into the existing steering buffer for the
locked writer to save as stopped input. If the model loop has already consumed
and saved the redirect, Stop closes the trailing user row with the same
cancellation marker so a subsequent request cannot merge with it. It closes tool sequences for strict role
alternation and never requeues canceled work. No Finite history store, additional
database writer, or new queue is introduced.

`scripts/proofs/hermes-steer-history.py` covers ordinary steering, stopped tool
steering, stopped model redirects and the consumed-redirect/Stop race in scratch
SQLite. The assertions require exactly-once storage, unchanged earlier rows,
drained buffers and unchanged Stop state. The sealed package passes all four
cases; the actual dashboard/provider proof also passes correction, clarification,
file/image, reload and restart checks. Current combined-image evidence is listed
under Product identity parity; older intermediate image digests are in git history.

## Lifecycle and recovery

Core-authorized restart uses RevertActor then ResumeActor for CRASHED,
REVERTING, RESUMING and SUSPENDING actors. It verifies actor identity, CSI `/data`
and DATA/cold-boot policy first. Revert releases interrupted compute while
retaining CSI data; it is not backup restoration. The existing Runner loop scans
paged Substrate inventory for CRASHED actors and asks Core to enqueue at most one
recovery each cycle, after leasing any already-queued control. A pending owner
stop therefore skips recovery inventory; a newly admitted recovery is leased in
the same cycle, including during admission drain. Core requires this host's active Substrate runtime, completed
creation, an online lifecycle latch, no offboarding, and no pending control. The
same durable control lease performs recovery; no second desired-state registry or
per-agent controller exists. New-admission drain does not stop this maintenance.
A delayed observation of an already-running actor does not interrupt it. Explicit
stops and failed/uncertain controls remain fenced; an exhausted recovery attempt
requires operator intervention. Anonymous ingress cannot undo a Core stop.
An actor can also crash after provider creation but before Core records launch
completion. A later creation lease reuses the same journaled actor and applies
the same CSI/cold-boot-validated Revert-then-Resume sequence for CRASHED or
REVERTING state. It leaves RUNNING/RESUMING actors on the normal idempotent resume
path. This does not shorten leases after ambiguous failures.
Resume retries ResourceExhausted once per second up to 30 times; other errors,
including ambiguous transport failures, are not blindly retried.
A creation that exhausts these retries remains `launching` in Core. Runner asks
Core to expire only its current creation lease, preserving the runtime identity,
installed credentials and provider journal, so the next polling cycle can retry.
The release requires Substrate Runner authority and the active lease token;
expired tokens cannot release a successor lease, and legacy placements cannot use
this path. Ambiguous transport failures retain their original lease. An older
Core that rejects the additive release endpoint also retains the lease (Runner's
default is 600 seconds), without failing creation or deleting credentials.
Real-Postgres and Runner tests cover release, fencing and old-Core rejection.
Local capacity exhaustion qualifies retry without changing creation identity,
credential, provider correlation, spec or actor UID. The observed launch after
freeing capacity was 3.93 seconds; this is not a GKE latency guarantee.
`stalled-boot-before.log` qualifies transient bootstrap HTTP failure followed by
Runner loss and natural lease expiry. The same creation resumes after the endpoint
recovers; production retains its configured lease (600 seconds by default).
Worker loss and whole-node restart evidence are recorded below.

Invalid central boot configuration is also qualified on the requester-dispatch
image (`invalid-boot-proof.log`, 329.55 seconds). The real authenticated Core
endpoint supplies a non-loopback `FINITE_AGENTD_BRIDGE_ADDR`; worker logs confirm
the daemon rejects it. Runner returns failure naturally while Core retains the
launching creation. Correcting Core's configuration and allowing the normal
lease retry recovers that creation without changing its runtime ID, spec,
credential hash or provider correlation. A before/after provider comparison also
confirmed unchanged actor UID and durable volumes. Both owners then pass native
chat, files/history, desktop, isolation and stop/restart checks. No production
recovery code was added. This qualifies a correctable central flag, not an
invalid immutable image or a lost credential. The failed RPC can consume the
300-second client deadline; production also retains its default 600-second lease.
Startup failure remains visible in operator logs rather than a terminal Core
creation error. Do not delete the actor, revoke its credential or create a
replacement request to repair this case: correct Core's environment source and
let the existing Runner retry. Fast diagnosis and user-facing startup status
remain rollout concerns.

Upgrades remain behind `FC_CORE_ENABLE_RUNTIME_UPGRADES`. A template name derives
from runtime and immutable Core artifact ID. Clone the actor's installed template,
not the operator's latest base, preserving storage and bootstrap identity.
Suspend/revert, replace the template with required actor UID/version preconditions,
then resume and check readiness. After ambiguous replacement, reread the same
actor; stale CAS fails closed. Core completion preserves Substrate's internal
contact endpoint and requires stable runtime host/published URLs.

On target failure, restore the previous template on the same actor/CSI data and
check readiness before reporting failure. Core's current/target artifacts remain
the authority for retries. Ordinary restart rejects an uncommitted template;
stop stays allowed. Repointing an image **does not undo writes** the failed image
made to the shared volume. `interrupted-completion-provider-proof.log` qualifies
Runner loss after provider success and before Core completion, followed by natural
lease expiry and completion of the same request.
Retirement and known-good backup recovery remain unadvertised.

The pinned provider does **not** supply an external-volume backup/restore path.
Its [CSI design](https://github.com/agent-substrate/substrate/blob/bb0effed188e06a44e03862cb6ea993e58f86893/docs/csi-volumes.md)
provisions volumes directly, without PV/PVC objects; PVC-oriented backup coverage
must not be assumed. The pinned volume plugin exposes create/delete/attach/detach,
not snapshot/restore. `CreateActor` rejects tag cloning with external volumes and
initializes fresh volume records; `UpdateActor` discards supplied status. Replaying
an exported actor through these APIs cannot restore its `storageVolumeId` or UID.
These boundaries are in the pinned
[actor API](https://github.com/agent-substrate/substrate/blob/bb0effed188e06a44e03862cb6ea993e58f86893/cmd/ateapi/internal/controlapi/actor.go)
and [volume interface](https://github.com/agent-substrate/substrate/blob/bb0effed188e06a44e03862cb6ea993e58f86893/internal/volume/plugin.go).

For empty-target qualification, the Runtime Recovery Set must include the entire
quiesced `/data` tree (identity, Hermes history/files, SimpleX state and other agent
state), Substrate snapshot objects (including the referenced manifest even for
DATA/cold-boot actors), the exact image and installed template, and the Core ownership/credential
and provider-operation bindings. Provider metadata must retain the actor UID and
volume mapping or be replaced through an explicitly authorized, identity-checked
rebinding contract. The independent Recovery Authority must be able to obtain the
backup and necessary decryption/bootstrap secrets after losing the source cluster.
Templates contain credentials and belong in protected recovery storage, never logs.

Substrate persists actor UIDs, full actor protobufs (including volume bindings),
templates and authorization state in Postgres. Restore that authority with its
storage; do not introduce a Core copy of provider bindings or a per-agent rebinding
API. The provider outbox uses PostgreSQL transaction IDs, so the qualified drill
uses a physical backup with WAL, verified by `pg_verifybackup`. A logical restore
alone does not prove watch continuity.

The two-owner empty-cluster recovery proof passed on 2026-09-23 in 591.69s
(`finite-substrate-restore/drill-5/proof.log`, private local evidence). It used the
pinned provider and canonical runtime plus fixture CA. Before teardown, both
owners passed native chat, image/file handling, desktop capture, SimpleX inference
and repeated cross-owner denials. After suspending the actors, the drill retained:

- A verified physical provider Postgres backup and an independent Core backup.
- The complete CSI data/state directory, including both agents' `/data` trees.
- The fenced local snapshot-store volume; both referenced manifest paths were
  checked in its archive before teardown.
- Exact actor metadata, image/template references and archive SHA-256 checksums.

It deleted and recreated only `kind-finite-hermes-restore`, installed the backups
into fresh target directories with their writers stopped, then verified both
original UIDs, templates, volume bindings and snapshot references. Normal Core
owner controls restarted both agents. Both recovered native history, exact file,
image and screenshot bytes, and accepted new native model turns. Their existing
SimpleX addresses and approved contacts survived, with real inference replies for
both. Both agents read the updated central Core environment value after cold boot.
Alternating native grants still returned 401 against the other agent in both
directions after both were running. Canonical fleet status was healthy before
and after; completed fixture agents were suspended with their data retained.

**Snapshot objects are required even for DATA/cold-boot actors.** Omitting the
object store in the negative drill caused `DataLoss` before Hermes started.
`RevertActor` preserves the external snapshot reference and cannot repair missing
objects. **Run upstream CSI setup before restoring data:** its setup command
clears the driver data directory. Fence the target API/controller, CSI driver and
snapshot-store writer while installing their coordinated backups. Do not use the
destructive local setup command on an existing source cluster.

`FC_TEST_SUBSTRATE_RECOVERY_DIRECTORY` pauses the existing two-owner proof after
initial native history verification. A fresh private directory receives mode-0600
`ready.json` containing the Core database URL, runtime IDs and a per-run nonce.
Writing that nonce to `resume` releases the barrier; a stale marker or 15-minute
timeout fails the test. Cross-agent isolation runs after both owner restarts and
requires 401, never a permissive acceptance of 503. This is test coordination,
not a product restore API. The test drops its isolated Core database on completion,
so capture recovery evidence inside that lifetime.

Limits: Core survived outside the simulated failure domain; its backup was retained
but not restored in this live drill. This does not qualify a full Core/identity
outage, populated provider authorization policy, automatic node-loss failover,
GKE storage recovery or a production backup service. Keep the independent Recovery
Authority and qualify those deployment-specific recovery boundaries before rollout.

## Product identity parity

Native chat does not mount the legacy Device UI, but Brain signing still uses
that Device's durable human key authority. A signer setup-required response (428)
initializes the existing state endpoint and retries once. Other errors, including
incomplete durable state, do not initialize or retry. Real Next approval routes
qualify fresh-user signing, wrong-nonce rejection, persisted membership/resolver
identity and replay denial. The current `fbrain` CLI no longer produces approval
requests or chat-card trailers; do not restore that removed workflow. Keep the
trailer reader for persisted history. Card rendering has separate browser fixtures.

The Brain CLI accepts native v2 requester leases and retained v1 leases. A present
invalid or expired v2 lease rejects without falling back to v1. Native identity
comes from the exact hosted human/Project binding even when Sites is unavailable;
optional Sites claims are an all-or-nothing pair. Native turns only write v2 leases,
bounded by requester/assertion expiry, through the existing terminal-tool writer.
Different requesters cannot steer a running turn or merge queued envelopes.

The Sites handoff reuses the registry-issued assertion binding verified mailbox,
human principal and exact agent. The dashboard obtains fresh context per prompt;
Hermes carries it outside prompt text/history and scopes it to the executing turn.
Sites remains authoritative when `fsite` submits Project Init. A wrong agent cannot
consume the assertion. Completion and dispatch failure clean up the lease. Requester
assignment precedes dispatch so immediate completion cannot leave stale authority
or overwrite a queued successor. Nine sealed-runtime regressions cover these
boundaries on macOS and Linux, including gateway-first imports. The sealed package
materializes the gateway directory to prevent upstream import-path shadowing.

Current combined evidence is `requester-dispatch-browser-retry.log` (204.18s), on
canonical image plus only the local CA:
`sha256:10ba77158eb9d75b325c26a41ece330483025ce386c19b56c63bb44915ba954a`.
Both owners pass actual Next onboarding, native chat/reload, browser-to-terminal
Brain creation and independent human access, Sites attribution, approvals,
clarifications, interruption, reconnect, desktop, configuration refresh,
retained history/files and cross-owner denial before/after restart.

Enable the actual browser/Brain proof with `FC_TEST_SUBSTRATE_BROWSER=1`,
`FC_TEST_SUBSTRATE_DASHBOARD=1` and `FC_TEST_SUBSTRATE_BRAIN_BINARY`; the disposable
Brain service listens on 18430. Its local-only Node relay inside the agent forwards
loopback HTTP to the host and closes after `fbrain` exits. The CLI uses bundled
public CA roots, so this does not qualify public-CA HTTPS. No relay or weakened
TLS policy is added to the runtime image. WorkOS/deployed-origin parity remains open.

For the focused native terminal/Sites boundary, build `fsite`, then run:

```sh
HERMES_AGENT_PYTHON=<pinned-python>/bin/python3 \
FSITE_TEST_BINARY="$PWD/target/debug/fsite" \
scripts/with-dev-env cargo test --locked -p finitesitesd --test e2e \
  native_hermes_terminal_initializes_sites_with_scoped_requester -- --ignored --exact
```

## Worker IP drift recovery

`0007-replace-workers-after-ip-change.patch` addresses the reproduced restart
failure at its provider owner. Current upstream `d277088b` still only logs IP
drift. The patch drains the registered worker, then gracefully deletes the
observed pod incarnation. It does not change the immutable registered IP or
release actor assignments while the pod is present. Kubernetes UID and resource
version preconditions protect a replaced pod or a stale observation; deletion
errors remain retryable. The existing Deleted-event path owns actor release.

`worker-ip-before.log` reproduces the stuck active worker on the pinned source.
`worker-ip-controller-tests.log` passes every controller package, and
`worker-ip-final-checks.log` covers generated RBAC, worker race tests and vet.
Negative cases retain the worker on deletion rejection and forbid deletion when
drain fails. These are controller tests, not proof that a Kubernetes deletion
always fences a partitioned node. Do not force-delete workers as part of this
recovery.

The patched controller is installed only in the disposable kind cluster as
`localhost:5017/restore/atecontroller@sha256:d31836bcca8be9892d2c0c0cb6116c1341e8e51c4d16a776c0d3413b5f24004c`.
`worker-ip-live-proof.log` passes the two-owner proof in 160.19 seconds. The
fixture stops the exact worker's CRI sandbox, leaving Kubernetes to recreate its
network. A separate pod watch records the same UID moving from `10.244.0.23` to
`10.244.0.26`, followed by controller deletion and a replacement pod at
`10.244.0.27`. An ordinary Runner cycle under admission drain recovers the actor
with unchanged UID and durable volumes and a single Core-attributed recovery
operation. Both owners pass native chat, files/history, desktop, configuration
refresh and isolation across recovery/restart. No manual pod deletion or actor
replacement is part of the fault path. Both test actors were suspended afterward
with data retained; canonical platform status is healthy.

The separate whole-node drill also passes (`node-restart-proof-1.log`, 286.05
seconds for the complete proof including the manual barrier). After both owners
have verified native history, restart only the Docker container hosting kind's
single control-plane/worker node. This takes Substrate's API, controller,
Postgres, snapshot store, network proxies and workers down together. Kubernetes
returns Ready first; projected certificate reissuance gates service startup.
The patched controller replaces both stale worker pods and marks their actors
CRASHED. Restore the host's disposable port-forwards, then release the nonce-bound
test barrier. Two ordinary Runner cycles under admission drain recover both
actors with unchanged UIDs and volumes and exactly one system-attributed restart
per runtime. Owner restart/stop commands are skipped in this mode: the next
native turns, retained files/history, desktop and cross-owner denials must pass
after automatic recovery alone. Both actors are suspended with data retained at
the end; canonical platform status is healthy.

Core, identity and the test coordinator remain alive on the host, and the node's
local disks survive the restart. Host port-forward restoration is test transport
repair, not product recovery. This does not establish Core/identity loss,
partition fencing, GKE CSI recovery, disk loss, or production recovery latency.

## Pinned provider dependencies

These are local qualification dependencies, not guarantees from an upstream
release. The cluster also contains the original spike's atelet cleanup changes
and custom egress fixture; repeat qualification against the intended release.

| Patch | Contract and evidence |
| --- | --- |
| `0001-egress-before-readiness.patch` | Enable authenticated egress before bootstrap/restore, while ingress waits for readiness. RESUMING is allowed only under the existing UID/policy; failure cleanup disables access. |
| `0002-connect-actor-isolation.patch` | Actor and port filter state participate in Envoy pool hashing. Required for tenancy; the original string factory leaked actor routing identity. Passed 1,000 alternating requests from 20 concurrent clients; keep repeated cross-agent denials. |
| `0003-ingress-explicit-resume.patch` | Enable `--ingress-disable-resume` on the router. Only RUNNING actors route; other states return 503 without calling ResumeActor. Live suspended-state and anonymous-access checks pass for both listeners. |
| `0004-local-hostpath-set.patch` | Local CSI v1.17.1 fixture only: treat publish paths as a set and remove persisted duplicates on unpublish. Regression fails before/passes after; clean stop works after worker loss. Build with `-X main.version=v1.17.1-finite-local-set-fix`; an empty version is rejected by sidecars. |
| `0005-revert-interrupted-lifecycle.patch` | Permit Revert from RESUMING/SUSPENDING under the existing actor lease. State regressions and full control API suite pass; real bad-image recovery passes. |
| `0006-skip-unused-golden-snapshots.patch` | Cold-boot templates never consume golden state. Skip unused warmup, which otherwise strands capacity on a bad image. Regression fails upstream/passes patched; full control API suite passes. Existing golden fixtures require explicit cleanup. |
| `0007-replace-workers-after-ip-change.patch` | Locally qualified: drain a worker after pod IP drift, request graceful pod deletion with UID/resource-version preconditions, and retain its actor assignment until the existing pod-deletion reconcile runs. Requires controller pod-delete permission. Regression fails on the pinned upstream code and passes patched; controller suites, worker race tests and generated RBAC pass. Real CRI sandbox recreation, IP drift and two-owner recovery pass; whole-node restart also passes with retained local disks and Core outside the failure domain; partition and GKE fencing remain unqualified. |

Hermes's pinned stream-writer guard still allowed a cancelled late opener to
supersede its replacement. `scripts/proofs/hermes-stream-writer.py` forces that
race in real AIAgent code without network: upstream emits only `fresh-`, the
patched package emits `fresh-complete`. `finite-agentd/patches/hermes-stream-writer.patch`
checks cancellation under the existing attempt lock before claiming the writer.
The sealed Linux Python package passes with bytecode writes disabled; other
provider paths are not established by this OpenAI-compatible regression. Database
writer consolidation is a separate mechanism.

The owner-control proof also exposed Finite Chat's received-cursor bug. Own sends
are skipped without advancing that cursor; a failing-before regression and 96 CLI
tests pass. This does not rewind cursors already advanced by an older binary.

## Reproduce local qualification

The ignored `substrate_two_owner_launch_and_native_access` test uses isolated real
Postgres, two authenticated account fixtures, the actual Rust Runner, real local
Finite Chat/Identity services and the canonical runtime image. Two actors reserve
2 CPU / 4 GiB each on a disposable 16-GiB node. Account Core is loopback 18420,
runtime Core 18422, Finite Chat 18426. Supply HTTPS termination for the latter two
and native ingress; never expose the account listener. A fixture CA belongs only
in the image/test process, never host-global trust. The public native hostname
must exercise native auth; `localhost` is rejected by the proof.

Required inputs: `FC_TEST_SUBSTRATE_IMAGE_DIGEST`,
`FC_TEST_SUBSTRATE_PUBLIC_ORIGIN`, `FC_TEST_SUBSTRATE_CORE_ORIGIN`,
`FC_TEST_SUBSTRATE_CHAT_ORIGIN`, `FC_TEST_SUBSTRATE_RUNNER_BINARY`,
`FC_TEST_SUBSTRATE_INFERENCE_KEY_FILE`, and the Runner settings above. Set
`SSL_CERT_FILE` / `NODE_EXTRA_CA_CERTS` for a fixture CA. No extra inference key
is issued; account/grant credentials travel over stdin, never URLs/argv. Long
fault injection renews the same synthetic account sessions; native HTTP operations
get fresh 60-second grants rather than extending production credential lifetimes.

```sh
scripts/with-dev-env cargo run --quiet --locked -p devfinity -- run -- \
  cargo test --locked -p finite-saas-core -p finite-agentd \
  substrate_two_owner_launch_and_native_access -- --ignored --nocapture
```

Optional gates:

- `FC_TEST_SUBSTRATE_DASHBOARD=1` with `FC_TEST_SUBSTRATE_BROWSER=1` uses the actual
  Next dashboard on loopback 18427 and exposes the real fixture Hosted Device on
  18428. Both fresh owners complete the launch-code wizard; browser chat and
  downloads use the actual Next routes without request interception. Account
  authentication uses the existing dev mode with Core-verified synthetic account
  details, so this does not qualify WorkOS login or a deployed origin. Next's
  generated config and output are isolated from the normal application config.

- `FC_TEST_SUBSTRATE_CRASH_KUBECONFIG` and `FC_TEST_SUBSTRATE_ATE_CLI` explicitly
  permit deletion of the first synthetic actor's worker, restricted to
  `kind-finite-hermes-spike` / `finite-hermes-spike`. Set
  `FC_TEST_SUBSTRATE_CONTEXT=kind-finite-hermes-restore` for the recreated local
  cluster; other context names are rejected. Require CRASHED, then Core
  restart, stop and restart again; check history/bytes/identity and stopped 503s.
- `FC_TEST_SUBSTRATE_SIMPLEX=1` and `FC_TEST_SUBSTRATE_SIMPLEX_BIN`: disposable
  human peers pair through exact owner-approved Device controls. Check real relay
  inference and stable addresses through lifecycle changes. Observe an unapproved
  peer for five seconds (bounded negative evidence). Never attach a second
  WebSocket consumer to the agent's managed SimpleX daemon.
- `FC_TEST_SUBSTRATE_UPGRADE_IMAGE_DIGEST`, `FC_TEST_SUBSTRATE_UPGRADE_MARKER`,
  `FC_TEST_SUBSTRATE_FAILED_IMAGE_DIGEST`: publish targets only after both base
  launches; require an actual image marker read, retained identity/volume records,
  history/files and recovery to the healthy artifact after a failing target.
- `FC_TEST_SUBSTRATE_INTERRUPT_UPGRADE=1` with a healthy upgrade target: pause
  the test HTTP completion request, kill the actual Runner after provider success,
  let its 60-second lease expire naturally, and reclaim the same request. Require
  unchanged actor/storage and old Core artifact until the retry commits. The real
  provider run passed in 186.43s (`interrupted-completion-provider-proof.log`).
- `FC_TEST_SUBSTRATE_AUTO_RECOVERY=1` with the crash kubeconfig and CLI: delete
  one worker and require an ordinary Runner cycle to recover it under admission
  drain without an owner request. Check Core system attribution, host/control
  fences, duplicate admission, delayed-observation idempotency, and stopped actors
  remaining suspended through later cycles.
- `FC_TEST_SUBSTRATE_NODE_RESTART=1`, a fresh
  `FC_TEST_SUBSTRATE_RECOVERY_DIRECTORY`, and local CLI/kubeconfig: wait for
  `ready.json`, restart the disposable kind node, restore its test port-forwards,
  verify canonical status and both actors CRASHED, then copy the private
  `resumeNonce` into `resume`. The fixture records actor/volume identities before
  the barrier, recovers both agents through normal Runner observation under
  admission drain, and skips owner controls before post-recovery native checks.
  Do not combine this mode with other worker/crash fault options.
- `FC_TEST_SUBSTRATE_RESTART_SANDBOX=1` with automatic recovery and the local
  fault CLI/kubeconfig: restart only the selected synthetic worker's CRI sandbox
  in `kind-finite-hermes-restore`, rather than deleting its pod. Requires the
  patched controller and its pod-delete permission. Capture a pod UID/IP watch
  to verify actual IP drift; require unchanged actor/volume identities and the
  ordinary two-owner chat/restart gates after automatic recovery.
- `FC_TEST_SUBSTRATE_INFLIGHT_CRASH=1` with automatic recovery: after the first
  owner's normal protocol checks, start a foreground terminal command that writes
  one marker and waits. Verify its side effect before killing the worker. Require
  automatic recovery alone (no owner restart), exactly one retained user prompt,
  an idle resumed session, a successful subsequent turn, and unchanged marker
  bytes before and after that turn. The second owner still exercises stop/restart.
- `FC_TEST_SUBSTRATE_STALLED_BOOT=1`, with the local fault CLI/kubeconfig:
  fail bootstrap environment fetches, kill the Runner while the actor is
  RESUMING, restore the endpoint, and retry after natural Core lease expiry.
  Assert durable creation identity and run the ordinary two-owner parity gates.
- `FC_TEST_SUBSTRATE_INVALID_BOOT=1`, with the local fault CLI/kubeconfig:
  serve a non-loopback `FINITE_AGENTD_BRIDGE_ADDR` through the real authenticated
  Core environment endpoint. Let the daemon reject it and Runner return naturally;
  require creation to remain retryable. Correct Core configuration, wait for the
  lease to expire, then run the same two-owner parity gates on the same creation.
- `FC_TEST_SUBSTRATE_CAPACITY_HOLDER`: a suspended disposable actor in the local
  two-worker pool, using a readiness-capable template independent of Core. Supply
  the same CLI/kubeconfig variables used for worker fault injection. The proof
  resumes this holder after the first owner launches, exhausts capacity for the
  second owner, then suspends the holder and checks immediate retry and identity
  continuity. Do not use an old Finite agent whose bootstrap credential belongs
  to a deleted test database: that initial fixture attempt stalled at readiness
  and was canceled; it did not qualify capacity recovery.
- `FC_TEST_SUBSTRATE_CREATION_CRASH=1`, together with the capacity-holder option:
  after capacity exhaustion, claim a real 20-second fixture lease and provision
  its normal Core bootstrap credential. Resume the journaled actor, kill its
  assigned worker before Core completion, and wait for natural lease expiry.
  The next Runner attempt must finish creation with unchanged Core identity,
  credential, actor UID and CSI volume mapping. This reproduced a pre-fix
  `ResumeActor` precondition failure. With the fix, the full two-owner proof
  passed on 2026-09-23 in 163.92 seconds, including native chat/history after
  restart. The test lease is deliberately shorter than the production default
  and does not establish production retry latency.
- `FC_TEST_SUBSTRATE_IMAGES=1`: require a real four-quadrant image response
  through an uploaded file reference and an observed native vision-tool call.
  Require the image bytes and persisted reference to survive restart.
- `FC_TEST_SUBSTRATE_BROWSER=1` and `FC_TEST_SUBSTRATE_BROWSER_PORT`: rendered
  dashboard with fixture account/project rows, real Core grants and real native
  HTTP/WebSocket traffic. Allow that exact loopback origin at ingress. Verify a
  model reply and reopening history after reload; reject legacy chat calls.
  This is not yet a full deployed account-session/browser qualification.
- `FC_TEST_SUBSTRATE_ARTIFACT_DIR`: retain the second agent's authenticated
  screenshot download, mode 0600. The proof observes headed Chromium on Xvfb,
  a unique HTML marker and `scrot`, validates 1280×800 PNG bytes and their hash
  across lifecycle changes, and rejects anonymous downloads. It terminates its
  browser process group. A visually inspected capture showed the real window.

Actors/volumes intentionally remain for inspection; suspend completed fixtures
to release compute. **DeleteActor destroys their data.** Never inspect original
SQLite files directly; use the snapshot helper or a scratch copy including WAL.

Focused provider tests also cover template CAS and capacity retry. For
`real_substrate_resume_waits_for_capacity`, occupy all slots, name a suspended
ready fixture with `FC_TEST_SUBSTRATE_ACTOR`, and a disposable running holder with
`FC_TEST_SUBSTRATE_CAPACITY_RELEASE_ACTOR`. It resumes the former and suspends the
latter, preserving both volumes. The template CAS test restores its original
reference and checks stale-version rejection and unchanged actor/CSI identities.

Before and after every rollout:

```sh
KUBECONFIG=/path/to/kubeconfig scripts/finite-status --substrate-context CONTEXT --json
```

This checks observed generation and full replica convergence for API, controller,
router and egress in `ate-system`; it does not claim chat, capacity or recovery.
Worker inventory must include all assignments (including `ate-golden`): an
atespace-filtered actor inventory is not the fleet's available capacity.

The local API credential rotation check passes in
`token-rotation-qualified.log`: one Rust client reads a retained actor, rejects
an atomically replaced invalid token, then accepts an independently issued token
and reads the same actor UID. This proves credentials are reloaded between calls;
it does not qualify Kubernetes projected-volume rotation on GKE. Runner library
tests pass (201 passed, four opt-in integration tests ignored); the real rotation
test passes separately. The full workspace Rust tests pass in
`final-rust-low-storage-2.log`. Its subsequent structure gate identified an
oversized proof file; moving dashboard onboarding into its own module preserves
behavior. Formatting, structure checks and strict workspace/all-target Clippy
then pass in `final-structure-clippy.log`.


## Evidence and remaining gates

The evidence below covers different boundaries and revisions; it is not one
all-features production certification. Full Rust workspace tests pass in
`final-rust-low-storage-2.log`; formatting, structure and strict workspace/all-target
Clippy pass in `final-structure-clippy.log`. Subsequent recovery fixtures have their
own live proofs and strict Core Clippy checks. Historical failed runs and superseded
image digests remain in git history instead of defining the current gate status.

Current local evidence (2026-09-22):

| Scope | Result and limit |
| --- | --- |
| Dev stack | `DEVFINITY_PORT_OFFSET=4000 just dev smoke` and source-structure check passed. |
| Dashboard | `final-dashboard-checks-2.log`: lint, 336 tests and production build pass. Native browser and deployed-auth boundaries remain separate. |
| Native protocol + SimpleX restart | Two-owner real-relay run passed, including worker loss, explicit stop/restart, native approval/clarification/reconnect/interruption, history/files, desktop screenshot and repeated isolation checks. This did not prove SimpleX across image upgrades. |
| Combined browser + SimpleX + image recovery | `native-home-simplex-upgrade-proof.log`: both owners passed in 627.42s. Native and SimpleX replies/addresses, actor UID/CSI records, Core artifact/contact/host, history/files and screenshot survived worker loss, restart, healthy upgrades and failed-image recovery. Live browser creation/reload also passed. |
| Image upload | `native-browser-image-proof.log` passed in 152.98s: real UI upload, native vision reply, image rendering and reload. `native-image-reference-probe.log` observes the native vision call; partial mixed-upload failure, retry, TypeScript and lint also pass. `image-reference-upgrade-qualified.log` passed in 163.37s: exact image bytes and the persisted user reference survived restart and a healthy runtime upgrade. |
| Empty-cluster recovery | `finite-substrate-restore/drill-5/proof.log`: two owners passed in 591.69s after physical provider Postgres, CSI and snapshot-store restore onto a recreated cluster. Native history/files/images, SimpleX identity/replies, central environment refresh and bidirectional isolation passed. Core stayed alive outside the failure domain; GKE and full Core recovery remain unqualified. |
| Automatic worker recovery | `automatic-recovery-qualified.log` passed in 178.01s with two owners, native chat, image persistence and SimpleX. Recovery during admission drain preserves actor/CSI identity; wrong-host requests, duplicate admission and pending/completed stops are fenced. A delayed crash observation against a running actor leaves its version unchanged. This does not establish node-loss or in-flight-turn recovery. |
| In-flight worker loss | `inflight-worker-recovery-qualified.log` passed in 116.46s on the canonical image. The worker dies after a foreground tool's observed file write, before tool completion. Normal recovery under admission drain preserves actor/CSI identity, retains the accepted prompt exactly once, clears the busy session, and accepts a subsequent model turn without repeating the side effect. No owner stop/restart intervenes for this actor. This covers that tool-execution boundary, not arbitrary external effects, model-request persistence windows, node loss or exactly-once execution in general. |
| Lost completion | The actual Runner was killed after provider success, before Core completion; natural lease expiry and a new Runner recovered the same upgrade request. Both owners retained native chat/history/files and actor/storage identity. This does not prove automatic worker crash recovery. |
| Clarification controls | Browser contract tests pass for single/multi-select, custom answers, reload, and ordered stale/current expiry events. `clarification-native-batch-qualified.log` passed in 115.86s: a real batch retains its accepted multi-select answer across reconnect, delivers two intact selections to the tool, completes the free-text answer, and rejects a late duplicate. Native timeout timing was not exercised. |
| Mid-turn corrections and queue | The consumed-redirect/Stop race is reproduced offline and fixed through the existing writer; `consumed-redirect-dashboard-qualified.log` passes the full two-owner sequence and restart in 171.37s. Real provider baselines reproduce lost ordinary steering and stopped model corrections; offline SQLite regressions fail before/pass after. The sealed package and canonical image pass all correction and stream-writer regressions. `dashboard-onboarding-canonical-qualified.log` passes ordinary, queued, stopped-tool and stopped-model input through actor restart in 123.23s. Earlier exact-reply failures preserved input but replied `done`; a diagnostic trace carries the correction as the final outbound user row. The test now explicitly revokes the earlier `done` instruction and retains exact-reply and persistence assertions. |
| Native Brain approval cards | Native tool-history projection consumes the existing `fbrain` reference trailer and reuses the shared server-backed approval cards. Two projection tests, TypeScript, targeted lint and browser proofs (`native-brain-cards-action.log`, `native-live-tools.log`) pass: cards appear from live native tool events and survive reload, repeated tool completions update the same row, approval posts the exact reference to the existing route, sends a native human-readable receipt, and the resolved server request stays closed after reload. Brain endpoints are fixtures in those card-rendering tests; fresh-user signing/service parity is separately qualified above. Approval choice metadata is not added to Hermes storage; request authority and resolution remain on the Brain server. |
| Actual dashboard routes | `dashboard-onboarding-canonical-qualified.log` passes in 123.23s on canonical runtime code plus only the fixture CA (`sha256:6f94a71db3000af2e1847b20e067d95e65b966a07635e2fbf2d7c12969d18929`). Both fresh owners complete the actual launch-code wizard and launch agents; real Next chat, image upload, authenticated download and reload pass without grant/file interception, followed by native protocol/restart checks for both owners. Account auth uses existing dev mode, not WorkOS login. Brain/Sites service behavior is not qualified by this chat proof. |
| Browser boundary | Real browser exposed native preflight 401. Shared ingress CORS tests, real HTTPS preflight and strict Runner clippy pass; the Home/Recents projection regression also passes. `native-home-browser-proof.log` confirms a live UI-created model reply and history reopening after reload. The combined run above passed; the deployed account/origin path remains unqualified. |

The protocol browser fixture covers pagination beyond 100 rows, failure of a later
page without partial publication, archive/reload, failed turns, unsent drafts,
interruption, reconnect with/without the persisted user row, queued approvals and
batch clarification. The real native proof observes an active tool before socket
reconnect, verifies native interruption and a later turn, reconnects to an exact
clarification request and rejects a late duplicate, and verifies separate deny
and allow-once file operations with duplicate approvals resolving zero requests.
The separate in-flight worker-loss proof above covers a foreground tool boundary.

Do not enable production admission until the remaining gates are satisfied:

- Qualify real WorkOS login and the deployed dashboard origin. Local actual-Next
  onboarding, native chat and correction/queue history pass on the canonical image
  with synthetic account authentication; this does not establish deployed auth
  or deployed Brain/Sites integration. Native Sites requester attribution passes
  locally (above), as does fresh-user human signing/approval through dashboard
  routes and native terminal Organization Brain creation/access with the current
  CLI. Historical card rendering has separate fixture coverage. The removed CLI
  request producer is not a new runner feature.
- Qualify invalid immutable images, lost boot credentials and actionable startup
  diagnostics. Correctable invalid central flags, transient bootstrap failure/
  RESUMING re-entry, capacity exhaustion/retry, worker-loss recovery and an
  interrupted foreground tool are qualified;
  partition fencing, disk loss and failures during model-request persistence
  remain separate gates. A retained-disk local node restart is qualified above.
  Keep upgrades opt-in; retirement and backup recovery remain unadvertised.
- Preserve the Recovery Authority and qualify the deployment Recovery Set. The
  local empty-cluster drill passes with Core retained; full Core/identity outage,
  populated provider authorization and GKE storage recovery remain unqualified.
  A provider volume/snapshot alone is not a backup.
- Qualify intended upstream release, production egress/DNS/internet tools, storage,
  isolation, TLS, projected identity rotation, capacity and fleet probes on GKE.
  Google's [installation documentation](https://docs.cloud.google.com/kubernetes-engine/ai-ml/install-overview-substrate)
  currently requires Standard, not Autopilot (1.37+, or 1.36 with certificate beta
  APIs enabled at creation). Ordinary GKE must not be presented as confidential
  Phala compute; resolve Confidential-tier placement explicitly.
- Verify existing-state and mixed-version product behavior, finish applicable CI
  gates, and retain a rollback that can read all admitted provider rows.
