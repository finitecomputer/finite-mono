# Hermes Runtime Test Matrix

Status: active v2 quality gate.

This document owns the product-shaped Hermes Agent Runtime and multi-device
ladder for finitecomputer-v2. Finite Chat owns the encrypted protocol, Devices,
Rust daemons, and Hermes sidecar. v2 owns proving the Hosted Web Device,
optional local Devices, real runtime, and product services together before
deploying them for users.

## Principle

Every rung must exercise the same product shape:

- deployed/default chat server is `https://chat.finite.computer` unless the run
  is explicitly validating an unmerged server branch;
- the Hosted Web Device is the primary launch Device, with a headless or
  Electron Device added to the same account and Room for multi-device proof;
- no PIN flow;
- the same Finite Chat Hermes plugin and CLI are packaged into the runtime;
- the Product Release pins Hermes 0.18.2, the CLI/service versions, the one
  canonical Runtime image digest, and the Finite Skills baseline bundled for
  fresh agents; no test lane supplies an independent default;
- `.github/workflows/runtime-image.yml` and
  `scripts/build_runtime_image.py` are the single image build path; every
  provider rung consumes the resulting digest rather than rebuilding a variant;
- normal Hermes delivery uses the resident Rust `finitechat hermes serve`
  sidecar and held inbound stream; Python must not schedule timer polling or
  spawn a CLI process per message;
- ordered pull/sync remains the recovery and catch-up consistency path after a
  stream, process, or server restart;
- a fresh Agent Home receives the image's bundled Finite Skills baseline once;
  restart and image replacement never overwrite it, and existing agents update
  only through an explicit `finite skills sync` invocation;
- the same runtime image is used after the local source-level rung;
- destination differences are limited to provider config, durable volume
  binding, and public ingress;
- Runtime Management Pipe traffic, when present, is outbound generic health and
  Product Release telemetry only; acknowledgements never become lifecycle,
  feature, credential, chat, or skills commands;
- Hermes must be real, not an echo handler;
- the Finite Private model at every rung is `glm-5-2`, served behind the
  historical `https://kimi-k2-6.finite.containers.tinfoil.dev/v1` limiter URL
  (docs/service-dependencies.md, Finite Private Routing Debt);
- acceptance is a human-usable conversation in the canonical BoxOne-derived
  dashboard UI. Automated rungs keep their normal test artifacts, while the
  blessed internal production canary does not require a bespoke evidence
  generator or report; local-device coverage proves the same Room/AppState
  contract and does not define a competing UX.

If a lower rung fails, do not climb. Fix the lowest failing contract first.

## Canonical Hermes Sidecar Path

The launch implementation is the current mono-imported lineage:

- resident Rust `finitechat hermes serve` and loopback action API;
- one held `/v1/hermes/inbound` NDJSON stream into the thin Python adapter;
- idle heartbeat so that stream does not terminate while quiet;
- local bridge readiness folded into generic runtime health; RMP does not gain a
  Finite Chat-specific status schema;
- Electron and Hosted Web Device UIs consuming Rust AppState/AppAction updates,
  not implementing chat or Hermes sync loops.

Production must enable this path unconditionally. Stream loss reconnects with
bounded backoff and resumes by durable cursor; it does not enter Python
`_poll_loop`, call `_poll_once`, invoke a CLI subprocess, or reopen the
encrypted store per message. CLI `hermes poll` may remain a diagnostic tool,
and ordered pull after a wake hint remains the Finite Chat consistency model.

Production mode now selects a strict resident stream loop whenever
`FINITECHAT_HERMES_INBOUND_STREAM=1`: failures reconnect with bounded backoff
and never select Python polling or CLI fallback. Focused adapter regressions
cover that choice. Rung 1 must still prove the real `hermes serve` process and
resident NDJSON path together; a one-shot CLI diagnostic is not sufficient.

## Mandatory Outage And Restart Matrix

Every product release runs these scenarios against one canonical Room with the
Hosted Web Device, Agent Runtime/Hermes sidecar, and a second headless or
Electron Device. Tests kill real processes or remove reachability while keeping
the same account, device identities, server URL, Room, and durable stores.
That restart matrix is necessary but not sufficient: the destructive recovery
cases below deliberately remove stores, volumes, keys, and service databases.

For the first working SaaS slice, process/service outage and same-volume restart
rows are mandatory. Destructive store/volume/key-loss, Recovery Snapshot,
Runtime Retirement, and Purge rows are retained below as explicit post-MVP
recovery TODOs; they do not block the trusted-cohort launch.

The named internal production canary manually exercises only the normal
dashboard-initiated Agent Runtime restart and proves a real turn before and
after it in the same visible conversation. The shared Finite Chat server,
Hosted Web Device, and other process restart rows stay in automated integration
coverage; they are not manual canary steps.

| Failure | Must remain usable | Required healing proof |
| --- | --- | --- |
| Chat server unavailable, then restarted | Each Device can preserve locally accepted outbound state; no Device becomes room authority | Server return causes ordered catch-up; every message has one stable id and appears at most once; Hermes produces no duplicate turn |
| Hosted Web Device daemon unavailable, then restarted | Agent and Electron/local Device continue through the canonical Room | Hosted store reopens, catches up without re-enrollment, and resumes the same topics and delivery state |
| Electron/local Device unavailable, then restarted | Hosted Web Device and agent continue without it | Local store reopens and catches up without copying state from the Hosted Web Device |
| Hermes sidecar unavailable, then restarted | User Devices keep syncing and can see an explicit agent-degraded state | Resident stream resumes from its durable cursor; acknowledged input is not redispatched and unacknowledged input is recovered |
| Agent Runtime restarted on the same durable volume | Chat server, product services, and user Devices remain independent | Agent identity, Room membership, Hermes memory, workspace, and runtime-owned durable state survive; service-owned feature state is unaffected |
| Any stream is interrupted while idle or active | Other participants and the ordered log remain healthy | Reconnect uses bounded backoff and durable sync; no tight reconnect loop, stuck cursor, duplicate bubble, or duplicate model turn |
| Core or RMP is unavailable | Chat, Hermes, product services, and user skills remain usable | Runtime work continues; generic health/release telemetry reconnects with bounded backoff after Core returns and never polls for desired state |
| A fresh agent starts while a skills source is unavailable | Hermes, user skills, and the bundled baseline remain usable | Runtime seeds from the image without a network fetch; no Core, Runner, RMP, Git, or HTTP polling path is involved |
| Explicit `finite skills sync` is interrupted | One complete managed baseline and all user skills remain usable | Before the atomic exchange the prior baseline remains active; after it the new baseline is active and prior staging may remain, but Hermes never sees a mixed tree or an automatic retry; Core, Runner, and RMP remain uninvolved |
| Hosted Web Device store is lost or corrupted | Electron/local Device and agent continue; dashboard exposes recovery rather than silently minting a new account | Restore a Recovery Snapshot or enroll a replacement Device through recovered account authority; retained history remains accessible through a surviving/recovered Device |
| Electron/local Device store is lost | Hosted Web Device and agent continue | Replacement Device links without changing the human account; product copy does not promise pre-membership MLS history until encrypted history backup/share is implemented |
| All user Device stores are lost | Server ciphertext and agent continue without being mistaken for recoverable plaintext | User Recovery Key or Finite-Assisted Recovery restores account/device authority and a usable history/export path; otherwise the release fails its Recoverability Contract |
| Agent Provider Durable Volume is deleted | User Devices, service backups, and Core remain available | An off-host Recovery Snapshot restores the same Agent Principal Key, MLS client store, Hermes memory, `/data/workspace`, connection state, and Brain Folder Keys onto empty replacement compute |
| Chat room-server database or encrypted blob store is lost | Surviving Devices retain local state and no replacement server accepts divergent history | Service-consistent off-host snapshots restore the ordered log, room metadata, invites needed for continuity, and retained encrypted blobs before clients resume |
| Finite Brain, Finite Sites, or Core primary storage is lost | Other product planes stay isolated and report a precise degraded state | Each service restores from a service-consistent off-host snapshot; Brain proof includes usable Folder Keys, Sites includes Git/blob/app state plus owner access, and Core retains Account/Project identity |
| Runtime Retirement is requested | User access is paused without erasing recovery material | Recovery Readiness passes before compute removal; one verified support-held Recovery Snapshot and the original local durable state remain available. User export is a later Purge/Export contract, not a v1 retirement prerequisite. |
| Purge User Data is requested | Nothing is deleted by stop, billing state, or retirement | Separate authorization, retention expiry, export offer, and explicit confirmation are proven before volume and snapshot deletion |

Automated scenarios may emit machine-readable artifacts when the existing
harness naturally provides them. Missing report machinery is not itself an
implementation gate, and deferred destructive-recovery fields do not apply to
the internal production canary. A health check alone is still not proof of a
human-usable chat. The production canary uses the normal product surface; if a
worksheet, hidden diagnostic, shell, or manual state reconstruction is needed
to decide whether it worked, the canary failed.

## Rung 1: Local Real-Hermes Sidecar And Devices

Purpose: fast source-level proof that Finite Chat Devices and the resident
Hermes sidecar complete real turns and heal before any runtime image is built.

Shape:

- `finitechat-server` may run locally for automated branch validation;
- a headless Device drives deterministic protocol state; Hosted Web Device and
  Electron adapters consume the same Rust AppState/actions contract;
- Hermes 0.18.2 runs as a local process with the finitechat-owned plugin and
  strict resident stream mode enabled;
- model provider keys come from local operator env only.

This rung belongs mostly to `finitechat`. It proves the plugin/app/protocol
contract, not the hosted runtime image.

Acceptance:

- two independently enrolled Devices join the same account/Room;
- user message reaches Hermes;
- Hermes produces at least two real model-backed replies;
- the Finite platform router and representative managed skill are visible;
- strict stream failure reconnects through the resident Rust service without
  Python polling or a CLI-per-message subprocess;
- app shows pending/thinking/delivery states correctly;
- sidecar and each Device restart independently without duplicate acknowledged
  messages or re-enrollment;
- the Mandatory Outage And Restart Matrix passes at source level.

## Rung 2: Canonical Runtime In Apple Container

Purpose: prove the real local SaaS and canonical Runtime image through the same
thin provider contract used by hosted Runners.

Operator command on Apple silicon and macOS 26 or newer:

```bash
container system start
export FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY=<one operator-held deployed key>
just dev saas-smoke
just dev up
```

On a fresh checkout, `just dev saas-smoke` is both the Launch Code bootstrap and
credential-gated acceptance; skip it on later interactive runs with a persisted
agent. Devfinity builds the one Hermes 0.18.2 image, registers it as a promoted
local artifact, runs Core and the generic Runner, launches an Apple VM with a
durable `/data` bind mount, and opens the same Hosted Web Device used by the
dashboard. The preferred chained-limiter path gives each runtime a Core-issued
key while keeping the operator key in the local limiter process; the explicit
direct Runner override is a fallback only.

Shape:

- Apple Container implements sandbox lifecycle, loopback publication, durable
  mount attachment, network routing, and generic secret injection only;
- Finite Chat owns Agent Principal discovery through `/contact` and
  KeyPackage/Add/Welcome admission;
- Core and Runner do not gain chat, Brain, Sites, skills, or Electron feature
  commands;
- Electron can later replace the Hosted Web Device with a local Device while
  using the same auth, AppState/actions contract, and Room.

Acceptance:

- dashboard create-agent produces a real Project, lease, Apple VM, and Agent
  Principal rather than a fixture;
- a fresh Agent Home receives the bundled Finite Skills baseline once, and
  Hermes discovers it without a network fetch or automatic fleet update;
- Hosted Web Device joins through Welcome-first admission and receives multiple
  real Hermes replies;
- generic restart replaces stale compute with the current image while
  preserving the exact agent npub, Room, workspace, Hermes memory, and `/data`;
- chat-server and Hosted Web Device restarts self-heal without restarting the
  Apple VM, polling, duplicate acknowledged messages, or MLS re-enrollment;
- the image reports generic `/healthz` readiness and `/contact` identity; the
  Runner does not inspect product-specific chat state;
- `finite skills sync` remains an explicit agent/user action and is tested
  separately from Runtime release or restart;
- Recovery Snapshot format, tooling, and off-host restore remain an explicit
  post-first-slice TODO/open question; normal stop/restart must never delete or
  make the durable state inaccessible.

## Rung 3: Runtime Image On Kata

Purpose: prove the first production Runner using the same OCI image and
provider-neutral Core, runtime bootstrap, and Runtime Management Pipe contracts.

Shape:

- use the same promoted runtime image from Rung 2 on finite-lat-1;
- Kata owns only sandbox lifecycle, durable volume attachment, network, and
  generic secret injection; product features remain in their owning services,
  UI, stable CLIs, or skills rather than gaining Runner branches or Runtime
  Management commands;
- keep the same durable mount layout and env names;
- use the same invite/status API as local Docker;
- every Device still talks to `https://chat.finite.computer`.

Acceptance:

- fresh Kata deployment works first from the documented runner command;
- invite/status API is reachable to the operator/dashboard path;
- Hosted Web Device and a second Device chat with real Hermes for multiple turns;
- remote restart preserves identity, chat state, and Hermes memory;
- provider-independent off-host snapshot and empty-target restore preserve the
  full Recovery Set; the Kata host volume is not counted as its own backup;
- `fsite` publish smoke passes;
- the unchanged fresh-agent skills seed, restart non-overwrite, and
  user-override suite from Docker passes without Runner assistance;
- `finite skills sync` runs only after explicit local invocation and passes
  unchanged without Runner assistance;
- logs and health endpoints make failures classifiable without shelling into
  random process internals.

## Rung 4: Phala CVM Fast Follow

Purpose: prove the production confidential-runtime path with durable mounts.

Shape:

- deploy the same runtime image with a Phala compose/CVM config;
- mount the provider durable volume at the same path used in Docker;
- provide the same env contract as Docker;
- keep public ingress and invite/status behavior equivalent to Docker;
- use Phala attestation/debug surfaces for canary evidence.

Acceptance:

- one-off CVM deploy succeeds from the v2 runbook;
- invite/status API returns the expected agent identity/profile metadata;
- Hosted Web Device and a second Device complete multiple real Hermes turns;
- agent can save a memory, restart, and demonstrate that memory survived;
- agent can publish a small site with `fsite`;
- Finite Private request succeeds (runtime-scoped key provisioned by Core —
  no operator key override — against the deployed limiter serving `glm-5-2`);
- Phala restart does not require re-pairing the user;
- the unchanged fresh-agent skills seed, restart non-overwrite, and
  user-override suite passes; attestation identifies the canonical image and
  therefore its bundled baseline, not a Core-managed active revision;
- Phala restores the same provider-independent Recovery Snapshot onto an empty
  replacement CVM and documents which Recovery Authority released the keys;
- deployment evidence includes image ref, finitechat commit, Hermes version,
  plugin commit, Phala app id, and health output.

## Rung 4b: Enclavia Enclave

Purpose: evaluate Enclavia as a second confidential runner target before
promoting it into the self-serve SaaS lane.

Shape:

- use a pre-created Enclavia enclave with container port `8080` and persistent
  encrypted `/data` storage;
- push the same promoted runtime image with an evaluation worker advertising
  `FC_RUNNER_CLASS=enclavia`;
- inject the same Docker-equivalent runtime env through Enclavia secrets;
- reach health and invite through
  `https://<enclave-id>.enclaves.beta.enclavia.io/proxy/...`;
- record Enclavia status/PCR/build evidence alongside the normal runtime
  health evidence.

Acceptance is the same as the Phala rung, plus an explicit cost note for the
configured storage size before any production decision.

## Rung 5: Dashboard-Controlled SaaS Product

Purpose: prove the user-facing self-serve flow.

This rung is the later customer-facing target, not the narrower internal
production canary. The canary proves WorkOS auth, single-use Launch Code
redemption, Kata launch, real Hosted Web chat, and same-volume Agent Runtime
restart; it does not stand in for Stripe, customer admission, Brain, Sites
list/share, or the backup/empty-target-restore gate below.

Shape:

- user signs in through Account Auth, names the agent, and chooses an icon;
- Core assigns the standard Runner class from product policy without exposing
  provider selection in onboarding;
- Core creates Project, Finite Private grant, runtime record, and launch request;
- Core routes the request to a compatible Runner without a global-queue race;
- the Runner deploys the same promoted OCI runtime image used by every lower rung;
- dashboard opens the Hosted Web Device and canonical agent Room;
- dashboard connection flows use their owning product services, stable APIs, or
  agent-local skills and never add Runtime Management feature commands/status;
- dashboard lists and previews Finite Sites and exposes Finite Brain through
  the same Account Auth identity binding;
- dashboard may render the release's informational Finite Skills catalog and
  explicit sync guidance, but never claims a Core desired/active agent revision
  or reads GitHub `main` as runtime status.

Acceptance:

- a new user can create one agent without operator shell access;
- dashboard web chat completes multiple real Hermes turns;
- Telegram pairing completes a real round trip;
- Google authorization completes one real scoped operation;
- Finite Sites publishes, lists, and previews a test site in chat/dashboard;
- Finite Brain creates, reads, and survives restart of a test knowledge item;
- the first turn sees the bundled Finite-specific skills on a fresh agent, while
  restarting or replacing the image does not silently update an existing
  agent's baseline;
- `finite skills sync` changes an existing agent only after explicit invocation
  and has no Core, Runner, polling, or RMP path;
- a second Device joins and stays independent of Hosted Web Device uptime;
- Electron uses the same Account Auth flow as SaaS, enrolls a new local Device,
  and never reuses or depends on the Hosted Web Device store;
- Core records the runtime provider id and deployed image/ref;
- generic restart/recovery is visible, works against the same runtime record,
  preserves identity, Hermes memory, workspace state, and user files, and does
  not acquire product-feature configuration behavior;
- destructive recovery deletes and restores Agent Runtime, Chat, Brain, Sites,
  and Core primaries from service-consistent off-host Recovery Snapshots while
  preserving usable keys or an explicit readable export/migration path;
- Runtime Retirement preserves a restore-verified snapshot and Purge User Data
  remains unreachable from normal lifecycle and billing controls;
- Stripe billing gates the launch without changing the runtime test shape;
- the full Mandatory Outage And Restart Matrix passes and its evidence is
  attached to the Finite Product Release manifest.

## Parked: Tinfoil Without Durable Mounts

Tinfoil can remain a later confidential-runtime target, but it is not the
default until durable state is solved without reintroducing a bespoke backup
control plane. Any future Tinfoil rung must pass the same Docker-equivalent
runtime contract before becoming a user-facing option.
