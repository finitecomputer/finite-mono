# Context Map

## Contexts

- [FiniteBrain](./finite-brain/CONTEXT.md) — encrypted, Folder-scoped
  knowledge spaces for humans and agents
- [Finite Identity](./finite-identity/CONTEXT.md) — public Principal
  resolution and identity lifecycle
- [Finite Nostr](./finite-nostr/CONTEXT.md) — reusable Nostr primitives
- [Finite Search](./finite-search/CONTEXT.md) — reusable search primitives
- [Finite Sites](./finite-sites/CONTEXT.md) — Sites publishing and hosting
- [Finite Skills](./finite-skills/CONTEXT.md) — managed Agent behavior and
  skill delivery
- [Finite Chat](./finitechat/CONTEXT.md) — chat, Hosted Device, and
  conversation surfaces
- [Finite Computer](./finitecomputer-v2/CONTEXT.md) — accounts, agents,
  runtimes, and dashboard orchestration
- [Finite Deployment](./infra/CONTEXT.md) — production delivery, artifact
  promotion, and rollout language

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
- **Finite Deployment → all product contexts**: records how product-owned
  artifacts become production state without taking ownership of product
  protocols, data, or runtime lifecycle authority.
