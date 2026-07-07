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
    fixtures.rs
    components/
      mod.rs
      postgres.rs
      core.rs
      chat.rs
      sites.rs
      dashboard.rs
    clients/
      mod.rs
      core.rs
      chat.rs
      sites.rs
    local_ui/
      mod.rs
      mprocs.rs
```

Responsibilities:

- `lib.rs`: public harness API.
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
- `fixtures.rs`: `run_devfinity_test`-style helpers for integration tests.
- `local_ui/`: optional local log viewer integration such as mprocs.

The target is a devimint-style harness: Rust owns process lifecycle, readiness,
environment, logs, and cleanup. A local TUI should be only a viewer over logs
and shell state. Fedimint's `mprocs` setup is the model: small viewer config,
not the orchestrator.

Do not copy `devimint` block for block. Copy the contract and ownership model:
one library-owned stack object starts child processes, exposes typed context to
tests, tears processes down reliably, and writes logs/artifacts for humans and
CI. Avoid importing Fedimint-specific federation, gateway, Bitcoin, version
selection, and CLI complexity.

Process-compose was a transitional implementation detail from the first
prototype and has been retired from the compiled harness. New architecture work
should not reintroduce `runtime::process_compose` or a generic
`RuntimeBackend` trait unless another real backend appears.

## Dependency Rule

`devfinity/Cargo.toml` should grow when `devfinity` actually uses a Finite
crate through Rust APIs.

Do not bulk-add every workspace crate only to resemble Fedimint. Add
dependencies as typed harness boundaries appear:

- Add Core crates when Core component config, clients, fixture data, or
  assertions use Core types.
- Add Chat crates when Chat component config, clients, fixture data, or
  assertions use Chat types.
- Add Sites crates when Sites component config, clients, fixture data, or
  assertions use Sites types.
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

- [ ] Add `vars.rs` with a `DevfinityVars` or `DevfinityGlobal` object that
      owns generated paths, ports, environment values, log paths, ready/error
      marker paths, and shell exports.
- [ ] Move `StackEnv` construction and env-file rendering out of `topology.rs`
      into `vars.rs`.
- [ ] Keep deterministic `default` run state and ports for manual local
      development.
- [ ] Add unique fixture run directories for tests, using temp dirs or an
      explicit test-dir argument similar to devimint's `--test-dir`.
- [ ] Add non-conflicting fixture port allocation while preserving deterministic
      manual ports.
- [ ] Add `fixtures.rs` with `DevfinityTestContext`.
- [ ] Add `run_devfinity_test` or an equivalent sync/async fixture entrypoint
      that accepts a closure, starts the stack, waits for readiness, runs the
      closure, and tears the stack down.
- [ ] Make the fixture context expose generated paths, ports, environment,
      log/artifact locations, and service URLs.
- [ ] Make fixture startup apply generated environment variables to the test
      process before constructing clients, matching devimint's env-first
      contract.
- [ ] Add ready/error marker files for external wrappers that need to wait for
      a stack from outside the process.
- [ ] Move the ignored Rust smoke test onto the fixture API.
- [ ] Keep shell smoke tests as compatibility checks, not the primary model.
- [ ] Add or document a `devfinity env` command only if shell workflows need to
      print the current run environment without starting a new stack.

Exit criterion: a Rust integration test can start the base profile through
`run_devfinity_test`, consume generated env/context, and shut the stack down
without calling the CLI directly.

## Phase 4: Process Reaper Hardening

Goal: make process teardown closer to devimint before relying on fixtures in CI
or parallel local tests.

- [ ] Add `process_reaper.rs`.
- [ ] Move SIGTERM/SIGKILL/wait logic out of `ProcessHandle` into the reaper.
- [ ] Ensure dropped handles synchronously reap children or otherwise guarantee
      ports and file locks are released before a replacement process starts.
- [ ] Reap recently killed children before spawning a new daemon.
- [ ] Keep cleanup scoped to devfinity-owned process metadata.
- [ ] Add tests for stale PID/control-file cleanup and repeated start/stop on
      the same ports.

Exit criterion: repeated fixture runs can start and stop the base stack without
port/file-lock races, and cleanup remains scoped to devfinity-owned processes.

## Phase 5: Typed Base Components

Goal: make Core, Chat, Sites, Dashboard, and Postgres first-class typed
components instead of service specs embedded in `stack.rs`.

- [ ] Add a `Component` abstraction for name, state, command, environment,
      readiness, and dependencies.
- [ ] Implement `components::postgres`.
- [ ] Implement `components::core`.
- [ ] Implement `components::chat`.
- [ ] Implement `components::sites`.
- [ ] Implement `components::dashboard`.
- [ ] Keep Postgres as typed infrastructure required by Core.
- [ ] Move Core-specific environment construction into `components::core`.
- [ ] Move Chat-specific SQLite state and environment construction into
      `components::chat`.
- [ ] Move Sites-specific state and environment construction into
      `components::sites`.
- [ ] Add only the Core, Chat, and Sites Rust dependencies that are needed for
      typed config, clients, fixture data, or assertions.
- [ ] Add `clients::core` for Core health and development-account workflows.
- [ ] Add `clients::chat` for Chat health and basic message/server workflows.
- [ ] Add `clients::sites` for Sites health and basic site/document workflows.
- [ ] Convert the current smoke flow to use the Core, Chat, and Sites clients
      where practical.

Exit criterion: Core, Chat, and Sites can be started, inspected, and tested
through typed devfinity components and clients, while the Rust process manager
owns lifecycle and cleanup.

## Phase 6: Local UI And Shell Contract

Goal: add local ergonomics only after lifecycle, env, fixtures, and base
components are library-owned.

- [ ] Add a tiny mprocs-style local viewer that tails generated logs and opens a
      developer shell with generated env loaded.
- [ ] Keep the viewer as a wrapper around `devfinity up -- <command>` or the
      fixture/env contract; it must not decide startup order or own teardown.
- [ ] Add a checked-in viewer config that tails Core, Chat, Sites, Dashboard,
      Postgres, and devfinity logs.
- [ ] Add shell aliases/helpers only as consumers of generated env.

Exit criterion: local developers can get the devimint-style log/shell
experience without adding a second process orchestrator.

## Phase 7: Base Architecture Checkpoint

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

## Phase 8: Add Identity And Auth

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

## Phase 9: Add Brain, Search, And Remaining Runtime Pieces

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

## Phase 10: App-Facing Development Backend

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
