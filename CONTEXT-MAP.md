# Context Map

## Contexts

- [FiniteBrain](./finite-brain/CONTEXT.md) — encrypted, Folder-scoped
  knowledge spaces for humans and agents
- [Finite Identity](./finite-identity/CONTEXT.md) — public Principal
  resolution and identity lifecycle
- [Finite Nostr](./finite-nostr/CONTEXT.md) — reusable Nostr primitives
- [Finite Search](./finite-search/CONTEXT.md) — reusable search primitives
- [Finite Drive](./finite-drive/CONTEXT.md) — shared blob existence,
  storage accounting, retention, and physical lifecycle
- [Finite Sites](./finite-sites/CONTEXT.md) — Sites publishing and hosting
- [Finite Skills](./finite-skills/CONTEXT.md) — managed Agent behavior and
  skill delivery
- [Finite Chat](./finitechat/CONTEXT.md) — chat, Hosted Device, and
  conversation surfaces
- [Finite Computer](./finitecomputer-v2/CONTEXT.md) — accounts, agents,
  runtimes, and dashboard orchestration

## Relationships

- **FiniteBrain → Finite Identity**: resolves public User and Agent identities;
  Brain retains ownership of Membership, Brain Roles, Folder Access, and
  Folder Key Grants.
- **FiniteBrain → Finite Nostr**: consumes reusable signing, identity encoding,
  and gift-wrap primitives while keeping Brain-specific crypto policy local.
- **Finite Skills → FiniteBrain**: teaches Agents to operate FiniteBrain
  through its public CLI interface and renders Brain's structured invitation
  and cohort results without inferring identity relationships.
- **Finite Chat → FiniteBrain**: carries authenticated human request context
  for the narrow sensitive Brain operations that remain human-authorized but
  agent-operated; conversational text asserted by an Agent is not authority.
- **Finite Chat / FiniteBrain / Finite Sites → Finite Drive**: products retain
  the meaning and authorization of their content while Drive owns the durable
  Blobs, Blob References, usage accounting, retention, and physical lifecycle.
- **Finite Computer → Finite Drive**: supplies durable User and Customer
  Organization identities for quota-bearing Storage Accounts; Drive remains
  authoritative for storage usage and enforcement.
- **Finite Computer → Finite Identity / FiniteBrain**: supplies authoritative
  account ownership, account-agent enumeration, and permanent agent-lifecycle
  facts. Brain turns those facts into durable product-scoped Account Access
  Cohorts and Personal Brain Agent Access; navigation context never grants
  authority and routine Brain work does not require Finite Computer to be live.
