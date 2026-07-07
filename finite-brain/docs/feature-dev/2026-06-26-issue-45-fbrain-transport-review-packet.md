# Issue #45 Review Packet: fbrain Transport

## Issue

- Issue: `finitecomputer/finite-brain#45`
- Slice type: AFK
- Acceptance criteria:
  - URL precedence is explicit `--server`, saved working-tree URL, `FINITE_BRAIN_SERVER_URL`, then legacy `FINITE_BRAIN_PUBLIC_BASE_URL`.
  - `doctor`, command requests, and sync requests use the same resolver.
  - `http_request` supports `https://`.
  - Signed auth URL canonicalization remains bound to the absolute request URL.
  - Focused CLI tests cover env precedence and HTTPS URL acceptance without requiring a public network service.
- Baseline: `df69b01521d9e126f430d926a7730f4f4c641d05`
- Current diff: `git diff df69b01521d9e126f430d926a7730f4f4c641d05...HEAD`

## Implementation Summary

`fbrain` now has a dedicated agent transport URL contract, supports HTTPS-capable requests, and uses the shared resolver for normal commands and sync-triggered commands.

## Implementation Evidence

- `implement` session: current thread
- `tdd` used: yes, focused CLI tests.
- Red test, if applicable: not preserved as a separate commit.
- Green implementation, if applicable: `cargo test -p finite-brain-cli`, `cargo test --workspace`, and live smoke.
- Refactor, if applicable: `submit_change_intent` and signed revision helper were folded into input/context structs after clippy review.
- Commands run:
  - `cargo fmt --check`
  - `cargo check -p finite-brain-cli`
  - `cargo test -p finite-brain-cli`
  - `cargo check --workspace`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo build`
  - `git diff --check`

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- None. The diff follows AGENTS.md protocol/sync guidance with explicit control flow, executable tests, and no plaintext server boundary drift.

SPEC_STATUS: pass
SPEC_FINDINGS:
- None. The implementation satisfies issue #45 and PRD #43 transport criteria.
```
