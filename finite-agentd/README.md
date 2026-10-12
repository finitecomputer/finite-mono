# finite-agentd

`finite-agentd` is the narrow, runtime-resident Finite platform daemon owned by
one Agent Principal. It survives Hermes restarts, receives typed encrypted
runtime commands through Finite Chat, publishes command results and observed
state, and applies allowlisted agent-local changes with durable rollback.

In the production Kata layout, each Agent Runtime has its own `/data`. The
Agent's Finite Chat Device store and `finite-agentd`'s durable
`/data/agent/agentd/agentd.sqlite3` authorization/command ledger therefore do
not share storage with another Agent Runtime. `finite-agentd` independently
supervises the resident Finite Chat sidecar, health service, and Hermes with
null stdin. Hermes must be healthy to produce a new model reply; retained Chat
state and typed management commands such as `agent.owner.claim` are not Hermes
interactivity contracts.

This is distinct from the web user's Hosted Device on lat1. One
`finitechat-hosted-device` service hosts many verified WorkOS users in a
runtime map, with a separate identity and encrypted SQLite store for each
user. It is not one `finite-agentd` or one process per web user.

It is not Core, Runner, RMP, `fsite`, `fbrain`, or the Finite Chat server. It
never accepts arbitrary shell, argv, filesystem paths, YAML, or environment
edits from the platform.

The architectural decision and first-slice acceptance criteria are in
[`docs/adr/0003-agentd-is-the-agent-owned-platform-boundary.md`](../docs/adr/0003-agentd-is-the-agent-owned-platform-boundary.md).

The current daemon accepts these versioned command families over the Agent
Platform Channel:

- `agent.status.inspect`
- `agent.owner.claim`
- `agent.connections.status`
- `agent.inference.apply` (v1), `agent.inference.select`, and
  `agent.inference.disconnect`
- `agent.openrouter.usage`, `agent.openrouter.connect`,
  `agent.codex.login.start`, `agent.codex.login.cancel`, and
  `agent.codex.models` (dispatched and admitted; they answer
  `unsupported_command` until the provider handlers are implemented)
- `agent.telegram.connect`, `agent.telegram.approve`, `agent.telegram.home`,
  and `agent.telegram.disconnect`
- `agent.google.apply` and `agent.google.disconnect`

AEON specialization is removed, not just retired. Runner no longer injects
`FINITE_SPECIALIZATION_BUNDLE` / `FINITE_SPECIALIZATION_WORKER_API_KEY`, and
`finite-agentd` carries no specialization writer machinery. Leftover copies of
those variables in container environment are ignored: the daemon does not
activate or probe `auxiliary.vision`, and the retired
`agent.specialization.aeon.reconcile` command falls through to the generic
unsupported-command error. Status still includes a `specialization` object so
mixed-version readers keep working; it is always `desired=false` /
`effective=false`. Agentd leaves persisted Hermes `auxiliary.vision` rows
alone. The Agent Runtime startup reconciler
(`finitechat/containers/agent/reconcile_hermes_config.py`) removes only the
exact AEON worker block at gateway start. It keeps the replaced config as
`hermes-home/config.yaml.pre-aeon-vision-retirement`. Any agentd rollback of
an old AEON proposal would then fail its config hash check, as intended.

`FINITE_AGENTD_AUTHORIZED_ACCOUNT_IDS` seeds that ledger when configured. For
the trusted internal-canary path only, the first `agent.owner.claim` may fill
an empty ledger; later claims and every other unauthorized command fail
closed. This is not the broader customer-admission authority that ADR 0003
still requires.

Durable ledger reopening, pending-command resume, and terminal-result replay
are covered locally, as are the supervisor's child signal-drain and post-exit
orphan sweep (the runtime image has no `kill` binary; signalling is in-process
via rustix and each supervised child leads its own process group). The
remaining production evidence gaps are a live lat1-plus-Kata composition gate
and off-host restore of the same Agent Device, ledger, and retained data onto
an empty target. Local Hermes CI runs the encrypted bridge flow, but its
wrapper can still synthesize the passing report artifact when the richer
in-test report hook is absent; that report is not independent live-runtime
evidence.

## Inference connections

The [runtime control contract](../finitecomputer-v2/docs/runtime-control-contract.md#inference-connections)
owns command semantics, status facts, compatibility, and recovery guarantees.
The inference handlers live in `src/inference_commands.rs`; the background
executor lives in `src/executor.rs`.

Test-only environment, never set in production:
`FINITE_AGENTD_OPENROUTER_API_BASE` (`https://…` or
`http://127.0.0.1:<port>` only), and `FINITE_AGENTD_INFERENCE_HELPER_PYTHON` /
`FINITE_AGENTD_INFERENCE_HELPER_MODULE`. agentd removes the helper's own test
variables (`FINITE_CODEX_AUTH_ISSUER`, `FINITE_CODEX_LOGIN_DEADLINE_S`,
`FINITE_HELPER_TEST_BARRIER`, `FINITE_HELPER_TEST_BARRIER_FILE`) from the
helper's environment.

## E-0 host harness

`examples/inference_host_harness.rs` runs real `finite-agentd serve` in
`.local-state/e0/`, with loopback bridge, HWD, OpenRouter and Core fakes. The
stub gateway runs the real reconciler and packaged disconnect helper, then
sleeps. `smoke` tests recovery and process ordering; it does not run chat turns
or contact real gateways/providers. `serve` exposes the same runtime to the
real dashboard through `FC_DESIGN_RUNTIME_COMMANDS_URL`.

Run it from the repository root, inside the Nix dev shell (the launcher step
needs coreutils `timeout`). It needs the patched Hermes environment in
`.local-state/hermes-env`; build it there once (the out-link keeps it from
garbage collection):

```sh
nix build --out-link .local-state/hermes-env \
  ".#packages.$(nix eval --impure --raw --expr builtins.currentSystem).hermes-agent-python"
scripts/with-dev-env bash -c 'cargo build -p finite-agentd --bins --examples && \
  target/debug/examples/inference_host_harness smoke'
```

`smoke` runs the E-0 proofs (P1–P10) and exits non-zero if one fails. `serve
--port <port>` keeps the agent up for the dashboard:
`FC_DESIGN_RUNTIME_COMMANDS_URL=http://127.0.0.1:<port> just dev web-design`.
Ctrl-C stops agentd and everything it started. `--hermes-env` and `--agentd`
override the patched env (`.local-state/hermes-env`) and the agentd binary.

## Optional hosted Hermes process

`finite-agentd hosted-hermes` validates trusted launch settings and replaces
itself with the image's native `hermes serve --isolated --no-open` process.
It requires these environment variables; no credential value is accepted in
argv or emitted by agentd's configuration errors:

| Variable | Contract |
| --- | --- |
| `FINITE_AGENTD_HOSTED_HERMES_ENABLED` | `1` or `true` enables the child; absent, empty, `0` or `false` leaves it off. |
| `FINITE_AGENTD_HOSTED_HERMES_BIND_ADDR` | Explicit IP address and nonzero port. The container publication boundary owns reachability. |
| `HERMES_HOME` | The existing absolute durable Hermes home; no copied or shadow home. |
| `HERMES_DASHBOARD_PUBLIC_URL` | HTTPS URL with a non-loopback hostname, including the external runtime prefix. Required even when the backend binds loopback so native auth remains engaged. |
| `HERMES_DASHBOARD_BASIC_AUTH_USERNAME` | Native basic provider username. Hermes itself logs this identifier. |
| `HERMES_DASHBOARD_BASIC_AUTH_PASSWORD` | Native password, supplied through the trusted secret environment. |
| `HERMES_DASHBOARD_BASIC_AUTH_SECRET` | Stable native signing secret, supplied through the trusted secret environment. |

When enabled by Core desired state, `finite-agentd serve` runs that command in a dedicated optional
supervisor task. Its failure/restart does not restart the gateway or SimpleX.
Daemon shutdown awaits this child's process-group termination. The existing
chat/health/gateway/SimpleX supervisor behavior is unchanged. In particular,
the existing Finite Chat bridge readiness deadline still terminates agentd
after 180 seconds; this slice does not claim independence from that outage.

The native process reads the existing Hermes configuration. Agentd does not
rewrite `dashboard`, enable disabled plugins, or change gateway ownership.
At the pinned Hermes version, `--isolated` prevents named-profile startup
from rerouting to the machine-default dashboard; it does not create another
home or copy history. Native basic-auth environment values work without a
`dashboard.basic_auth` YAML section. An existing password hash can take precedence
over the supplied plaintext password; readiness fails rather than rewriting it.
An agent-owned plugin conflict must be reported, not repaired silently.

`HERMES_TUI_WS_ORPHAN_REAP_GRACE_S=0` keeps accepted work alive when a client
disconnects. Stopping the native process is a separate interruption boundary.
The username/password and signing secret must remain stable on ordinary
process restarts; the launcher never generates or rotates them.

This process capability is not a public-access grant. `/api/status` is public
in the pinned native API, so successful status retrieval alone cannot prove
authentication. Native password login and a protected read such as
`/api/auth/me` must also be qualified. `running` in agentd status describes
the child process, not native auth, Caddy readiness, or user authorization.

The Core assignment-pull path controls enablement and reports application after
native authentication or child exit. Without Core bootstrap configuration this
optional service stays off. Caddy publication lifetime and existing-agent
enrollment remain release gates. No new Chat management command or local credential store
is introduced. Ship this capability in the coordinated R1 Agent rollout,
default off; do not schedule a separate rollout for the launcher alone.
