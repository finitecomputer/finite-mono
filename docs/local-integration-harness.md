# Local Integration Harness

`devfinity` is the Finite monorepo local integration harness. It follows the
Fedimint `devimint` pattern at a smaller scale: one Finite-aware command
prepares deterministic local state, writes environment exports, generates the
process supervisor config, and leaves process lifecycle and visualization to a
dedicated runtime.

The runtime is `process-compose`. By default, `devfinity up` starts the
process-compose TUI so the running services, health state, and logs are visible.
Quitting the TUI or pressing Ctrl-C should shut down the stack. Use
`--headless` for automation or non-interactive terminals.

## Commands

From the monorepo root:

```sh
just dev up
just dev up --headless
just dev status
just dev cleanup
```

The equivalent direct commands are:

```sh
cargo run -p devfinity -- up
cargo run -p devfinity -- up --headless
cargo run -p devfinity -- status
cargo run -p devfinity -- cleanup
```

`status` is read-only. It prints the generated state paths, process-compose
socket state, devfinity pid-file process states, the local Postgres container
state, and short TCP/HTTP checks for the configured services.

`cleanup` is a recovery command, not the normal shutdown path. It asks
process-compose to stop the generated stack if the local Unix socket is still
present, then checks devfinity pid files for any remaining managed process
trees, removes devfinity-labeled Docker containers, removes Docker containers
publishing devfinity's Postgres port, and removes stale control files. It
intentionally avoids broad host process killing.

## Initial Stack

The first `devfinity` stack is intentionally narrow:

- Local Postgres for `finite-saas-core`.
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
- `process-compose.yaml`: generated process-compose configuration.
- `process-compose.sock`: local Unix socket for process-compose control.
- `urls.txt`: useful service URLs.
- `logs/`: process-compose and service logs.

The control API uses a Unix socket by default to avoid the default
process-compose TCP port and keep the control plane local to the machine.

## Prerequisites

Run from `nix develop` or otherwise provide:

- Rust/Cargo.
- `process-compose`.
- Docker, for local Postgres.
- Node/npm, for the dashboard dev server.

The harness generates the process-compose config before starting services, so
`cargo run -p devfinity -- up --dry-run` is useful for checking config shape
without launching the stack.
