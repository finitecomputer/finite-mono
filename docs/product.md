# finite.computer

Status: PROPOSED

Finite.computer is a personal, hosted agent people can name, launch, and work with from a calm dashboard and a great chat. It is for people who want frontier AI to help with real work without having to operate a server, understand a model stack, or use a developer tool.

The product starts with an account, an agent, and one familiar place to talk to it. People can connect the services they choose, work with files and images, publish what their agent makes, and return after a supported restart without losing their agent or their place in the conversation. The existing Finite Chat experience is the quality bar; Electron later joins as another Device, not another product.

During the internal-canary and white-glove-training phase, the public landing page offers normal sign-in plus a clear path for people who already have a Launch Code; everyone else may request access. Open paid/self-serve launch is a later customer-run decision, not something the canary landing page implies.

## UX invariants

- The product speaks plainly. User-facing surfaces do not make people learn our infrastructure, protocols, or internal service names to accomplish ordinary work.
- Chat is the north star: responsive, legible, attachment-capable, recoverable, and continuous across supported restarts.
- Make the agent easier to understand and steer instead of teaching people to fear or manage Hermes as infrastructure. Honest turn cancellation may still be useful, but a fake process-level stop is not a substitute for better interaction.
- The dashboard has one clear personal home: Agent, Connections, and Chat. Brain joins that navigation only when its Principal, signer, and Folder Key path works for the current client; hidden unfinished entry points are not product capability.
- Connections, Sites, Brain, and skills stay composable product capabilities. They do not turn Runner, Runtime Management, or the runtime image into a feature control plane.
- The agent's identity and durable work belong to the agent; a person's account opens their own dashboard and Devices. We never present a privacy or recovery promise stronger than the evidence.
- User data must not become locked or inaccessible. The first trusted cohort can use honest, Finite-assisted recovery while stronger user-controlled recovery and operator privacy are proven.

This page deliberately links to, rather than replaces, the working product and architecture contracts: [SaaS v1 PRD](../finitecomputer-v2/docs/vertical-slice-v1-prd.md), [ADR 0001](adr/0001-recoverability-precedes-operator-blindness.md), [ADR 0002](adr/0002-managed-skills-are-hot-swappable-product-revisions.md), [ADR 0003](adr/0003-agentd-is-the-agent-owned-platform-boundary.md), and the [architecture overview](architecture-overview.md).
