# ADR 0001: Adopt A Rust Workspace And A Separate finite-nostr Crate

Status: accepted

Date: 2026-06-23

## Context

FiniteBrain is being rebuilt from scratch in Rust from the FiniteBrain Portable
v1 specification. The rebuild needs production-shaped module boundaries early:
domain validation, encrypted object handling, sync rules, storage, HTTP APIs,
the Product Client, and development smoke tooling should be separable without
turning the first crate into a catch-all.

The rebuild also uses Nostr primitives that are useful outside FiniteBrain:
NIP-19 identity encoding, Nostr event construction and validation, NIP-44
encryption/decryption adapters, NIP-59 wrapping helpers, and NIP-98-style HTTP
authorization. Those primitives should be reusable by other Finite Rust repos.

## Decision

FiniteBrain will become a Cargo workspace immediately.

The intended workspace shape is:

- `crates/finite-brain-core`: domain model, validation, sync rules, encrypted
  object envelopes, and pure Portable v1 logic.
- `crates/finite-brain-store`: SQLite storage, schema, migrations, and
  transaction boundaries.
- `crates/finite-brain-server`: HTTP/API surface and request validation.
- `crates/finite-brain-app`: application server binary that wires runtime
  configuration, SQLite state, HTTP routes, the Product Client, and the
  development Smoke UI.

Reusable generic Nostr primitives live in a separate repository and crate:

- `finitecomputer/finite-nostr`

FiniteBrain application policy must not leak into `finite-nostr`.

`finite-nostr` will wrap the lower-level `nostr` crate from the `rust-nostr`
project first, not the higher-level `nostr-sdk` crate. The reusable crate should
depend on protocol primitives rather than relay/client behavior until a Finite
repo specifically needs a higher-level Nostr client surface.

## Consequences

- The first implementation work has more crate scaffolding than a single-crate
  prototype.
- Core logic can be tested without HTTP or SQLite.
- Storage invariants can be tested through a narrow store interface.
- HTTP routing can stay thin and delegate to core/store behavior.
- Other Finite repos can reuse the same Nostr primitive wrappers without
  depending on FiniteBrain domain concepts.
- The first Nostr integration work stays closer to deterministic protocol
  helpers: event construction/validation, NIP-44, NIP-59, and NIP-98-style HTTP
  auth.
