# Finite Computer

Core owns accounts, Projects, entitlements and Agent Runtime lifecycle. Product
services own their data, permissions and protocols.

- **Account Auth**: the verified dashboard account session. It is distinct from
  a Nostr Principal and never supplies a user's private key to an Agent.
- **Project**: Core's ownership and placement boundary for an Agent Runtime;
  it is distinct from a Sites Project Repository.
- **Agent Runtime**: the agent's compute environment and durable state.
  **Agent Principal Key** is its independent Nostr identity, shared by the
  Finite tools in that Runtime's **Finite Home**.
- **Runner**: the provider adapter that executes Core-authorized lifecycle
  operations. A provider handle identifies compute, not product ownership.
- **Desired Runtime State / Runtime Operation**: Core's persisted lifecycle
  intent and the explicit operation that reconciles it. Restart does not
  silently choose a new image; Runtime Upgrade names an immutable artifact.
- **Runtime Management Pipe**: outbound health and release telemetry. It is
  not a feature API, credential channel or remote shell.
- **Finite Agent Daemon**: `finite-agentd`, the runtime-local supervisor and
  typed agent action boundary. Compute lifecycle remains Core-to-Runner.
- **Canonical Agent Room**: a navigation binding to existing Chat state. It
  grants no authority to choose or rewrite ambiguous durable conversations.
- **Recovery Set**: the data and keys required to restore the declared service
  or Runtime state. A **Recovery Snapshot** is its integrity-checked copy;
  a provider volume is live storage, not a backup.
- **Recovery Authority**: a Principal or protected capability able to unlock
  recovery material. Recovery is proven by restoration onto an empty target.
- **Runtime Retirement / Purge User Data**: retirement removes compute while
  retaining recovery material; purge is a separate, explicitly authorized,
  retention-gated irreversible operation.

Account enrollment, Agent admission, launch, identity readiness and chat
readiness are separate checks. Runner capacity and drain affect availability.
See [runtime control](docs/runtime-control-contract.md) and
[recovery policy](../docs/adr/0001-recoverability-precedes-operator-blindness.md).
