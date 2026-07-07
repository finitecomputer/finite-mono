# ADR 0001: Use The rust-nostr Protocol Crate First

Status: accepted

Date: 2026-06-23

## Context

`finite-nostr` exists to provide reusable Nostr primitives for Finite Rust
repos. FiniteBrain needs NIP-19 identity handling, Nostr event construction and
validation, NIP-44 encryption/decryption adapters, NIP-59 wrapping helpers, and
NIP-98-style HTTP authorization helpers.

The Rust ecosystem has a higher-level `nostr-sdk` crate and a lower-level
`nostr` protocol crate from the same `rust-nostr` project. The crate named
`ndk` in Rust is Android NDK, not Nostr Dev Kit.

## Decision

`finite-nostr` will start by wrapping the lower-level `nostr` crate, using only
the NIP features needed by Finite projects.

`finite-nostr` will not depend on `nostr-sdk` until a consuming repo needs
relay/client/subscription behavior.

## Consequences

- `finite-nostr` stays focused on deterministic protocol helpers.
- FiniteBrain can depend on reusable event, NIP-44, NIP-59, and auth helpers
  without pulling in relay/client behavior.
- If a future Finite repo needs full Nostr client behavior, that can be added
  deliberately as a higher-level module or separate crate feature.

