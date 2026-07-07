# Issue 29 Review Packet: Lifecycle Integration Test

## Scope

- `crates/finite-brain-store/src/lib.rs`

## Checks

- `cargo test -p finite-brain-store sqlite_full_lifecycle_invite_share_sync_revoke_and_filter_visibility -- --nocapture`
- `cargo fmt --all --check`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build --workspace`

## Result

Pass. The store now has a persisted SQLite lifecycle test for invite, share, sync projection, member removal, export filtering, and revocation.
