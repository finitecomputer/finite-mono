# Issue 24 Session: CI Guardrails

## Issue

- Issue: finitecomputer/finite-brain#24
- Fixed point before session:
  - finite-brain: `7283b6c0affe7f718b26b8d93cdbd0de2dda31ce`
  - finite-nostr: `621bb347f9734f2dcb891333ed8e7c2862ca73e1`
- Worker session: current Codex thread
- Commit:
  - finite-nostr: `baaa13a05f3691cf207f78f640f99c8bbd76cb0b`
  - finite-brain: this commit (`Add Rust CI guardrails`)
- Status: complete

## Inputs

- PRD issue: finitecomputer/finite-brain#23
- Slice issue: finitecomputer/finite-brain#24
- Relevant glossary terms: FiniteBrain Portable v1, FiniteBrain Policy, Reusable Nostr Primitive, Hard Cut
- Relevant ADRs:
  - `docs/adr/0001-adopt-rust-workspace-and-finite-nostr.md`
  - `docs/adr/0002-use-sqlite-from-day-one.md`
  - finite-nostr `docs/adr/0001-use-rust-nostr-protocol-crate.md`
- Prototype answer, if any: none

## Implementation

- Public interface used: GitHub Actions workflow entrypoints for pull requests and pushes.
- Behaviors covered:
  - finite-brain CI runs Cargo formatting, tests, clippy, build, Product Client JavaScript syntax checks, Smoke UI syntax checks, and Product Client smoke tests.
  - finite-nostr CI runs Cargo formatting, tests, clippy, and build.
  - finite-brain CI loads a read-only `finite-nostr` deploy key from `FINITE_NOSTR_DEPLOY_KEY` so Cargo can fetch the private companion crate.
  - No deployment, secrets, production config, or live data operations are introduced.
- `tdd` used: not applicable; this is CI workflow configuration. Local command parity was verified directly.
- Commands run during implementation:
  - finite-brain: `cargo fmt --all --check`
  - finite-nostr: `cargo fmt --all --check`
  - finite-brain: `node --check crates/finite-brain-server/src/product-client.js`
  - finite-brain: `node --check crates/finite-brain-server/src/smoke-ui.js`
  - finite-brain: `node crates/finite-brain-server/src/product-client.test.js`
  - finite-brain: `cargo test --workspace`
  - finite-nostr: `cargo test`
  - finite-brain: `cargo clippy --workspace --all-targets -- -D warnings`
  - finite-brain: `cargo build --workspace`
  - finite-nostr: `cargo clippy --all-targets -- -D warnings`
  - finite-nostr: `cargo build`
- Full suite command:
  - finite-brain: `cargo test --workspace`
  - finite-nostr: `cargo test`

## Review

- Review fixed point:
  - finite-brain: `7283b6c0affe7f718b26b8d93cdbd0de2dda31ce`
  - finite-nostr: `621bb347f9734f2dcb891333ed8e7c2862ca73e1`
- Standards findings: pass; workflows follow repo-local commands and keep production work out of scope.
- Spec findings: pass; all issue #24 acceptance criteria are covered.
- Worthy fixes applied: none.
- Findings ignored with reasons: none.

## Risks

- GitHub-hosted CI has not run until the branch is pushed; local command parity passed.
- Follow-up after push: GitHub-hosted CI could not fetch private `finite-nostr` without credentials, so a read-only deploy key was added to `finite-nostr` and stored as a finite-brain Actions secret.
