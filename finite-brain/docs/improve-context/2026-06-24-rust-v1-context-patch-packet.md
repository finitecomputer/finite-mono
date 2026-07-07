# Context Patch Packet: Rust v1 Product Client Drift

## Summary

This patch aligns durable context with the current Rust v1 branch: the app crate
now serves the Product Client and the development Smoke UI, not just a smoke app.

## Files Changed

- `README.md`
- `CONTEXT.md`
- `docs/adr/0001-adopt-rust-workspace-and-finite-nostr.md`
- `docs/improve-context/2026-06-24-rust-v1-context-ledger.md`

## Evidence To Run

```sh
git diff --check
```

Optional broader checks remain in the existing Product Client parity runbook:

```sh
node --check crates/finite-brain-server/src/product-client.js
node crates/finite-brain-server/src/product-client.test.js
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
```

## Handoff

Use `docs/runbooks/product-client-parity-local-staging.md` for the executable
client/server parity checklist. Use `CONTEXT.md` for glossary terms and ADR 0001
for the workspace-shape decision.
