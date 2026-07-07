# Devfinity Architecture Plan

`devfinity` should evolve from a pragmatic local process launcher into the
Finite integration harness, following the same broad role Fedimint gives
`devimint`: a typed Rust library for constructing local stacks, running
integration tests, and backing ergonomic local development.

This plan is intentionally narrow. The first checkpoint is to get the harness
architecture right for the base local profile: Core plus Chat plus Sites.
Identity, Auth, Brain, Search, and app-facing development integration are
follow-up phases after the base profile architecture feels stable.

## Design Target

The durable shape should be:

```text
devfinity/
  src/
    lib.rs
    main.rs
    topology.rs
    vars.rs
    process.rs
    process_reaper.rs
    stack.rs
    components/
      mod.rs
      postgres.rs
      core.rs
      chat.rs
      sites.rs
    clients/
      mod.rs
      core.rs
      chat.rs
      sites.rs
```

Responsibilities:

- `lib.rs`: public harness API, including the `run_devfinity_test` fixture
  entrypoint, mirroring devimint's small `run_devfed_test` shape.
- `main.rs`: thin CLI over the library.
- `topology.rs`: profiles and stable stack shape.
- `vars.rs`: generated paths, ports, environment contract, shell export helpers,
  and unique fixture run allocation, following devimint's `vars.rs` role.
- `process.rs`: Rust-owned `ProcessManager` and `ProcessHandle` equivalents,
  following the durable `devimint` pattern.
- `process_reaper.rs`: small SIGTERM/SIGKILL/wait helper used by process
  handles so ports and file locks are released before subsequent runs.
- `stack.rs`: stack startup, readiness, wrapped-command execution, status, and
  teardown.
- `components/`: typed service definitions, commands, logs, environment, and
  readiness probes.
- `clients/`: typed or semi-typed harness clients used by tests.

The target is a devimint-style harness: Rust owns process lifecycle, readiness,
environment, logs, and cleanup for backend infrastructure and Rust-controllable
services. A local TUI or app-dev process composer should live outside
devfinity and consume generated env/ready markers. Fedimint's `mprocs` setup is
the model for the outer layer: small viewer/composer config, not the backend
orchestrator.

Do not copy `devimint` block for block. Copy the contract and ownership model:
one library-owned stack object starts child processes, exposes the stack handle to
tests, tears processes down reliably, and writes logs/artifacts for humans and
CI. Avoid importing Fedimint-specific federation, gateway, Bitcoin, version
selection, and CLI complexity.

Do not make devfinity launch Rust services from the source checkout. The current
intermediate implementation still does this for Core, Chat, and Sites by
building workspace binaries and spawning `target/debug/...`; that is a
transitional violation to remove next. The durable shape is for `devfinity` to
depend on the relevant Rust crates and start their library server entrypoints
directly. If a service does not expose the right library API yet, add that API
to the service crate before wiring it into devfinity.

Non-Rust services are outside devfinity. For the dashboard, prefer a very small
checked-in `just` recipe or script owned by an outer dev process composer. That
outer layer should start or wrap devfinity for backend infra, wait for the
devfinity readiness/env contract, then start dashboard or other UI commands.
Do not spread dashboard source paths, npm install rules, or framework-specific
startup details through the Rust harness.

Process-compose was a transitional implementation detail from the first
prototype and has been retired from the compiled harness. New architecture work
should not reintroduce `runtime::process_compose` or a generic
`RuntimeBackend` trait unless another real backend appears.

## Dependency Rule

`devfinity/Cargo.toml` should grow when `devfinity` actually uses a Finite
crate through Rust APIs.

Do not bulk-add every workspace crate only to resemble Fedimint. Add
dependencies as typed harness boundaries appear:

- Add Core crates when the Core component starts the Core server through a
  library entrypoint, or when clients, fixture data, or assertions use Core
  types.
- Add Chat crates when the Chat component starts the Chat server through a
  library entrypoint, or when clients, fixture data, or assertions use Chat
  types.
- Add Sites crates when the Sites component starts the Sites server through a
  library entrypoint, or when clients, fixture data, or assertions use Sites
  types.
- Add later component crates only in the phase where those components become
  typed devfinity components.

This keeps the harness library-first without making `status`, `cleanup`, or
basic config generation compile against unrelated packages.

## Phase 1: Library-First Core

Goal: preserve current behavior while making `devfinity` architecturally ready
to become a typed harness. This phase landed during the initial
process-compose prototype and was later superseded by the Rust-owned
orchestrator.

- [x] Keep `main.rs` as a thin CLI wrapper.
- [x] Move the current stack construction API into stable library types.
- [x] Add `DevfinityStack` as the main public stack object.
- [x] Add `StackProfile` with an initial Core plus Chat plus Sites base profile.
- [x] Add typed `StackPaths`, `StackPorts`, and `StackEnv` structs.
- [x] Move process-compose YAML generation and lifecycle helpers behind a
      concrete `runtime::process_compose` module.
- [x] Avoid a `RuntimeBackend` trait until a second backend or test double forces
      that abstraction.
- [x] Keep existing `just dev up`, `just dev status`, `just dev cleanup`,
      `just dev smoke`, and `just dev rust-smoke` behavior working.
- [x] Keep native Nix Postgres as infrastructure owned by the base profile.
- [x] Add tests that prove the CLI delegates through the library layer rather
      than owning stack construction itself.

Exit criterion: the harness still behaves the same, but the core types make it
clear where profiles, runtime ownership, components, and fixtures belong.

## Phase 2: Devimint-Style Rust Orchestrator

Goal: replace process-compose as the core runtime with a Rust-owned process
manager that can run locally, run in CI, and become the foundation for test
fixtures.

- [x] Add `process.rs` with `ProcessManager` and `ProcessHandle`-style types.
- [x] Spawn each child with explicit command, working directory, environment,
      log file, and shutdown behavior.
- [x] Make child handles clean up on drop and on explicit stack shutdown.
- [x] Add shared polling helpers for TCP, HTTP, and command/readiness checks.
- [x] Add `RunningDevfinityStack` or equivalent to own live process handles.
- [x] Move `up --headless -- <command>` onto the Rust process manager.
- [x] Keep per-service logs under the run directory for CI artifact retention.
- [x] Keep native Postgres startup owned by devfinity, but model it as a normal
      managed process.
- [x] Preserve deterministic `default` run state for local development.
- [x] Retire process-compose after the Rust orchestrator passed the same
      dry-run, smoke, and status checks.
- [x] Remove `runtime::process_compose` once the Rust orchestrator has feature
      parity for the base profile.

Exit criterion: `devfinity up`, `up --headless -- <command>`, `status`, and
`cleanup` no longer depend on process-compose for orchestration, and the base
smoke passes with Rust-owned child processes.

## Phase 3: Devimint-Style Env And Fixtures

Goal: close the main structural gap with devimint before adding more component
abstractions. The library should expose a generated environment/global state
object and a test fixture entrypoint that starts the stack, waits for readiness,
runs test code, and tears everything down.

- [x] Add `vars.rs` with a `DevfinityVars` or `DevfinityGlobal` object that
      owns generated paths, ports, environment values, log paths, ready/error
      marker paths, and shell exports.
- [x] Move `StackEnv` construction and env-file rendering out of `topology.rs`
      into `vars.rs`.
- [x] Keep deterministic `default` run state and ports for manual local
      development.
- [x] Add unique fixture run directories for tests, using temp dirs or an
      explicit test-dir argument similar to devimint's `--test-dir`.
- [x] Add non-conflicting fixture port allocation while preserving deterministic
      manual ports.
- [x] Add `run_devfinity_test` in `lib.rs`, following devimint's small
      `run_devfed_test` shape.
- [x] Add a sync fixture entrypoint
      that accepts a closure, starts the stack, waits for readiness, runs the
      closure, and tears the stack down.
- [x] Pass the `DevfinityStack` handle to fixture tests so generated paths,
      ports, environment, log/artifact locations, and service URLs stay on the
      primary stack object.
- [x] Make fixture startup apply generated environment variables to the test
      process before constructing clients, matching devimint's env-first
      contract.
- [x] Add ready/error marker files for external wrappers that need to wait for
      a stack from outside the process.
- [x] Move the ignored Rust smoke test onto the fixture API.
- [x] Keep shell smoke tests as compatibility checks, not the primary model.
- [x] Defer `devfinity env` until a shell workflow needs to print the current
      run environment without starting a new stack.

Exit criterion: a Rust integration test can start the base profile through
`run_devfinity_test`, consume generated env/stack data, and shut the stack down
without calling the CLI directly.

## Phase 4: Rust Backend Service Boundaries

Goal: replace source-checkout binary launches deliberately. Do not start by
deleting `repo_root`; it is only removable after Core, Chat, and Sites have
usable library server entrypoints and devfinity has a way to supervise
in-process backend tasks.

Current state:

- [x] Remove Dashboard from `DevfinityStack`, `StackPorts`, `StackPaths`,
      generated env, service readiness checks, and process ownership.
- [x] Move the dashboard create-agent smoke out of devfinity's backend fixture
      tests.
- [x] Keep native Postgres as an external infrastructure process because it is
      not a Rust service crate.
- [ ] Add the dashboard create-agent smoke to an outer dev/e2e composer
      workflow.

Boundary audit:

- [x] Document the existing server library surfaces before changing devfinity:
      Core exposes router/store APIs but its `serve()` wrapper is binary-only;
      Chat exposes `HttpServerState` and `http_router` but not a public server
      runner; Sites exposes `server::serve_on` plus lower-level pieces but
      could use a smaller dev serve helper.
- [x] Decide the in-process service supervision shape for devfinity: a small
      task manager that owns async service tasks, captures failures, and
      coordinates shutdown alongside the native Postgres process.
- [x] Remove the binary-launch backend path after the service entrypoints and
      task manager are in place and the backend smoke still passes.

Service API work:

- [x] Add a public Core server entrypoint, for example `CoreServeOptions` plus
      `serve_core(...)`, in `finite-saas-core` rather than shelling out to the
      binary.
- [x] Add a public Chat server entrypoint, for example `ChatServeOptions` plus
      `serve_chat(...)`, around `finitechat-server`'s durable state and router.
- [x] Add a public Sites dev server entrypoint, for example
      `ServeOptions` plus `serve_sites(...)`, that hides Engine/Mailer/
      Supervisor setup from devfinity.

Devfinity migration:

- [x] Add Core, Chat, and Sites crates to `devfinity/Cargo.toml` only as each
      component starts using its library API.
- [x] Migrate backend services from `target/debug/...` to their
      library entrypoint, keeping `just dev smoke` and `just dev rust-smoke`
      passing.
- [x] Stop running `cargo build` as a devfinity startup preflight once no
      backend service is launched from `target/debug/...`.
- [ ] Remove `repo_root` from `DevfinityStack` and `StackPaths` only after
      service startup and wrapped-command execution no longer need it.
- [ ] Remove `default_repo_root()` from `lib.rs` after fixture startup no
      longer needs source-checkout path recovery.
- [ ] Prove `run_devfinity_test` works regardless of the Cargo test process
      current directory.

Exit criterion: Core, Chat, and Sites are started through Rust crate
dependencies, Dashboard remains outside devfinity, the backend smoke still
passes, and devfinity no longer needs a source-checkout repo root to run
fixture tests.

## Phase 5: Process And Task Shutdown Hardening

Goal: make teardown closer to devimint before relying on fixtures in CI or
parallel local tests. After Phase 4, this covers both native Postgres process
cleanup and in-process Rust backend task shutdown.

- [ ] Add a small shutdown/reaper module for native child processes and
      in-process service tasks.
- [ ] Move SIGTERM/SIGKILL/wait logic out of `ProcessHandle` into the reaper.
- [ ] Ensure dropped handles synchronously reap children or otherwise guarantee
      ports and file locks are released before a replacement process starts.
- [ ] Ensure dropped service task handles request shutdown and observe task
      completion or failure.
- [ ] Reap recently killed children before spawning a new daemon.
- [ ] Keep cleanup scoped to devfinity-owned process/task metadata.
- [ ] Add tests for stale PID/control-file cleanup and repeated start/stop on
      the same ports.

Exit criterion: repeated fixture runs can start and stop the base stack without
port/file-lock races, failed service tasks are surfaced clearly, and cleanup
remains scoped to devfinity-owned processes/tasks.

## Phase 6: Typed Base Components

Goal: make Core, Chat, Sites, and Postgres first-class typed
components instead of service specs embedded in `stack.rs`. This phase should
build on Phase 4's crate-dependency service startup, not on source-checkout
binary launches.

- [ ] Add a `Component` abstraction for name, state, environment, readiness,
      dependencies, and either an in-process Rust service entrypoint or a
      tightly scoped external infrastructure process.
- [ ] Implement `components::postgres`.
- [ ] Implement `components::core`.
- [ ] Implement `components::chat`.
- [ ] Implement `components::sites`.
- [ ] Keep Postgres as typed infrastructure required by Core.
- [ ] Move Core-specific environment construction into `components::core`.
- [ ] Move Chat-specific SQLite state and environment construction into
      `components::chat`.
- [ ] Move Sites-specific state and environment construction into
      `components::sites`.
- [ ] Keep Core, Chat, and Sites Rust dependencies justified by library
      startup, typed config, clients, fixture data, or assertions.
- [ ] Add `clients::core` for Core health and development-account workflows.
- [ ] Add `clients::chat` for Chat health and basic message/server workflows.
- [ ] Add `clients::sites` for Sites health and basic site/document workflows.
- [ ] Convert the current smoke flow to use the Core, Chat, and Sites clients
      where practical.

Exit criterion: Core, Chat, and Sites can be started, inspected, and tested
through typed devfinity components and clients, while the Rust process manager
owns lifecycle and cleanup.

## Phase 7: Outer Dev Composer And Shell Contract

Goal: add local ergonomics only after backend lifecycle, env, fixtures, and
base components are library-owned by devfinity. UI/dev-server processes live in
this outer layer, not inside devfinity.

- [ ] Add one outer dev composer entrypoint, likely a `just` recipe plus a tiny
      script or mprocs config.
- [ ] Have the outer composer start or wrap `devfinity up --headless`, wait for
      backend readiness, load generated env, then start Dashboard or other UI
      commands.
- [ ] Keep Dashboard npm install/start details in the outer dashboard recipe,
      not in the devfinity crate.
- [ ] Tail Core, Chat, Sites, Postgres, devfinity, and Dashboard logs from the
      outer viewer when useful.
- [ ] Add shell aliases/helpers only as consumers of generated env.

Exit criterion: local developers can get the devimint-style log/shell
experience and dashboard workflow without making devfinity orchestrate
non-backend UI processes.

## Phase 8: Base Architecture Checkpoint

Goal: stop and validate the harness shape before adding more architecture
components.

- [ ] Review whether `DevfinityStack`, `vars.rs`, `process.rs`,
      `process_reaper.rs`, `stack.rs`, components, clients, and fixtures have
      clear ownership
      boundaries.
- [ ] Confirm `devfinity/Cargo.toml` contains only dependencies justified by the
      base harness.
- [ ] Confirm `devfinity status` and `cleanup` remain pragmatic helpers over
      the same typed stack model.
- [ ] Confirm app-facing local development can plausibly consume the library
      API later without depending on CLI parsing.
- [ ] Document the stable base harness API in
      `docs/local-integration-harness.md`.

Exit criterion: the team is comfortable using this architecture as the pattern
for additional components.

## Phase 9: Add Identity And Auth

Goal: make identity and auth explicit harness components instead of implicit
environment conventions.

- [ ] Add typed identity fixture helpers using `finite-identity`.
- [ ] Add Auth component or fixture support using `finite-auth-core` and
      related crates as needed.
- [ ] Replace hard-coded development identities with typed fixture identities
      where practical.
- [ ] Add tests that prove Core and Chat can share the same generated identity
      fixture.

Exit criterion: identity state is owned by devfinity fixtures and can be shared
across Core, Chat, and later components.

## Phase 10: Add Brain, Search, And Remaining Runtime Pieces

Goal: add the rest of the local architecture after the component model has
proved itself.

- [ ] Add Brain component support.
- [ ] Add Brain client or fixture helpers.
- [ ] Add Search component support if it has a local runtime in the monorepo.
- [ ] Add any required shared infra components.
- [ ] Add profiles that compose these pieces without making the default Core
      plus Chat plus Sites path heavy.
- [ ] Add multi-component fixture tests only after each component has a clear
      typed boundary.

Exit criterion: devfinity can stitch together the broader Finite architecture
without losing the simple base development path.

## Phase 11: App-Facing Development Backend

Goal: let apps use devfinity as their local backend and integration-test
substrate.

- [ ] Define the app-facing API surface for starting or connecting to a
      devfinity stack.
- [ ] Add stable environment export helpers for apps.
- [ ] Add profile selection for app workflows.
- [ ] Add documentation for app developers using devfinity as the backend.
- [ ] Keep app workflows consuming the library or generated env, not duplicating
      process orchestration logic.

Exit criterion: apps can run against a devfinity-managed local backend without
each app reinventing stack startup, state layout, or test fixtures.
