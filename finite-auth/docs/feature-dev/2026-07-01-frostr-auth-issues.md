# Frostr Auth Local Issue Breakdown

Status: Historical artifact. ADR 0003 and the shared Frostr signer issue
breakdown supersede the delegated agent-key acceptance criteria below.

## Local-1: Frostr Auth Model Scaffold

Type: AFK

Blocked by: None.

User stories covered: 1, 2, 3, 4, 5.

### What to build

Add a narrow Frostr-aware auth scaffold that models a user's primary key as a
2-of-3 Frostr group public key, records fixed share placements, and records
per-agent delegated Nostr public keys.

### Acceptance Criteria

- [x] Core domain types reject malformed Frostr keyset placement.
- [x] Core domain types reject an agent key that equals the user's primary key.
- [x] Store schema persists Frostr keyset metadata under constraints.
- [x] Store schema persists delegated agent keys and reloads them after restart.
- [x] Docs explain where finite-auth stops and bifrost-rs begins.
- [x] `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings` pass.
