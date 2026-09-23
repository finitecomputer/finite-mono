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

## Runtime configuration and processes

Public flags come from Core's existing `FC_CORE_RUNTIME_ENV_JSON`, through
`GET /api/core/v1/runtime/environment` on its dedicated runtime listener. The
no-store response excludes secrets and derives the creation's owner allowlist.
There is no new configuration table. `finite-agentd bootstrap` fetches flags
before invoking the canonical runtime entrypoint. Updates apply on the next
cold boot; a failed fetch never falls back to cached flags. Existing runners
continue consuming their persisted RuntimeSpec environment.

The real two-owner proof changes Core's boot configuration between launch and
restart, leaving installed templates and creation specs untouched. Each agent
uses its terminal tool to write one actual environment value to a file; the
verifier downloads and compares the bytes before and after restart.
`environment-refresh-qualified.log` passes both values, native chat and retained
history for both owners in 169.62 seconds on the qualified runtime image recorded below.
This proves next-boot refresh of an updated value, not hot reload or removal of
an operator flag. Rust clippy/format, JavaScript syntax and structure checks pass.

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

`scripts/proofs/hermes-steer-history.py` uses real scratch SQLite for ordinary
steering, stopped tool steering, and a stopped model redirect with no tool history.
It checks exactly-once storage, unchanged earlier rows, drained buffers and
unchanged Stop state. The native proof requires ordinary and queued replies, then
checks ordinary, queued, stopped-tool and stopped-model input after settlement
and actor restart. The earlier combined candidate passed both owners. Its sealed Linux package
builds and passes all three SQLite cases, the stream-writer regression and Nix
integrity verification (`model-stop-canonical-proof.log`,
`model-stop-canonical-stream.log`, `model-stop-canonical-integrity.log`).
The canonical image plus only the local fixture CA passes real dashboard onboarding,
native chat and restart in `dashboard-onboarding-canonical-qualified.log` (123.23s).
Its test digest is `sha256:6f94a71db3000af2e1847b20e067d95e65b966a07635e2fbf2d7c12969d18929`.

A later generated-file browser proof passed authenticated file bytes and reload
twice, but exposed the consumed-redirect/Stop race during the subsequent native
clarification turn (`generated-file-dashboard-qualified.log` and
`generated-file-dashboard-traced.log`). Native request dumps showed the canceled
correction merged with the next user request. The fourth offline SQLite case
reproduces that failure on the previous image and passes on the rebuilt sealed
package (`consumed-redirect-before.log`, `consumed-redirect-after.log`). The
canonical image also passes all four SQLite cases; the sealed package passes
the stream-writer regression and integrity verification. The combined actual
dashboard/provider proof passes in 171.37 seconds
(`consumed-redirect-dashboard-qualified.log`): both owners launch, native browser
chat/image upload/generated-file bytes and reload pass, the Stop/clarification
sequence completes, and both owners retain native chat/history after restart.
The qualified test image is
`sha256:2e6886a545af36aefdf1b41396776aa7a6487e8d5c96bb747aac0308324d7ad5`
(canonical runtime plus only the local fixture CA). Request dumping is disabled.
Both disposable actors are suspended with data retained; fleet status is healthy
before and after. This remains local synthetic account auth, not WorkOS qualification.

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
The real two-owner proof also exhausts the full provider retry window with every
worker occupied, verifies the expired Core lease, then frees one worker. On
2026-09-23 the same creation launched in 3.93 seconds with unchanged runtime ID,
Core credential, provider correlation, runtime spec and actor UID. Native chat
and history after restart passed for both owners (full proof: 139.87 seconds).
This is one local observation, not a GKE latency guarantee.

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
made to the shared volume. Interrupted Runner completion still needs proof.
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
state), the exact image and installed template, and the Core ownership/credential
and provider-operation bindings. Provider metadata must retain the actor UID and
volume mapping or be replaced through an explicitly authorized, identity-checked
rebinding contract. The independent Recovery Authority must be able to obtain the
backup and necessary decryption/bootstrap secrets after losing the source cluster.
Templates contain credentials and belong in protected recovery storage, never logs.

Substrate already persists actor UIDs, full actor protobufs (including volume
bindings), templates and authorization state in its Postgres database. Prefer a
coordinated provider-database/storage restore over introducing a Core copy of
these bindings or a new per-agent rebinding API. On 2026-09-23, a private custom
`pg_dump` of the local provider restored transactionally onto empty Postgres 18
storage in a network-isolated container. Both named suspended proof actors
retained byte-identical serialized state, UID and version. The database restored
236 actors and 241 templates; there were zero authorization tuples in this local
fixture, so it does not qualify recovery of populated authorization policy.
Truncating the archive caused restore failure and left zero public tables.
Evidence is in `provider-restore/` under the private local proof directory. This
is only a metadata restore proof: no restored controller or runtime was started.
Database role grants were excluded on the isolated target and remain a separate
deployment-identity requirement.

The existing two-owner test drops its isolated Core database on completion. A
complete recovery drill must capture the coordinated Recovery Set inside that
test lifetime, after fencing all selected writers; retained actor volumes from
a completed run cannot establish recovery of the deleted Core authority.

The next restore proof must fence the source writer, restore onto genuinely empty
storage without reading the source, start exactly one writer, and exercise native
history/files, SimpleX identity and owner isolation with the retained Core binding.
Copying `/data` while Core/provider bindings remain unrecoverable is only a data-copy
proof. GKE storage backup and provider rebinding remain unimplemented qualification
gates; do not add direct actor-status edits as an operator repair procedure.

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
  `kind-finite-hermes-spike` / `finite-hermes-spike`. Require CRASHED, then Core
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
- `FC_TEST_SUBSTRATE_INFLIGHT_CRASH=1` with automatic recovery: after the first
  owner's normal protocol checks, start a foreground terminal command that writes
  one marker and waits. Verify its side effect before killing the worker. Require
  automatic recovery alone (no owner restart), exactly one retained user prompt,
  an idle resumed session, a successful subsequent turn, and unchanged marker
  bytes before and after that turn. The second owner still exercises stop/restart.
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

## Evidence and remaining gates

Current local evidence (2026-09-22):

| Scope | Result and limit |
| --- | --- |
| Complete workspace | 2,053 passed, 15 ignored in `workspace-bounded-concurrency.log` with `TMPDIR=/tmp` and `--test-threads=4`. Includes the latest browser/migration work; predates the control-priority fix, separately covered by 28 Runner-cycle tests and strict all-target Runner clippy. |
| Current candidate regression run | `workspace-final-candidate.log` stopped at six parallel Kata lifecycle failures (four fake-command 2s timeouts, one cleanup assertion and one synchronization deadline). All six pass unchanged in `runner-serial-qualification.log`: 200 passed, 3 ignored in 85.93s. This does not establish a green default-parallel workspace run. The full-schema placement replay extension separately passes for Apple Container and Substrate. |
| Dev stack | `DEVFINITY_PORT_OFFSET=4000 just dev smoke` and source-structure check passed. |
| Dashboard | Current `dashboard-current-full.log`: 327 tests and production build pass after native Brain-card/tool-event changes. Typecheck, targeted lint and the native protocol browser fixture also pass. Build reports 15 Turbopack file-tracing warnings through `workspace-paths.ts`; no warning suppression was added. |
| Native protocol + SimpleX restart | Two-owner real-relay run passed, including worker loss, explicit stop/restart, native approval/clarification/reconnect/interruption, history/files, desktop screenshot and repeated isolation checks. This did not prove SimpleX across image upgrades. |
| Combined browser + SimpleX + image recovery | `native-home-simplex-upgrade-proof.log`: both owners passed in 627.42s. Native and SimpleX replies/addresses, actor UID/CSI records, Core artifact/contact/host, history/files and screenshot survived worker loss, restart, healthy upgrades and failed-image recovery. Live browser creation/reload also passed. |
| Image upload | `native-browser-image-proof.log` passed in 152.98s: real UI upload, native vision reply, image rendering and reload. `native-image-reference-probe.log` observes the native vision call; partial mixed-upload failure, retry, TypeScript and lint also pass. `image-reference-upgrade-qualified.log` passed in 163.37s: exact image bytes and the persisted user reference survived restart and a healthy runtime upgrade. |
| Automatic worker recovery | `automatic-recovery-qualified.log` passed in 178.01s with two owners, native chat, image persistence and SimpleX. Recovery during admission drain preserves actor/CSI identity; wrong-host requests, duplicate admission and pending/completed stops are fenced. A delayed crash observation against a running actor leaves its version unchanged. This does not establish node-loss or in-flight-turn recovery. |
| In-flight worker loss | `inflight-worker-recovery-qualified.log` passed in 116.46s on the canonical image. The worker dies after a foreground tool's observed file write, before tool completion. Normal recovery under admission drain preserves actor/CSI identity, retains the accepted prompt exactly once, clears the busy session, and accepts a subsequent model turn without repeating the side effect. No owner stop/restart intervenes for this actor. This covers that tool-execution boundary, not arbitrary external effects, model-request persistence windows, node loss or exactly-once execution in general. |
| Lost completion | The actual Runner was killed after provider success, before Core completion; natural lease expiry and a new Runner recovered the same upgrade request. Both owners retained native chat/history/files and actor/storage identity. This does not prove automatic worker crash recovery. |
| Clarification controls | Browser contract tests pass for single/multi-select, custom answers, reload, and ordered stale/current expiry events. `clarification-native-batch-qualified.log` passed in 115.86s: a real batch retains its accepted multi-select answer across reconnect, delivers two intact selections to the tool, completes the free-text answer, and rejects a late duplicate. Native timeout timing was not exercised. |
| Mid-turn corrections and queue | The consumed-redirect/Stop race is reproduced offline and fixed through the existing writer; `consumed-redirect-dashboard-qualified.log` passes the full two-owner sequence and restart in 171.37s. Real provider baselines reproduce lost ordinary steering and stopped model corrections; offline SQLite regressions fail before/pass after. The sealed package and canonical image pass all correction and stream-writer regressions. `dashboard-onboarding-canonical-qualified.log` passes ordinary, queued, stopped-tool and stopped-model input through actor restart in 123.23s. Earlier exact-reply failures preserved input but replied `done`; a diagnostic trace carries the correction as the final outbound user row. The test now explicitly revokes the earlier `done` instruction and retains exact-reply and persistence assertions. |
| Native Brain approval cards | Native tool-history projection consumes the existing `fbrain` reference trailer and reuses the shared server-backed approval cards. Two projection tests, TypeScript, targeted lint and browser proofs (`native-brain-cards-action.log`, `native-live-tools.log`) pass: cards appear from live native tool events and survive reload, repeated tool completions update the same row, approval posts the exact reference to the existing route, sends a native human-readable receipt, and the resolved server request stays closed after reload. Brain endpoints are fixtures here; live signing/service parity remains unqualified. Approval choice metadata is not added to Hermes storage; request authority and resolution remain on the Brain server. |
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
  or Brain/Sites service parity.
- Qualify stuck-start/capacity-exhaustion handling without overriding Core
  stop/control intent. Local worker-loss recovery and an interrupted foreground tool are qualified;
  node loss and failures during model-request persistence remain separate gates.
  Keep upgrades opt-in; retirement and backup recovery remain unadvertised.
- Preserve the Recovery Authority and restore the same Recovery Set onto an
  empty target. A provider volume/snapshot is not a backup. Local worker-pod loss
  does not establish node-loss or GKE CSI recovery.
- Qualify intended upstream release, production egress/DNS/internet tools, storage,
  isolation, TLS, projected identity rotation, capacity and fleet probes on GKE.
  Google's [installation documentation](https://docs.cloud.google.com/kubernetes-engine/ai-ml/install-overview-substrate)
  currently requires Standard, not Autopilot (1.37+, or 1.36 with certificate beta
  APIs enabled at creation). Ordinary GKE must not be presented as confidential
  Phala compute; resolve Confidential-tier placement explicitly.
- Verify existing-state and mixed-version product behavior, finish applicable CI
  gates, and retain a rollback that can read all admitted provider rows.
