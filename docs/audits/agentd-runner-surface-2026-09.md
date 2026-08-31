# agentd responsibility census + runner↔Core interface map (2026-09)

Read-only census, ratified by prompt 13 (*"agentd probably is doing a lot more
than it should be doing"* + the runner goal: *a VERY simple interface the
runner talks, with cross-service concerns cut out*). No behavior changes; this
document is the only artifact.

**Basis.** Branch `docs/agentd-runner-census` at `ea6475f0`. The ratifying
spec cited `ff7aad17`; `git diff --stat ff7aad17..ea6475f0 -- finite-agentd
finitecomputer-v2/crates/finite-saas-runner finitecomputer-v2/crates/finite-saas-core
infra/runbooks/deploy-core.md` is empty, so all line citations were re-verified
at `ea6475f0` and hold. Two citation nuances:

- `connections.rs:632-638` (OpenRouter rotation chain) is today
  `select_openrouter_key` at `connections.rs:629-648` (the cited 632-638 is
  the candidate loop inside it).
- `deploy-core.md:42` is exactly the `FC_RUNNER_RUNTIME_ENV_JSON` N-1 fallback
  note; `deploy-core.md:69-71` is the *other* N-1 caveat (no N-1 binary
  rollback once Finite Private epochs accepted traffic), not the env note.

Paths below are relative to the repo root. `agentd` = `finite-agentd`,
"the runner" = `finitecomputer-v2/crates/finite-saas-runner`, "Core" =
`finitecomputer-v2/crates/finite-saas-core`.

---

## Part 1 — what finite-agentd actually owns

Concept bar: **agentd = supervise + quiesce + host for finite-shell (payload
generations)**. Everything is judged against that. Module sizes (pre-test
lines / total): daemon.rs 1303/1654, config.rs 974/1343, connections.rs
838/1071, supervisor.rs 389/657, ledger.rs 296/390, transport.rs 185/185,
lib.rs 104/104.

### CORE — belongs in the concept's agentd (8)

| # | Responsibility | Entry points (verified) |
|---|---|---|
| C1 | Supervision of the three children (finitechat sidecar, python health server, Hermes gateway) with restart/backoff and crash-loop surfacing into a published status wire shape | `supervisor.rs:201-339` (`start_supervisor`, `supervise_process`, `spawn_process`), state model `supervisor.rs:14-135`, `restart_hermes` wait `supervisor.rs:151-188`; children wired at `daemon.rs:344-348`, specs `daemon.rs:1136-1185` |
| C2 | Process-group quiesce: each child leads its own pgrp; TERM→10s→KILL drain; post-exit orphan sweep; agentd setsids as group leader when parented by PID 1; SIGTERM routed through the same graceful path as ctrl-C | `supervisor.rs:319-373` (`process_group(0)` at 332, in-process `signal_group` 341-350, `terminate_child` 352-373), `daemon.rs:394-408`, `daemon.rs:411-432` (`become_process_group_leader`, contract with `finitechat/containers/agent/entrypoint.sh:265-270`) |
| C3 | Boot-scoped health evidence: stale bridge/status/ready files removed before any writer spawns, so `/healthz` can only describe this boot | `daemon.rs:443-458` (test `daemon.rs:1363-1409`); consumed by `finitechat/containers/agent/health_server.py:136-166,339-343` |
| C4 | Agent Platform Channel transport: loopback-only bridge client, long-poll inbound stream with bounded buffers, ack/result/state posting, bridge readiness gate (180s default) | `transport.rs:22-185`, `daemon.rs:1187-1198` (stream retry), `daemon.rs:1224-1239` + `daemon.rs:1200-1206` (readiness), `daemon.rs:460-484` (non-blocking redelivery loop) |
| C5 | Durable command ledger: pending resume, terminal replay, request-id byte-conflict rejection | `ledger.rs:39-186` (`command_ledger` table `ledger.rs:52-58`), `ledger.rs:280-283`; decision point `daemon.rs:704-716` |
| C6 | Authorization ledger + first-claimer-wins `agent.owner.claim` + env seeding of principals | `ledger.rs:74-77, 89-123`; `daemon.rs:46`, claim logic `daemon.rs:686-716`, execute arm `daemon.rs:744-747`; seeding `daemon.rs:152-158, 315-319` |
| C7 | Status publication: 1 Hz private `status.json` heartbeat, post-command `runtime.agentd` state snapshots, `finite-agentd status` CLI | `daemon.rs:1241-1273` (writer), `daemon.rs:997-1018` (snapshot), `daemon.rs:1275-1295` (private atomic write), `main.rs:16-21`. The health server re-serves this to the runner's readiness probe (`health_server.py:18-19,308-343`) |
| C8 | Hermes restart sequencing on config transitions (restart, rollback-and-restart on activation/verification failure) | `daemon.rs:626-643`, `daemon.rs:748-752`, `daemon.rs:783-807`, `daemon.rs:892-904`, `daemon.rs:939-975` — the supervisor action is core; what *decides* a restart is R9-R13 below |

### MISPLACED — real job, wrong home (6)

| # | Responsibility | Entry points (verified) | Right home |
|---|---|---|---|
| M1 | Hermes YAML config engine: path-addressed get/set, preview/apply/rollback with durable pre-images, ownership hashes, per-path allowlists, redaction, atomic writes | `config.rs:141-321` (ConfigManager), `config.rs:524-555`, validators `config.rs:700-893`, path/hash/redact helpers `config.rs:895-973`; ledger tables `ledger.rs:59-73, 188-273` | finite-shell payload (Hermes-side config service / `hermes` CLI); agentd keeps command routing + the durable ledger |
| M2 | Inference profile application + **OpenRouter key custody and rotation chain** (requested → durable `.env` → legacy config key → provisioned env), dotenv parse/upsert engine, credential snapshot/restore | rotation inputs `connections.rs:158-176`, chain `connections.rs:629-648`, `.env` path `connections.rs:552-554`, dotenv engine `connections.rs:650-706`, stage/restore `connections.rs:204-230`; apply choreography `daemon.rs:761-767, 906-976` | finite-shell connection layer; key custody belongs with whichever home ends up owning credentials (Core already mints Finite Private keys — see E11) |
| M3 | Telegram connection family: connect/home/disconnect offers, token validation, pairing-code approval via `hermes pairing approve` shell-out | `connections.rs:232-349` (incl. shell-out `connections.rs:321-349`), `connections.rs:485-537`; arms `daemon.rs:810-838` | finite-shell connection service; the Hermes-CLI invocation is payload knowledge |
| M4 | Google Workspace OAuth custody: client-secret/token/metadata files, scopes contract, `setup.py --check/--revoke` shell-outs **into a managed skill's internal tree** (`managed-skills/finite/current/productivity/google-workspace-finite/…`) | `connections.rs:351-423`, status `connections.rs:474-483`, skill-root paths `connections.rs:539-571`, auth check `connections.rs:573-595`, token shape `connections.rs:598-610`; arms `daemon.rs:839-854` | the skill itself / finite-shell. Deepest knowledge leak in agentd: the daemon knows a *skill's* internal scripts and reference files |
| M5 | AEON specialization product logic: startup bundle admission from `FINITE_SPECIALIZATION_BUNDLE`/`FINITE_SPECIALIZATION_WORKER_API_KEY`, desired-state derivation, config writes, continuous verifier loop, python vision probe with pinned semantic output | `daemon.rs:53-56, 219-292` (bundle parse + status), `daemon.rs:325-337, 349-358` (activation + verifier spawn), probe `daemon.rs:500-539`, verifier `daemon.rs:541-608`; `config.rs:17-95, 323-522, 575-698`; probe script ships in the image (`finitechat/containers/agent/probe_hermes_vision.py`) | finite-shell payload generation / specialization worker config; agentd at most restarts Hermes afterward |
| M6 | Chat admission boot seeding + hosted/locked marker for the sidecar | locked default `daemon.rs:1094-1102`, seed `daemon.rs:1104-1134`, sidecar env `daemon.rs:1136-1161` (spec-known-members citation `daemon.rs:1094-1134` verified exact) | sidecar/entrypoint only: the enforcing seed already runs in the sidecar's own boot; agentd's copy is the ordering optimization its own comment admits is best-effort (`daemon.rs:1104-1118`) |

### DEAD-WEIGHT — nothing in the mono needs it (4)

Proof method: repo-wide grep for each command string across `.rs`, `.ts`,
`.tsx`, `.py` (excluding `finite-agentd` itself, node_modules, target), plus a
check for dynamically-built command names in the dashboard sender
(`finitecomputer-v2/apps/dashboard/src/lib/hosted-agent-controls.ts`). Caveat:
the channel is a chat protocol, so an out-of-tree sender is *possible*; within
the first-party mono (per doctrine, the only first-party surface) these have
no sender.

| # | Responsibility | Entry points (verified) | Proof |
|---|---|---|---|
| D1 | `agent.hermes.restart` command arm | `daemon.rs:748-752` | 0 senders. Restarts are owned by Core runtime-control → runner (`finite-saas-core/src/api.rs:806-812` → kata restart path) |
| D2 | `agent.chat.recover` → bridge `/v1/hermes/recover` | `daemon.rs:753-756`, `transport.rs:132-135` | 0 senders. The live recovery path is Core's recover-known-good-chat control → runner recovery boot (`kata.rs` recovery environment), not this chat command |
| D3 | `agent.specialization.aeon.reconcile` remote path (desired state over the wire, rollback + verify helpers, `extra_body`-carrying reconcile) | arm `daemon.rs:768-809`, helpers `daemon.rs:626-674`, `config.rs:323-389` + target/validate `config.rs:619-698` | 0 senders. Only the startup-env bundle path (M5's runner-driven half) is live |
| D4 | `agent.hermes.config.preview` / `.apply` / `.rollback` remote commands | `daemon.rs:855-887` | 0 senders for all three. The underlying `ConfigManager.apply` is still used internally by the Telegram/inference offers and the startup bundle — only the *generic remote* surface is dead |

### The command-family reality check

16 command families are advertised (`README.md:29-44`). Senders found:
`agent.owner.claim` (hosted-device, 7 refs), `agent.connections.status`
(finitechat-core + dashboard), `agent.inference.apply`, `agent.telegram.*`
(4), `agent.google.apply`/`disconnect` (dashboard), `agent.status.inspect`
(`finitechat-cli/src/hermes.rs`). **Six families have no sender**: the four
dead-weight items above plus nothing else — i.e. the generic Hermes-config
surface, the remote AEON reconcile, hermes.restart, and chat.recover are
carried in every build, README, and compat reasoning for zero first-party
callers.

### Ordered slimming sketch — explicitly NOT executed

1. **Delete the four dead command families and their exclusive machinery**
   (~250-400 LOC in `daemon.rs`: D1-D4 arms, `rollback_aeon_specialization` /
   `verify_aeon_specialization`, the `/v1/hermes/recover` transport call;
   `config.rs::reconcile_aeon_specialization` + the `extra_body` reconcile
   target become internal-only or follow M5 out). Removes 6 of 16 wire
   commands and one bridge endpoint dependency. Cheapest, most reversible.
2. **Move Google Workspace custody out** (~350 LOC: `connections.rs:351-423,
   539-610` + arms). Highest coupling-to-concept ratio in the daemon; also
   kills the managed-skill-internals knowledge.
3. **Move the connection product (Telegram + inference/OpenRouter) to a
   finite-shell connection service** (~500 LOC in `connections.rs` + arms
   `daemon.rs:761-854`); agentd keeps typed routing and the durable ledger.
   Resolves the "who rotates the OpenRouter key" question into one home.
4. **Move the Hermes config engine (M1)** (~970 pre-test LOC, all of
   `config.rs` + ledger config tables) behind a narrow "apply allowlisted
   patch, validate, restart" contract owned by the payload. Largest move; do
   after 1-3 prove the pattern.
5. **Collapse admission seeding to the sidecar's enforcing copy** (~60 LOC,
   `daemon.rs:1104-1161`) once the gateway's first-read ordering is proven to
   be handled by the entrypoint rather than by agentd's duplicate seed.
6. **What must stay**: supervisor + quiesce + boot hygiene + transport +
   ledger + authorization + status publication (C1-C7) — roughly 1,460
   pre-test LOC across `supervisor.rs`, `transport.rs`, `ledger.rs` and the
   corresponding `daemon.rs` scaffolding. That residual *is* the concept's
   agentd.

---

## Part 2 — runner↔Core interface map

### Topology

- **Transport:** runner → Core, HTTP only (`ureq`, sync, bearer
  `FC_CORE_RUNNER_API_TOKEN`, `FC_CORE_URL`; `main.rs:170-171`,
  `lib.rs:2115-2127`). Core never initiates HTTP toward the runner (Core's
  only outbound client is WorkOS JWKS, `finite-saas-core/src/auth.rs:4,294`).
  No webhooks, no streaming from Core.
- **No shared database:** all durable rows (agent creation requests, runtime
  control requests, runtime records, artifacts, health reports, Finite
  Private keys) are Core-owned Postgres; the runner has no Postgres driver at
  all. The schema coupling is at the DTO level — see E14.
- **Cadence:** `serve()` loops `run_cycle()` at `FC_RUNNER_IDLE_INTERVAL_MS`
  (default 1000 ms, min 100; `main.rs:499-543`). Each cycle: forward due
  health reports → lease runtime control → else lease creation
  (`lib.rs:509-560`). Error backoff doubles to 30 s.
- **Auth:** per-runner credential, `runner_id` constant-time binding,
  capacity/class authorization incl. a legacy-Kata compatibility shim
  (`api.rs:2271-2321`).

### Edge census — 21 edges: 13 SIMPLE, 7 LEAKY, 1 REMOVABLE

| # | Edge | Where | Class | Note |
|---|---|---|---|---|
| E1 | `POST /api/core/v1/runtime-control-requests/lease` | `lib.rs:2155` | SIMPLE | The work queue; polled every cycle |
| E2 | `POST …/runtime-control-requests/{id}/renew` | `lib.rs:2173` | SIMPLE | Lease discipline (retirement holds 1 h leases) |
| E3 | `POST …/runtime-control-requests/{id}/retry` | `lib.rs:2184` | SIMPLE | |
| E4 | `POST …/runtime-control-requests/{id}/complete` | `lib.rs:2196` | SIMPLE | Terminal state |
| E5 | `POST …/runtime-control-requests/{id}/fail` | `lib.rs:2209` | SIMPLE | Terminal state |
| E6 | `POST /api/core/v1/agent-creation-requests/lease` | `lib.rs:2222` | SIMPLE | |
| E7 | `POST …/agent-creation-requests/{id}/complete` | `lib.rs:2241` | SIMPLE | |
| E8 | `POST …/agent-creation-requests/{id}/runtime` | `lib.rs:2255` | SIMPLE | Registers the launched runtime |
| E9 | `POST …/agent-creation-requests/{id}/provider-operation/transitions` | `lib.rs:2269` | SIMPLE | Provider operation journal (Phala/Kata) |
| E10 | `POST …/agent-creation-requests/{id}/fail` | `lib.rs:2296` | SIMPLE | Carries the provisioned-key revoke id |
| E11 | `POST …/agent-creation-requests/{id}/finite-private-key` | `lib.rs:2283`, Core `api.rs:2104` | **LEAKY** | Split-brain inference config: Core mints/holds/revokes the Finite Private credential, but `base_url`/`model`/context length are runner env (`main.rs:181-190`, `lib.rs:63-70`); the deploy runbook records the migration as half-done — three keys moved into `FC_CORE_RUNTIME_ENV_JSON`, the model/base_url split not (`deploy-core.md:38-46`) |
| E12 | `GET /api/core/v1/runtime-artifacts/{id}` — **every cycle** | `lib.rs:2135`, fetched unconditionally at `main.rs:173-174` | **REMOVABLE** (as a per-cycle call) | An immutable artifact descriptor is re-fetched ~1 Hz per runner forever; nothing needs that. Serve artifact facts inside the lease payload or cache. Keep the endpoint for on-demand use |
| E13 | `POST /api/core/v1/runtime-health-reports` | `lib.rs:2305`, ferry `health_reports.rs`, Core `api.rs:741` | SIMPLE | Outbound-only, lossy by contract; Core projects staleness from the runner's reported interval |
| E14 | Compile-time crate dependency runner → core | `finite-saas-runner/Cargo.toml` (`finite-saas-core = { path = … }`); Core lib exports `api`/`auth`/`billing`/`store` (`core lib.rs:1-5`) with axum + deadpool-postgres | **LEAKY** | The runner links Core's router and Postgres layer to reuse DTO types (`lib.rs:2223` uses `finite_saas_core::LeaseAgentCreationRequestInput` directly). The "shared schema" is source coupling, not a wire-contract module; every Core change recompiles and version-locks the runner |
| E15 | Runner-side re-validation of Core's RuntimeSpec | `lib.rs:1738-1783` (port 8080, `/healthz`, `/contact`, Kata=4vCPU/8GiB, Phala=2vCPU/4GiB, sha256 digest form) + binding equality `lib.rs:1650-1736` | **LEAKY** | Deliberate defense-in-depth, but the contract constants have two homes; every spec change is a two-sided change |
| E16 | Duplicated reserved-env-key lists | runner `lib.rs:2027-2065` (35 keys) vs Core `lib.rs:3069-3098` (29 keys); identical secret-shape heuristic duplicated at runner `lib.rs:2068-2072` / Core `lib.rs:3104-3109` | **LEAKY** | Already drifted by 6 keys (`FINITE_PRIVATE_CONTEXT_LENGTH`, `FINITECHAT_HERMES_CONTEXT_LENGTH`, `FINITE_SPECIALIZATION_BUNDLE`, `FINITE_SPECIALIZATION_WORKER_API_KEY`, `FBRAIN_EMBEDDING_ENDPOINT`, `FBRAIN_EMBEDDING_BEARER_TOKEN` are runner-reserved but not Core-reserved). A Core operator setting `FINITE_SPECIALIZATION_BUNDLE` in `FC_CORE_RUNTIME_ENV_JSON` passes Core validation and then fails every runner launch with "owned by the Runtime contract" |
| E17 | `FINITECHAT_OWNER_NPUBS` pass-through | Core `lib.rs:3140-3158` + lease-time injection (`store.rs`, pinned by tests at `store.rs:9248, 9253, 9289`); runner treats it as opaque, deliberately non-reserved spec env | SIMPLE | **The model edge**: per-request state owned by Core, opaque to the runner, consumed once at birth by the sidecar; no-carry-forward on upgrade is documented (`core lib.rs:2969-2974`) |
| E18 | Launch base env hardcodes the chat-admission trio | `lib.rs:2949-2951` (`FINITECHAT_ALLOW_ALL_USERS` / `FINITE_ALLOW_ALL_USERS` / `GATEWAY_ALLOW_ALL_USERS` = `true`), Core spec env appended after at `lib.rs:3019-3030` | **LEAKY** | The runner knows chat-admission semantics; the allow-all vs owner-npubs precedence is reconciled invisibly inside the image. The admission default belongs to one owner (Core request state), not three env keys in the launcher |
| E19 | Runner knows agentd's internals | readiness deadline chosen to match agentd's 180 s bridge timeout (`runner lib.rs:49-53` vs `daemon.rs:1200-1206`); `ready_reason` vocabulary incl. `agentd_status_stale` understood runner-side (`kata.rs:3976, 4010`; produced by `health_server.py:339-343`) | **LEAKY** | `/healthz` shape is a fine contract; the matched timeout and the reason vocabulary are a three-service timing/semantics coupling. If agentd's timeout changes, the runner's silently mis-matches |
| E20 | Specialization bundle admission lives in the runner | attestation envs `main.rs:191-213`; canonical-profile gate `lib.rs:1565-1602`; injection incl. FBRAIN endpoint/token aliasing `lib.rs:2997-3016`; consumed by agentd `daemon.rs:54-55, 219-268` | **LEAKY** | The policy lives in runner env, the consumer is agentd, Core is unaware. The AEON worker credential flows runner-env → agent-env → Hermes config with no Core record of *why* a runtime got it |
| E21 | `FC_RUNNER_*` operator env surface (~45 names) + `FC_RUNNER_RUNTIME_ENV_JSON` N-1 fallback | `main.rs` throughout; fallback `main.rs:667-672`; runbook `deploy-core.md:42` | SIMPLE | Contracts as such are fine; the env-JSON fallback is a documented transitional dual-write (Core's `FC_CORE_RUNTIME_ENV_JSON` is primary, `core main.rs:1596-1606`) |

Adjacent (not runner↔Core, noted for completeness): the runner also binds
agent identity directly against the Finite Identity Authority
(`/api/v1/operator/agent-email-bindings`, `lib.rs:731-800`) — a third service
the launcher talks to, same "launcher as trusted operator client" pattern.

### Input for the simple-interface design pass

WHO-knows-WHAT, distilled:

- **Core owns (clean):** request/control lifecycle rows, lease discipline,
  placement, artifacts, Finite Private key minting, owner-npubs request state.
- **Runner owns (clean):** provider adapters (docker/kata/apple/phala), the
  lease-poll loop, capacity advertisement, standing `/contact` ferry.
- **Split or duplicated (the cut list):** inference endpoint/model (E11),
  the runtime-image env contract as two drifted lists (E16), admission
  defaults (E18), readiness timing + reason vocabulary (E19),
  specialization admission policy (E20), DTO schema via crate coupling (E14),
  spec re-validation constants (E15).
- **The pattern to copy:** E17 (owner-npubs) — Core computes per-request
  state, injects it as an opaque, deliberately-unreserved spec key, the
  runner never interprets it, the image consumes it exactly once. Every cut
  above can be phrased the same way: one computing owner, one opaque
  transport, one consumer.
- **Free wins:** E12 (stop fetching the artifact every second), Part 1's
  dead command families (delete, no design needed).

---

## Surprises relative to the spec's known-members list

1. **Six of sixteen agentd command families have no sender in the mono** —
   including the entire generic Hermes-config remote surface and the remote
   AEON reconcile the README advertises as current behavior.
2. **agentd shells into a managed skill's internal tree** (Google Workspace
   `scripts/setup.py --check/--revoke`, `references/…-scopes.json`) — the
   deepest knowledge leak found, not on the known-members list.
3. **The runner links the whole Core crate** (axum router + Postgres store)
   just to reuse DTO types — the schema sharing is compile-time source
   coupling, not a wire contract.
4. **The reserved-env lists have already drifted** (runner 35 vs Core 29
   keys) with a live failure mode (Core-side `FINITE_SPECIALIZATION_BUNDLE`
   would brick every launch).
5. **agentd contains a dotenv engine** (parse + upsert + restore of Hermes
   `.env`) for OpenRouter key custody — a mini config-format implementation
   beyond the YAML engine.
6. **The supervised health server's whole job is to re-serve agentd's own
   `status.json`** as `/healthz` with a liveness window — status indirection
   that the runner then interprets (E19).
7. **The launcher hardcodes allow-all chat admission** in every launch env
   and relies on the image to reconcile it with Core's owner-npubs seed
   (E18).
