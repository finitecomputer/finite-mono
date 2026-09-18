# Operational runbooks

Runbooks describe supported operations, verification and recovery. Plans and
unverified follow-ups live in [Linear](https://linear.app/finitecomputer).
Executable configuration defines desired state; `scripts/finite-status` and
fresh read-only evidence establish observed state.

| Operation | Runbook |
| --- | --- |
| App-plane deployment | [Core and dashboard](deploy-core.md) |
| Cross-component deployment order | [Platform rollout](platform-rollout.md) |
| Chat deployment and single-writer constraints | [Chat server](deploy-finitechat-server.md) |
| Brain deployment | [Brain](deploy-brain.md) |
| Sites deployment, backup and redirects | [Sites](deploy-sites.md) |
| Identity Directory | [Identity](identity-authority.md) |
| CLI releases and aliases | [CLI release](release-cli.md) |
| Runtime image build, promotion and existing-Agent upgrade | [Runtime image](runtime-image.md) |
| Stopped Runtime relocation | [Cold relocation](runtime-cold-relocation.md) |
| Legacy Hermes import tool | [Legacy import](legacy-hermes-box1-to-lat3.md) |
| Phala operations | [Phala](phala-confidential-runner.md) |
| Host installation from CI artifacts | [Host installation](install-host.md) |
| New-agent qualification | [Targeted canary](targeted-agent-canary.md) |
| Runtime inference route repair | [Runner route](runner-finite-private-route.md) |
| Coordinated service recovery | [Hosted recovery](hosted-web-chat-recovery.md) |
| Postgres restore | [Postgres](postgres-backup-restore.md) |
| Continuous Chat/Brain replicas | [Litestream](litestream-chat-replication.md) |
| Missing conversations | [Chat diagnosis](chats-appear-missing.md) |
| Billing support | [Stripe](stripe-billing.md) |
| Host access | [Break-glass](break-glass.md) |

## Standing boundaries

- Run `scripts/finite-status` before and after rollouts. Its exit codes are
  0 healthy, 1 unhealthy, and 2 unknown without a known failure. Heartbeats
  establish Core-recorded state, not proof of live provider compute. Add
  missing incident probes there instead of inventing parallel status commands.
- Every promotion changes one authoritative source: component tag/rolling
  alias, Core Runtime artifact plus host pin, or NixOS closure. Existing Agents
  retain their launch artifact until explicitly upgraded.
- Build and smoke the exact immutable image before promotion. An optional
  source rebuild is not proof of the bytes being promoted.
- Deployment does not authorize data repair. Ambiguous identity, ownership or
  durable state fails closed. Preserve source state and backup/rollback
  boundaries before mutation.
- Never open snapshot SQLite in place; use `scripts/snapshot-sqlite` or a
  private scratch copy. No secret values or customer content in git or logs.
- A Runtime lifecycle probe gates upgrade eligibility separately from app
  health. Never clear its failure by blindly deleting compute or MLS state.
