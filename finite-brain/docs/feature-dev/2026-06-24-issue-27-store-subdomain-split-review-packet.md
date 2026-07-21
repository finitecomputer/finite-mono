# Issue 27 Review Packet: Store Subdomain Split

## Scope

- `crates/finite-brain-store/src/lib.rs`
- `crates/finite-brain-store/src/schema.rs`
- `crates/finite-brain-store/src/loading.rs`
- `crates/finite-brain-store/src/brains.rs`
- `crates/finite-brain-store/src/folder_access.rs`
- `crates/finite-brain-store/src/links.rs`
- `crates/finite-brain-store/src/shared_folders.rs`

## Checks

- `cargo fmt --all --check`
- `cargo test -p finite-brain-store`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build --workspace`

## Result

Pass. Store behavior remains covered by 32 store tests and the workspace suite.
