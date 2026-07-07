# Agent Guide

## Domain Docs

This is a single-context repo: read root `CONTEXT.md`, root `docs/adr/`, and
root `docs/specs/` before changing auth behavior.

## Engineering Style

`finite-auth` follows the Finite Rust engineering style:

- Authentication state uses schema, constraints, and transactions.
- Nostr protocol primitives stay in `finite-nostr`; this repo owns auth policy.
- NIP-01 event identity and signature validation must be delegated to
  `finite-nostr` wrappers unless there is a documented gap.
- NIP-05 is identification, not proof of real-world identity.
- Use typed errors at crate boundaries.
- Keep auth work bounded: challenge lifetimes, session lifetimes, response
  sizes, relay lists, and replay windows must have explicit limits.
- Every state transition gets a valid test and at least one invalid or replay
  test.
- Do not add compatibility shims before first users; hard-cut scaffold changes
  and update tests to the current shape.
