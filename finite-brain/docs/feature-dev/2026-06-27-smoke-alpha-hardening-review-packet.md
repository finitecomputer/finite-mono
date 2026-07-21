# Smoke Alpha Hardening Review Packet

## Scope

Feature Dev run: `2026-06-27-smoke-alpha-hardening`

Issues covered:

- `finitecomputer/finite-brain#48` Product Client Folder Key Grant hardening
- `finitecomputer/finite-brain#51` `fbrain daemon watch`
- `finitecomputer/finite-brain#49` Product Client organization Brain invitations
- `finitecomputer/finite-brain#50` smoke alpha backup, restore, and SilverBullet cutover handoff

## Summary

The branch is a hard-cut smoke-alpha readiness pass. It does not preserve the
old SilverBullet runtime as a compatibility layer. It prepares the Rust Product
Client, `fbrain`, and smoke operator handoff for the internal smoke deployment.

Implemented:

- Product Client Folder Key Grants now default to gift-wrapped NIP-44 open and
  creation paths, with plaintext development grants kept behind explicit test
  fallback helpers.
- `fbrain daemon watch` runs the real Brain Working Tree sync path in a
  foreground loop with bounded smoke/test options.
- Product Client Access sidebar now supports organization Brain invitation
  create, inspect, accept, and revoke flows.
- Smoke backup/restore/cutover runbook and local SQLite backup verifier were
  added.

## Review Findings

| Finding | Severity | Result |
| --- | --- | --- |
| Product Client enabled Brain invitation create/revoke controls while the active Brain was personal. Server-side authorization would reject the operation, but smoke users would get a confusing client path. | minor | Fixed by enabling create/revoke only when the active metadata kind is `organization`; inspect/accept stay available anywhere. |

No remaining blocking review findings are known.

## Verification

- `node --check crates/finite-brain-server/src/product-client.js`
- `node crates/finite-brain-server/src/product-client.test.js`
- `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
- `cargo test -p finite-brain-cli`
- `scripts/verify-smoke-alpha-backup-restore.sh`
- `cargo fmt --check`
- `node --check scripts/verify-obsidian-product-client.mjs`
- `git diff --check`
- `node scripts/seed-smoke-doc-pages.mjs`
- `node scripts/verify-obsidian-product-client.mjs`
- `cargo test --workspace`
- `cargo clippy --all-targets -- -D warnings`
- `cargo build --workspace`

## Deployment Handoff

Live smoke route mutation is intentionally outside this Feature Dev branch.
Deployment should use
`docs/runbooks/smoke-alpha-backup-restore-cutover.md` to replace the old
SilverBullet route with the Rust `finite-brain-app` service and verify the
Product Client plus `fbrain` paths on the smoke box.

