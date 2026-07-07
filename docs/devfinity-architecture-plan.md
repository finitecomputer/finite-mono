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
    env.rs
    process.rs
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
- `topology.rs`: profiles, ports, paths, environment, and state layout.
- `env.rs`: generated environment contract and shell export helpers.
- `process.rs`: Rust-owned `ProcessManager` and `ProcessHandle` equivalents,
  following the durable `devimint` pattern.
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
to become a typed harness. This phase landed against the initial
process-compose prototype and is now a compatibility baseline, not the final
runtime direction.

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

Exit criterion: the current harness still behaves the same, but the core types
make it clear where profiles, components, the transitional process-compose
module, and fixtures belong.

## Phase 2: Devimint-Style Rust Orchestrator

Goal: replace process-compose as the core runtime with a Rust-owned process
manager that can run locally, run in CI, and back test fixtures.

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
- [ ] Preserve unique run-state support for future fixture tests.
- [ ] Add a tiny mprocs-style local viewer that tails generated logs, or defer
      it behind a documented `devfinity logs` command if mprocs is not needed
      yet.
- [x] Retire process-compose after the Rust orchestrator passed the same
      dry-run, smoke, and status checks.
- [x] Remove `runtime::process_compose` once the Rust orchestrator has feature
      parity for the base profile.

Exit criterion: `devfinity up`, `up --headless -- <command>`, `status`, and
`cleanup` no longer depend on process-compose for orchestration, and the base
smoke passes with Rust-owned child processes.

## Phase 3: Typed Base Components

Goal: make Core, Chat, and Sites first-class typed components instead of ad hoc
process specs.

- [ ] Add a `Component` abstraction for name, state, command, environment,
      readiness, and dependencies.
- [ ] Implement `components::core`.
- [ ] Implement `components::chat`.
- [ ] Implement `components::sites`.
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

## Phase 4: Rust Fixture API

Goal: make devfinity useful to integration tests as a library, similar in role
to Fedimint's `devimint`.

- [ ] Add `DevfinityTestContext`.
- [ ] Add `run_devfinity_test` or an equivalent async/sync fixture entrypoint.
- [ ] Make fixture runs use unique run directories and non-conflicting ports by
      default, while preserving the named `default` run for manual local
      development.
- [ ] Support headless stack startup around a test closure.
- [ ] Automatically tear down the stack after the test closure exits.
- [ ] Expose typed Core, Chat, and Sites clients from the test context.
- [ ] Expose generated paths, ports, and environment from the test context.
- [ ] Expose log and artifact paths from the test context so CI can retain useful
      failure evidence.
- [ ] Migrate the ignored Rust smoke test onto the new fixture API.
- [ ] Keep shell smoke tests as compatibility checks, not the primary model.

Exit criterion: a Rust integration test can start the base profile through
devfinity, exercise Core, Chat, and Sites through typed clients, and shut the
stack down without calling the CLI directly.

## Phase 5: Base Architecture Checkpoint

Goal: stop and validate the harness shape before adding more architecture
components.

- [ ] Review whether `DevfinityStack`, profiles, components,
      `process.rs`, clients, and fixtures have clear ownership
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

## Phase 6: Devimint-Style Environment Contract

Goal: make the generated devfinity environment the app-facing and shell-facing
contract, following devimint's pattern.

- [ ] Keep `StackEnv` as the canonical list of exported variables.
- [ ] Keep writing an `env` file under the run directory.
- [ ] Keep passing the same generated variables to wrapped commands.
- [ ] Add a library helper that applies the generated variables to the current
      test process before constructing typed clients.
- [ ] Add `devfinity env` or equivalent only if shell workflows need to print the
      current run environment without starting a new stack.
- [ ] Treat existing external applications as consumers of generated endpoints
      and credentials, not as separate runtime backends.
- [ ] Add explicit profile options only when a workflow must start fewer local
      services or connect to an externally started dependency.

Exit criterion: shell commands, app tests, and Rust fixtures all consume the same
generated env contract, while typed Rust clients remain a convenience layer over
that contract.

## Phase 7: Add Identity And Auth

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

## Phase 8: Add Brain, Search, And Remaining Runtime Pieces

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

## Phase 9: App-Facing Development Backend

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
