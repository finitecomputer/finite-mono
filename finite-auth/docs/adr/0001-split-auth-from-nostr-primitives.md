# ADR 0001: Split Auth Policy From Nostr Primitives

## Status

Accepted.

## Context

FiniteBrain needs Nostr-heavy authentication, and Finite repos already use
`finite-nostr` for reusable protocol wrappers. NIP-01 event identity,
signature verification, and NIP-98 HTTP authorization are generic Nostr
operations. Challenge replay handling, session state, and current NIP-05
bindings are Finite authentication policy.

## Decision

`finite-auth` owns authentication policy and durable auth state. It depends on
`finite-nostr` for Nostr protocol validation and does not reimplement event ID
serialization, Schnorr signature checks, NIP-19 parsing, or NIP-98 auth event
semantics.

The initial repo is a Rust workspace with:

- `finite-auth-core` for bounded domain types and validation.
- `finite-auth-store` for SQLite-backed challenge, session, and NIP-05 binding
  state.

## Consequences

FiniteBrain can consume authenticated principals without importing auth storage
details into its core domain. `finite-nostr` remains product-neutral. Any
future auth transport or signer support must preserve the public-key-first
identity rule.
