# Review Packet: Issue 24 CI Guardrails

## Issue

- Issue: finitecomputer/finite-brain#24
- Slice type: AFK
- Acceptance criteria:
  - finite-brain has Cargo and JavaScript CI checks.
  - finite-nostr has Cargo CI checks.
  - CI commands match local passing commands.
  - Workflows avoid production deploys, secrets, and live data operations.
- Baseline:
  - finite-brain: `7283b6c0affe7f718b26b8d93cdbd0de2dda31ce`
  - finite-nostr: `621bb347f9734f2dcb891333ed8e7c2862ca73e1`
- Current diff:
  - Added finite-brain `.github/workflows/ci.yml`
  - Added finite-nostr `.github/workflows/ci.yml`
  - Added Feature Dev ledger and issue session evidence

## Implementation Summary

Both Rust repos now have GitHub Actions CI definitions for the commands that
already pass locally. finite-brain also checks the Product Client and Smoke UI
JavaScript entrypoints. finite-brain's Rust CI job authenticates private
`finite-nostr` fetches with a read-only deploy key.

## Implementation Evidence

- `implement` session: current Codex thread
- `tdd` used: not applicable for CI workflow configuration
- Red test, if applicable: not applicable
- Green implementation, if applicable: local command parity passed
- Refactor, if applicable: none
- Commands run:
  - `cargo fmt --all --check`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo build --workspace`
  - `node --check crates/finite-brain-server/src/product-client.js`
  - `node --check crates/finite-brain-server/src/smoke-ui.js`
  - `node crates/finite-brain-server/src/product-client.test.js`
  - finite-nostr `cargo fmt --all --check`
  - finite-nostr `cargo test`
  - finite-nostr `cargo clippy --all-targets -- -D warnings`
  - finite-nostr `cargo build`

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- No documented standards violations found. The workflows use explicit Rust and Node setup, read-only permissions, repo-local commands, and no production operations.
- Follow-up check: the finite-brain Rust workflow now uses a read-only deploy key secret for the private finite-nostr git dependency.

SPEC_STATUS: pass
SPEC_FINDINGS:
- No spec gaps found. Issue #24 requested CI for finite-brain and finite-nostr with fmt, tests, clippy, build, and JS smoke checks; the added workflows cover those commands.
```
