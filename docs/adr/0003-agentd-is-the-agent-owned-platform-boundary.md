# ADR 0003: `finite-agentd` is the agent-owned platform boundary

Status: accepted for the first implementation slice, 2026-07-09.

## Context

Finite needs to offer improvements and product actions to a running agent
without reaching into a Runner, editing a remote `.env`, or continuously
forcing a centrally stored copy of Hermes configuration over changes made by
Hermes or the user. Legacy `finitec` proved the value of one stable agent-side
surface, but it also accumulated host, Kubernetes, publishing, repository,
chat, and configuration responsibilities in one broad binary.

Finite Chat already defines encrypted, targeted runtime command requests,
results, cancellation, latest-state snapshots, and an idempotent command
ledger. Finite Sites, Finite Brain, and Finite Chat remain independently usable
products with their own CLIs and protocols.

## Decision

Add one runtime-resident Rust daemon named `finite-agentd`.

`finite-agentd`:

- owns the Agent Principal's outbound platform command/status connection;
- is supervised independently from Hermes and remains available while Hermes
  is broken or restarting;
- consumes only typed, allowlisted Finite Chat runtime commands;
- publishes typed command results and encrypted latest-state snapshots;
- keeps a durable local idempotency, configuration-ownership, and rollback
  ledger under the Agent Home;
- supervises agent-local processes such as the Finite Chat bridge and Hermes;
- delegates Sites, Brain, and Chat behavior to `fsite`, `fbrain`, and
  `finitechat` instead of absorbing their product protocols; and
- coexists with the small local `finite` command surface for explicit
  agent-owned workflows such as `finite skills sync`.

Dashboard and hosted services send intent, not YAML, environment edits,
filesystem paths, shell, or command argv. The daemon materializes intent with
the Runtime image's trusted handler implementation.

The first command family is deliberately narrow:

- inspect redacted agent/Hermes status;
- restart the Hermes process;
- recover incomplete Finite Chat/Hermes turns;
- preview, apply, and roll back an allowlisted Hermes configuration offer;
- select either the Finite Private or OpenRouter inference profile;
- connect, approve pairing for, select a home chat for, and disconnect
  Telegram through Hermes' supported configuration and pairing flows; and
- install or revoke the exact product-scoped Google Workspace grant used by
  the bundled Finite Skill.

Inference, Telegram, and Google remain separate typed schemas even when they
share the same encrypted transport and durable ledger. The browser cannot name
an arbitrary command, path, YAML field, environment variable, or executable.

## Configuration ownership

Every Finite-applied field records the pre-image and applied value. An offer
may apply when the field is unset/`auto`, or when it still matches the last
Finite-applied value. A user or Hermes edit that differs from that value is a
conflict and is never overwritten automatically. Rollback is allowed only when
the current value still matches the recorded Finite-applied value.

There is no central desired Hermes configuration and no continuous
reconciliation loop. Offers are explicit and agents adopt them at their own
pace.

## Boundary with lifecycle infrastructure

`finite-agentd` may restart Hermes and other processes inside an Agent Runtime.
It does not replace, stop, restart, or destroy compute. Agent Runtime lifecycle
continues to flow from Core through the provider-neutral Runner contract.

The Runtime Management Pipe remains outbound-only generic release and health
telemetry. Product commands and product-specific state use Finite Chat and do
not widen RMP.

## Recovery and security

- Mutating commands require a durable authorized Principal and fail closed for
  every other Principal. In the trusted-user first slice, an unclaimed agent
  lets the first Principal already admitted to its Finite Chat room perform one
  `agent.owner.claim`; the daemon persists that Principal before accepting any
  other mutation. This is an honest bootstrap expedient, not a public-SaaS
  proof that account auth and Agent Principal authorization are cryptographically
  bound. Before untrusted room admission, replace it with a Core-bound,
  one-time claim or equivalent attested enrollment without changing the
  command schemas.
- Authorization is expressed as Finite Chat Principal account IDs, not Account
  Auth identity, bearer tokens, or a Runner capability. Account Auth may gate
  the dashboard Device, but the Device's cryptographic Principal signs the
  agent operation.
- Secrets are accepted only inside the encrypted typed command body when a
  handler requires one; they never appear in URLs, argv, status, errors, or
  command results.
- The command ledger is durable and replay-safe across daemon, Hermes, Chat,
  and Runtime restarts.
- Runtime command requests, results, and latest-state snapshots are durable
  Finite Chat application events but are deliberately excluded from the human
  chat transcript.
- A Chat server or Hosted Device outage is retried with bounded backoff. It
  does not terminate the Agent Runtime; result replay plus durable delivery
  heals an acknowledgement lost after the result was sent.
- Configuration writes are atomic and validated before Hermes is restarted.
- Failed validation restores the exact previous bytes.
- Export remains an explicit future recovery workflow; this daemon boundary
  does not itself claim a Recovery Snapshot implementation.

## Rejected shapes

- Core or a fleet worker continuously rewriting runtime Hermes YAML.
- Dashboard-to-runtime shell, filesystem, Kubernetes, or provider access.
- Product feature commands on the Runtime Management Pipe.
- Reintroducing legacy `finitec` publishing, repository, gateway, or broad
  arbitrary command behavior.
- Folding Finite Sites, Finite Brain, or Finite Chat into this daemon.

## First-slice evaluation

The implementation is acceptable when tests prove:

- duplicate command delivery executes once and replays the recorded result;
- conflicting request-id reuse fails closed;
- unauthorized and unknown commands do not mutate local state;
- a Finite-owned configuration offer applies atomically and can roll back;
- user/Hermes drift is detected and not overwritten;
- failed Hermes validation restores the previous config;
- Hermes can be restarted without stopping the command transport; and
- an Apple Container replacement preserves Agent identity, daemon ledger,
  Hermes state, Chat state, Sites workspace, and existing attachments.
