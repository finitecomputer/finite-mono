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
    fixtures.rs
    runtime/
      mod.rs
      process_compose.rs
    components/
      mod.rs
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

- `lib.rs`: public harness API.
- `main.rs`: thin CLI over the library.
- `topology.rs`: profiles, ports, paths, environment, and state layout.
- `runtime/`: process-compose config generation, startup, readiness, status,
  cleanup, and wrapped-command execution.
- `components/`: typed service definitions and runtime wiring.
- `clients/`: typed or semi-typed harness clients used by tests.
- `fixtures.rs`: `run_devfinity_test`-style helpers for integration tests.

Process-compose should remain the concrete runtime because it gives us the TUI,
logs, health visualization, and lifecycle behavior we want. Keep it behind a
small `runtime::process_compose` module boundary, but do not introduce a
multi-backend abstraction until another runtime actually exists.

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
to become a typed harness.

- [ ] Keep `main.rs` as a thin CLI wrapper.
- [ ] Move the current stack construction API into stable library types.
- [ ] Add `DevfinityStack` as the main public stack object.
- [ ] Add `StackProfile` with an initial Core plus Chat plus Sites base profile.
- [ ] Add typed `StackPaths`, `StackPorts`, and `StackEnv` structs.
- [ ] Move process-compose YAML generation and lifecycle helpers behind a
      concrete `runtime::process_compose` module.
- [ ] Avoid a `RuntimeBackend` trait until a second backend or test double forces
      that abstraction.
- [ ] Keep existing `just dev up`, `just dev status`, `just dev cleanup`,
      `just dev smoke`, and `just dev rust-smoke` behavior working.
- [ ] Keep native Nix Postgres as infrastructure owned by the base profile.
- [ ] Add tests that prove the CLI delegates through the library layer rather
      than owning stack construction itself.

Exit criterion: the current harness still behaves the same, but the core types
make it clear where profiles, components, the process-compose runtime module,
and fixtures belong.

## Phase 2: Typed Base Components

Goal: make Core, Chat, and Sites first-class typed components instead of ad hoc
YAML sections.

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
- [ ] Convert the current smoke flow to use the Core, Chat, and Sites clients where
      practical.

Exit criterion: Core, Chat, and Sites can be started, inspected, and tested
through typed devfinity components and clients, while process-compose remains
only the concrete runtime implementation.

## Phase 3: Rust Fixture API

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

## Phase 4: Base Architecture Checkpoint

Goal: stop and validate the harness shape before adding more architecture
components.

- [ ] Review whether `DevfinityStack`, profiles, components,
      `runtime::process_compose`, clients, and fixtures have clear ownership
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

## Phase 5: Devimint-Style Environment Contract

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

## Phase 6: Add Identity And Auth

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

## Phase 7: Add Brain, Search, And Remaining Runtime Pieces

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

## Phase 8: App-Facing Development Backend

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
