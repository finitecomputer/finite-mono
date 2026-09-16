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
`effective=false`. Persisted Hermes `auxiliary.vision` rows are left alone.

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

## Service lifecycle

Service lifecycle uses one policy: an unavailable bridge does not terminate
agentd or restart Hermes. The command stream reconnects with bounded backoff,
including during initial startup. `FINITE_AGENTD_BRIDGE_READY_TIMEOUT_SECS` is
obsolete and ignored; there is no feature-dependent startup deadline. The
existing health/contact server still requires live service and bridge evidence
before reporting chat ready. A running process alone is not chat readiness.

The supervisor drains children before shutdown returns, accepts stop during
restart backoff, and sweeps a departed wrapper's process group before automatic
respawn. This prevents surviving grandchildren from holding service ports.
Lifecycle acceptance uses actual daemon/child processes and loopback HTTP;
the service fixtures do not establish a real Hermes model-turn proof.

**Agent rollout required:** these lifecycle changes live in the runtime image.
Bundle them with the Iroh runtime integration instead of rolling out this
preparatory change alone. No fleet configuration is enabled by this code.

## Core endpoint registration

`CoreEndpointRegistration` is the outbound HTTPS client for the runtime networking
process. Core provisions `FINITE_CORE_URL` and `FINITE_CORE_CREDENTIAL` through
the launch environment. The endpoint owner constructs one registration object
for its ephemeral Iroh endpoint and reuses that object for retries. Construction
allocates a monotonically increasing generation in the existing agentd ledger;
only the credential hash and counter are persisted, not an Iroh private key.
Reopening the ledger for a new process advances the generation, so delayed
requests from the prior process cannot replace its endpoint. `Superseded` means
the prior process must stop advertising; `Unauthorized` grants no access and
can occur before launch binding or after revocation. Registration is not peer
admission and does not itself permit traffic to Hermes.

The client is tested against the actual Core TCP router and Postgres alongside
the Runner provisioning client. When `FINITE_IROH_RELAY_URL` is configured,
`serve` supervises `finite-agentd iroh`
and native `hermes serve --host 127.0.0.1 --port 8642 --no-open` alongside the
existing services. Invalid Iroh configuration fails only that child. The Iroh
child requires the Core URL/credential from provisioning and uses the existing
ledger for its process generation; it has no Finite Chat dependency.

Core enablement is default-off per runtime creation. Admin writes include the
expected endpoint to reject stale dashboard actions, but a normal process
restart does not reset that policy. Each browser admission is separately bound
to the exact generation, endpoint and Hermes service: it lasts five minutes and
must be renewed every minute. The runtime polls every five seconds and closes
removed connections; during Core outage its monotonic local deadlines still
expire. Temporary authentication failures clear access and retry the same
endpoint; superseded endpoints close and stay closed until process shutdown.
Only CONNECT to `127.0.0.1:8642` is allowed, capped at 256 live Iroh connections.

The Core credential also follows the runtime creation. Ordinary restart and
in-place upgrade preserve it; stop suspends access and removes admissions until
a successful restart. Explicit revocation and destruction never revive it.
Replacement creations require a new credential and default-off enablement.

The canonical local ARM64 image passed real-chat and configured-Iroh restart
checks. Deployment remains blocked on production architecture/provider
qualification, existing-agent and replacement credential delivery, and the
hosted-access dashboard integration. The root pin matches the real-Hermes
acceptance revision.
Do not enable this across the fleet based only on local acceptance.
