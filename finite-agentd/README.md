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
- `agent.hermes.restart`
- `agent.chat.recover`
- `agent.connections.status`
- `agent.inference.apply`
- `agent.specialization.aeon.reconcile`
- `agent.telegram.connect`, `agent.telegram.approve`, `agent.telegram.home`,
  and `agent.telegram.disconnect`
- `agent.google.apply` and `agent.google.disconnect`
- `agent.hermes.config.preview`
- `agent.hermes.config.apply`
- `agent.hermes.config.rollback`

Specialization reconciliation owns only the `auxiliary.vision` Hermes config
field. Its typed AEON desired state includes the worker endpoint, canonical
model alias, independently declared image/audio/video capabilities, prompt
versions, and normalization limits. Existing worker credentials are retained
unless a replacement credential is supplied through the encrypted command.
Finite-applied values carry a durable pre-image and ownership hash; validation
failure restores the exact previous bytes, and later user/Hermes drift blocks
automatic rollback. Remote commands fail closed unless the sending Finite Chat
Principal is in the durable authorization ledger.

Specialization reconciliation is deliberately a model-profile operation. It
does not register model-named tools, intercept attachments, or add behavioral
instructions to the main agent. Hermes keeps its normal tool catalog and the
main model decides when to use a native capability. The current AEON profile
backs Hermes's `vision_analyze` and `video_analyze` tools through
`auxiliary.vision`. The capability flags constrain requests accepted by the
worker; they do not create a missing Hermes tool surface. Semantic audio
interpretation therefore remains unavailable to the agent until Hermes has a
generic instruction-preserving audio-analysis capability. This profile-first
rule applies to every Finite specialization, not only AEON or vision.

An AEON image reconciliation becomes effective only after Hermes restarts and
its installed `vision_analyze_tool` returns exact semantic output for a fixed
image through `auxiliary.vision`. The packaged probe uses the same
`HERMES_HOME` as the resident process and emits only a bounded pass/fail result;
it does not expose the worker credential or provider response.
`FINITE_AGENTD_AUTHORIZED_ACCOUNT_IDS` seeds that ledger when configured. For
the trusted internal-canary path only, the first `agent.owner.claim` may fill
an empty ledger; later claims and every other unauthorized command fail
closed. This is not the broader customer-admission authority that ADR 0003
still requires.

Durable ledger reopening, pending-command resume, and terminal-result replay
are covered locally. The remaining production evidence gaps are a live
lat1-plus-Kata composition gate, real child-death/signal/orphan coverage for
the supervisor, and off-host restore of the same Agent Device, ledger, and
retained data onto an empty target. Local Hermes CI runs the encrypted bridge
flow, but its wrapper can still synthesize the passing report artifact when the
richer in-test report hook is absent; that report is not independent
live-runtime evidence.
