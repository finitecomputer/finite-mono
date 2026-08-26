# Context Map

## Contexts

- [FiniteBrain](./finite-brain/CONTEXT.md) — encrypted, Folder-scoped
  knowledge spaces for humans and agents
- [Finite Identity](./finite-identity/CONTEXT.md) — public Principal
  resolution and identity lifecycle
- [Finite Nostr](./finite-nostr/CONTEXT.md) — reusable Nostr primitives
- [Finite Sites](./finite-sites/CONTEXT.md) — Sites publishing and hosting
- [Finite Skills](./finite-skills/CONTEXT.md) — managed Agent behavior and
  skill delivery
- [Finite Chat](./finitechat/CONTEXT.md) — chat, Hosted Device, and
  conversation surfaces
- [Finite Computer](./finitecomputer-v2/CONTEXT.md) — accounts, agents,
  runtimes, and dashboard orchestration
- [Finite Deployment](./infra/CONTEXT.md) — production delivery, artifact
  promotion, and rollout language
- [Commercial Relationships](./docs/commercial-relationships/CONTEXT.md) —
  internal organizations, arrangements, offerings, contributions, and wants

## Relationships

- **FiniteBrain → Finite Identity**: resolves public User and Agent identities;
  Brain retains ownership of Membership, Brain Roles, Folder Access, and
  Folder Key Grants.
- **FiniteBrain → Finite Nostr**: consumes reusable signing, identity encoding,
  and gift-wrap primitives while keeping Brain-specific crypto policy local.
- **Finite Skills → FiniteBrain**: teaches Agents to operate FiniteBrain
  through its public CLI interface.
- **Finite Computer → Finite Identity / FiniteBrain**: supplies authenticated
  account-agent associations and navigation context, never Brain authority.
- **Commercial Relationships → Twenty / FiniteBrain / Finite Computer**: Twenty
  owns curated structured commercial records and manual subledger facts;
  FiniteBrain owns meeting, Wants, relationship, research, and proposal
  context; Finite Computer remains authoritative for product access, Projects,
  Hosting Tiers, and runtime state.
- **Finite Deployment → all product contexts**: records how product-owned
  artifacts become production state without taking ownership of product
  protocols, data, or runtime lifecycle authority.
