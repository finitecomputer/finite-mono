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
- `agent.inference.apply`
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

## Container output observations

`finite-agentd serve` samples the stdout and stderr it inherited once a
second on a dedicated thread. In the Runtime image these are the container
output pipes that every supervised child writes to. `status.json`,
`agent.status.inspect` and the `runtime.agentd` snapshot carry the latest
sample as `container_output`. Once `status.json` parses, the health server
projects an allowlisted copy as `agentd.container_output` in `/healthz` and
`/contact`.

Each stream has a `kind`: `pipe`, `not_pipe`, `same_pipe_as_stdout`,
`unavailable`, `unsupported_platform` or `sampler_not_started`. A `pipe`
sample carries:

- `observed_at_ms`: wall-clock time after both reads returned. It is not the
  instant either value was captured; if the second read waited, the first
  value is older.
- `buffered_bytes` (`FIONREAD`).
- `capacity_bytes` (`F_GETPIPE_SZ`). This is a separate read, so a concurrent
  resize can leave `buffered_bytes` above it in one sample.
- `observed_decrease_bytes`: the saturating sum of drops in `buffered_bytes`
  between consecutive successful samples since `first_observed_at_ms`, over
  `sample_count` samples. It is only that sum.

What the samples do not prove:

- `observed_decrease_bytes` is not bytes read, received or delivered. A
  writer in packet mode (`O_DIRECT`, which `F_SETFL` can switch on at any
  time) makes each write one packet. A read shorter than a packet returns
  part of it, and the kernel discards the rest; the discarded bytes count as
  a drop too.
- An unchanged `observed_decrease_bytes` does not mean nothing was read.
  Output written and removed between two samples is invisible.
- A full pipe means output is waiting for its reader. It does not prove that
  a given writer is waiting (a small write can still merge into the last
  page), that the host collector is gone, or that Hermes is stuck.
- Nothing here measures gateway, channel or reply progress. None of it feeds
  readiness, supervision or restarts.

From kata-agent 3.29.0 source (not observed in a booted guest), kata-agent
reads the guest pipes only when the host shim asks for output. A draining
guest pipe would therefore show the local output path moving, not end-to-end
delivery. Runtimes on older images report nothing.

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
