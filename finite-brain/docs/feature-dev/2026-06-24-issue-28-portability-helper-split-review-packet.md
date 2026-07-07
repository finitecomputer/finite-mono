# Issue 28 Review Packet: Portability Helper Split

## Scope

- `crates/finite-brain-core/src/portability.rs`
- `crates/finite-brain-core/src/portability/*.rs`

## Checks

- `cargo fmt --all --check`
- `cargo test -p finite-brain-core`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build --workspace`

## Result

Pass. Portability APIs are still exported from `finite_brain_core::portability`, while implementation code is split by domain.
