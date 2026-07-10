# finite-agentd

`finite-agentd` is the narrow, runtime-resident Finite platform daemon owned by
one Agent Principal. It survives Hermes restarts, receives typed encrypted
runtime commands through Finite Chat, publishes command results and observed
state, and applies allowlisted agent-local changes with durable rollback.

It is not Core, Runner, RMP, `fsite`, `fbrain`, or the Finite Chat server. It
never accepts arbitrary shell, argv, filesystem paths, YAML, or environment
edits from the platform.

The architectural decision and first-slice acceptance criteria are in
[`docs/adr/0003-agentd-is-the-agent-owned-platform-boundary.md`](../docs/adr/0003-agentd-is-the-agent-owned-platform-boundary.md).

The first slice accepts only these versioned commands over the Agent Platform
Channel:

- `agent.status.inspect`
- `agent.hermes.restart`
- `agent.chat.recover`
- `agent.hermes.config.preview`
- `agent.hermes.config.apply`
- `agent.hermes.config.rollback`

Only the `auxiliary.vision` Hermes config field is allowlisted initially.
Finite-applied values carry a durable pre-image and ownership hash; validation
failure restores the exact previous bytes, and later user/Hermes drift blocks
automatic rollback. Remote commands fail closed unless
`FINITE_AGENTD_AUTHORIZED_ACCOUNT_IDS` names an authorized Finite Chat
Principal.
