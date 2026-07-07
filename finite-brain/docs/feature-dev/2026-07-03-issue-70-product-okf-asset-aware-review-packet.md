# Review Packet: Issue #70 Product OKF Asset Awareness

## Scope

Issue #70 makes the trusted Product Client understand typed Assets while keeping
the Rust server plaintext-blind and keeping agent-readable surfaces Markdown
first.

## Files To Review

- `crates/finite-brain-server/src/product-client.js`
- `crates/finite-brain-server/src/product-client.test.js`

## Review Focus

- Does `openFolderObject` preserve existing Page decode compatibility while
  returning typed Asset objects for Asset plaintext?
- Does Asset plaintext encode enough metadata for OKF and future client writes?
- Are typed Asset paths constrained to the Folder-local `raw/assets/`
  convention?
- Does OKF import planning preserve accessible Assets without rewriting them as
  Markdown?
- Do copied Page links still rewrite only between Markdown Pages?
- Do search, graph, replay, command palette, and reader page rows exclude raw
  Asset bytes?
- Does the server remain plaintext-blind?

## Checks

- `node --check crates/finite-brain-server/src/product-client.js`
- `node crates/finite-brain-server/src/product-client.test.js`
- `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
- `git diff --check`
- `cargo fmt --check`
- `cargo test --workspace`
- `cargo check --workspace`
- `cargo clippy --all-targets -- -D warnings`
- `cargo build --workspace`

All checks passed.

## Review Result

Local code-review pass found no blocking issues.

The trusted-client boundary remains intact: Asset bytes are decoded and encoded
in the Product Client, while server routes continue to handle encrypted Folder
Object envelopes.
