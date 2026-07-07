# Local Integration Harness

`devfinity` is the Finite monorepo local integration harness. It follows the
Fedimint `devimint` pattern at a smaller scale: one Finite-aware command
prepares deterministic local state, writes environment exports, starts local
services, exposes the stack handle to tests, and tears the stack down.

The current implementation uses Rust-owned orchestration, modeled after
`devimint`: devfinity owns child processes, logs, readiness, generated
environment, and teardown for backend infrastructure. Dashboard and other
UI/dev-server processes should move to an outer dev composer that consumes the
devfinity env/readiness contract.

By default, `devfinity up` starts the stack, waits for readiness, and keeps the
parent process alive until Ctrl-C. Use `--headless` for automation or
non-interactive terminals; today it runs the same Rust orchestration path
without an interactive log viewer.

## Commands

From the monorepo root:

```sh
just dev up
just dev up --headless
just dev up --headless -- scripts/devfinity-smoke
just dev smoke
just dev rust-smoke
just dev status
just dev cleanup
```

The equivalent direct commands are:

```sh
cargo run -p devfinity -- up
cargo run -p devfinity -- up --headless
cargo run -p devfinity -- up --headless -- scripts/devfinity-smoke
cargo test -p devfinity --locked --test stack_smoke -- --ignored --nocapture
cargo run -p devfinity -- status
cargo run -p devfinity -- cleanup
```

When a command is passed after `--`, `devfinity up` starts the stack in
headless mode, waits for the configured services to become ready, runs the
command with the generated devfinity environment variables, and tears the stack
down afterward. This is the automation path for local integration tests.

`just dev smoke` verifies backend readiness for Postgres, Core, Chat, and
Sites. Dashboard create-agent coverage belongs to the future outer e2e composer
because it includes a non-Rust UI/dev-server process.

`just dev rust-smoke` demonstrates the same model from Rust. The test is marked
`#[ignore]` so regular workspace test runs do not start or require devfinity.
The test calls `run_devfinity_test`, which starts a unique fixture stack,
applies generated environment variables to the test process, passes the
`DevfinityStack` handle to the closure, and tears the stack down afterward.

`status` is read-only. It prints the generated state paths, devfinity-owned PID
control files, and short TCP/HTTP checks for the configured services.

`cleanup` is a recovery command, not the normal shutdown path. It terminates
processes recorded in the active run's devfinity-owned PID files and removes
stale control files. Normal wrapped-command runs tear the stack down
automatically through Rust process handles.

## Library API

The primary Rust entrypoints are `devfinity::DevfinityStack` for direct stack
control and `devfinity::run_devfinity_test` for integration tests. The base
profile is represented by `StackProfile::Base`, which should own backend
infrastructure only: Core, Chat, Sites, and native Postgres.

The typed topology surface is:

- `StackPaths`: generated state, logs, control, Postgres, service, and
  `FINITE_HOME` paths.
- `StackPorts`: deterministic manual ports and allocated fixture ports for
  Core, Postgres, Chat, and Sites.
- `StackEnv`: the canonical generated environment values used by env files,
  wrapped commands, and fixture tests.
- `DevfinityVars`: generated paths, ports, env, ready/error markers, and shell
  exports for a run.
- `run_devfinity_test`: devimint-style fixture entrypoint that starts a stack,
  runs a closure with `&DevfinityStack`, and tears the stack down.

`process.rs` owns child process spawning, log files, PID control files, and
shutdown behavior. `stack.rs` owns startup order, readiness polling,
wrapped-command execution, status, and cleanup. Typed components and clients
are the next architectural layers.

The current base stack still contains one transitional violation: Rust services
are built from the workspace and launched as `target/debug/...` binaries. The
next harness phase audits and adds the missing server entrypoints, adds
in-process service supervision to devfinity, then migrates Core, Chat, and Sites
one at a time. `repo_root` and the startup `cargo build` preflight should
disappear only after no backend service depends on `target/debug/...`.
Dashboard is a Node app and belongs in an outer dev composer that consumes
devfinity env/readiness before starting UI commands.

## Initial Stack

The first `devfinity` stack is intentionally narrow:

- Native local Postgres for `finite-saas-core`, using the Postgres binaries from
  the Nix development shell.
- `finite-saas-core`, currently through its binary but targeted to move behind
  a library server entrypoint.
- Local `finitechat-server` backed by SQLite, targeted to move behind its
  library server entrypoint.
- Local `finitesitesd` with app execution disabled, targeted to move behind its
  library server entrypoint.

Dashboard dev server startup and the richer create-agent canary belong to the
outer dev/e2e composer, not to devfinity's backend fixture API.

## State

Generated state lives under:

```text
.local-state/devfinity/runs/default/
```

Fixture runs created by `run_devfinity_test` use unique run directories under
`$DEVFINITY_TEST_STATE_DIR` when set, or under the system temp directory by
default. Manual `devfinity up` keeps the deterministic `default` run.

Important generated files:

- `env`: shell exports for local CLI tools.
- `ready`: marker written after the stack becomes ready.
- `error`: marker written when startup or a fixture test fails.
- `control/*.pid`: devfinity-owned process control files for recovery.
- `run-postgres.sh`: generated native Postgres launcher.
- `urls.txt`: useful service URLs.
- `logs/`: preflight and service logs.
- `postgres/data/`: native Postgres data directory.

`postgres/data/` is reset before each non-dry-run stack start. This preserves
the old Docker-backed behavior where the Core database started clean for each
`devfinity up` run. `--dry-run` validates generated state and command
prerequisites without launching services and does not
delete the database directory.

## Prerequisites

Run from `nix develop` or otherwise provide:

- Rust/Cargo.
- Postgres client/server binaries from the Nix shell.
- `curl`, for the smoke script.

The harness writes state files before starting services, so
`cargo run -p devfinity -- up --dry-run` is useful for checking generated state
and command prerequisites without launching the stack.
