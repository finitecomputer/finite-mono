# Issue #45 Session: fbrain Transport

## Issue

- Issue: `finitecomputer/finite-brain#45`
- Fixed point before session: `df69b01521d9e126f430d926a7730f4f4c641d05`
- Worker session: current thread
- Commit: `8bf422f969aa689c7c0214d70f98df85f1eca7b7`
- Status: implemented, verified locally

## Inputs

- PRD issue: `finitecomputer/finite-brain#43`
- Slice issue: `finitecomputer/finite-brain#45`
- Relevant glossary terms: Agent CLI, Brain Working Tree, Local Agent Signer, Blocked Sync State
- Relevant ADRs/specs: `AGENTS.md`, `CONTEXT.md`, `docs/specs/finitebrain-portability-spec.md`, `docs/adr/0001-adopt-rust-workspace-and-finite-nostr.md`
- Prototype answer, if any: none

## Implementation

- Public interface used: `fbrain` command seam through `run_with_env`, `http_request`, signed JSON requests, and live `target/debug/fbrain`.
- Behaviors covered:
  - `FINITE_BRAIN_SERVER_URL` added as the agent transport URL.
  - URL precedence is explicit `--server`, saved Brain Working Tree URL, `FINITE_BRAIN_SERVER_URL`, then legacy `FINITE_BRAIN_PUBLIC_BASE_URL`.
  - The CLI HTTP client now supports both `http://` and `https://` via `ureq`.
  - `doctor`, command requests, `open`, `daemon start`, `daemon tick`, and `sync now` use the same resolver path.
  - Signed HTTP auth remains bound to the absolute request URL used by the request.
- `tdd` used: focused CLI tests for URL precedence, HTTPS URL acceptance, and public-seam sync `--server` override.
- Commands run during implementation:
  - `cargo fmt --check`
  - `cargo check -p finite-brain-cli`
  - `cargo test -p finite-brain-cli`
  - `cargo check -p finite-brain-server`
  - `cargo test -p finite-brain-server finish_setup_route_repairs_empty_setup_incomplete_folder -- --nocapture`
  - `cargo test -p finite-brain-server secure_object_routes_create_update_delete_and_pull_sync -- --nocapture`
  - `cargo build -p finite-brain-app -p finite-brain-cli`
  - live smoke against `http://127.0.0.1:4016`
  - `cargo fmt --check && cargo check --workspace && cargo test --workspace`
  - `cargo fmt --check && cargo test -p finite-brain-cli && cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo build && git diff --check`
- Full suite command: `cargo test --workspace`

## Review

- Review fixed point: `df69b01521d9e126f430d926a7730f4f4c641d05`
- Standards findings: none from direct two-axis review.
- Spec findings: none from direct two-axis review.
- Worthy fixes applied:
  - Passed sync command args into the server URL resolver so `fbrain sync now --server ...` uses the explicit URL.
  - Refactored sync submit/revision helper arguments to satisfy clippy with warnings denied.
  - Addressed local CodeRabbit findings for local-only plaintext HTTP, validated `open` URL persistence, timestamp overflow handling, and bootstrap Folder Key reuse.
- Findings ignored with reasons: none.

## Risks

- Resident OS daemon supervision and file watching remain out of scope for this slice; command-driven sync is the verified behavior.
