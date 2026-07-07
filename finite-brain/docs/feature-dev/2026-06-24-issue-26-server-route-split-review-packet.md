# Issue 26 Review Packet: Server Route Split

## Scope

- `crates/finite-brain-server/src/lib.rs`
- `crates/finite-brain-server/src/routes/*.rs`

## Checks

- `cargo fmt --all --check`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build --workspace`
- `node --check crates/finite-brain-server/src/product-client.js`
- `node --check crates/finite-brain-server/src/smoke-ui.js`
- `node crates/finite-brain-server/src/product-client.test.js`

## Result

Pass. Route handlers are separated by domain and the existing server route tests continue to pass.
