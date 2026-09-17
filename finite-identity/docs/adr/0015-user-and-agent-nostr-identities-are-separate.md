# User and agent Nostr identities are separate

Each Agent Runtime owns a distinct Nostr key under its Finite Home and reuses
it across `finitechat`, `fsite` and `fbrain`. The human Chat identity is separate;
Account Auth independently gates SaaS access. Agents authenticate as their own
Principal and require explicit product grants to access another user's data.
