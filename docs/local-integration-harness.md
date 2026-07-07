# Local Integration Harness

`devfinity` is the Finite monorepo local integration harness. It follows the
Fedimint `devimint` pattern at a smaller scale: one Finite-aware command
prepares deterministic local state, writes environment exports, starts local
services, exposes test context, and tears the stack down.

The current implementation uses Rust-owned orchestration, modeled after
`devimint`: devfinity owns child processes, logs, readiness, generated
environment, and teardown. A future mprocs-style local UI should only tail logs
and provide an ergonomic shell; it should not own lifecycle.

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
cargo run -p devfinity -- up --headless -- cargo test -p devfinity --locked --test stack_smoke -- --ignored --nocapture
cargo run -p devfinity -- status
cargo run -p devfinity -- cleanup
```

When a command is passed after `--`, `devfinity up` starts the stack in
headless mode, waits for the configured services to become ready, runs the
command with the generated devfinity environment variables, and tears the stack
down afterward. This is the automation path for local integration tests.

`just dev smoke` is intentionally more than a health check. It verifies service
readiness, then submits the dashboard create-agent form, lets the dashboard call
Core, and confirms through Core's `/api/core/v1/me` response that the agent
creation request and project were persisted in Postgres for the dev account.

`just dev rust-smoke` demonstrates the same model from Rust. The test is marked
`#[ignore]` so regular workspace test runs do not start or require devfinity;
the `just` recipe runs it through the wrapped-command path so it receives the
same generated environment variables as any other devfinity integration test.

`status` is read-only. It prints the generated state paths, devfinity-owned PID
control files, and short TCP/HTTP checks for the configured services.

`cleanup` is a recovery command, not the normal shutdown path. It terminates
processes recorded in the active run's devfinity-owned PID files and removes
stale control files. Normal wrapped-command runs tear the stack down
automatically through Rust process handles.

## Library API

The primary Rust entrypoint is `devfinity::DevfinityStack`. The base profile is
represented by `StackProfile::Base`, which starts Core, Chat, Sites, Dashboard,
and native Postgres through Rust-owned process handles.

The typed topology surface is:

- `StackPaths`: generated state, logs, control, Postgres, service, and
  `FINITE_HOME` paths.
- `StackPorts`: deterministic local ports for Core, Dashboard, Postgres, Chat,
  and Sites.
- `StackEnv`: the canonical generated environment values used by env files and
  wrapped commands.

`process.rs` owns child process spawning, log files, PID control files, and
shutdown behavior. `stack.rs` owns startup order, readiness polling,
wrapped-command execution, status, and cleanup. Typed components and fixture
APIs are the next architectural layers.

## Initial Stack

The first `devfinity` stack is intentionally narrow:

- Native local Postgres for `finite-saas-core`, using the Postgres binaries from
  the Nix development shell.
- `finite-saas-core`.
- Dashboard dev server with development auth enabled.
- Local `finitechat-server` backed by SQLite.
- Local `finitesitesd` with app execution disabled.

The richer create-agent canary still lives in
`finitecomputer-v2/scripts/local_create_agent_canary.sh`. That flow can become
a later `devfinity` profile after the base stack is stable.

## State

Generated state lives under:

```text
.local-state/devfinity/runs/default/
```

Important generated files:

- `env`: shell exports for local CLI tools.
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
- Node/npm, for the dashboard dev server.
- `curl`, for the smoke script.

The harness writes state files before starting services, so
`cargo run -p devfinity -- up --dry-run` is useful for checking generated state
and command prerequisites without launching the stack.
