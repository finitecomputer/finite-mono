# finite-nostr Context

## Glossary

### Reusable Nostr Primitive

A generic Nostr operation that can be reused across Finite repos without
depending on a product domain. Examples include NIP-19 identity encoding, event
serialization and verification, NIP-44 encryption adapters, NIP-59 gift-wrap
helpers, and NIP-98-style HTTP authorization helpers.

### Product Policy

Application-specific behavior owned by a consuming repo. FiniteBrain Vaults,
Folders, Folder Key Grants, sync, sharing, and OKF behavior are Product Policy
and must not be implemented inside `finite-nostr`.

### Protocol Wrapper

A small typed Rust API over lower-level Nostr protocol functionality. The
wrapper should make Finite call sites explicit and testable while preserving the
underlying Nostr semantics.

