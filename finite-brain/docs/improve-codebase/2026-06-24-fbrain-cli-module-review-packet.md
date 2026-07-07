# Review Packet: Deepen The fbrain CLI Module

## Issue

- Issue: Improve Codebase selected candidate 1, Deepen the fbrain CLI module.
- Slice type: structural refactor.
- Acceptance criteria:
  - Preserve existing public `finite-brain-cli` runner interface.
  - Split command-support implementation into named internal modules.
  - Preserve CLI behavior, output shape, file layout, and signed route usage.
  - Keep tests at the CLI behavior interface.
- Baseline: `21994fd`
- Current diff: working tree before slice commit

## Implementation Summary

The `finite-brain-cli` crate now keeps the public runner and command flow in
`src/lib.rs`, while lower-level responsibilities live in internal modules:
arguments, environment, error type, clock/id helpers, output formatting, HTTP
transport, local state, signing, admin command helpers, and model types.

No feature behavior was intentionally changed.

## Implementation Evidence

- `implement` session: current Codex thread.
- `tdd` used: existing interface-level tests were preserved; no new behavior was
  introduced.
- Red test, if applicable: not applicable for behavior-preserving refactor.
- Green implementation, if applicable: existing CLI behavior tests remain green.
- Refactor, if applicable: split one large CLI implementation module into
  focused internal modules while preserving public exports.
- Commands run:
  - `cargo test -p finite-brain-cli` baseline passed before refactor.
  - `cargo fmt --check`
  - `cargo check -p finite-brain-cli`
  - `cargo test -p finite-brain-cli`
  - `cargo clippy -p finite-brain-cli --all-targets -- -D warnings`
  - `cargo check --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cargo build`
  - `git diff --check`

## Review Instructions

Review only this structural slice unless you find a severe cross-slice
regression. Keep standards and spec findings separate.

Check:

- Acceptance criteria are met.
- Tests verify behavior through public interfaces.
- No implementation-only tests are masquerading as behavior tests.
- No obvious incomplete work, TODO placeholders, or unrelated changes.
- Relevant test, typecheck, build, and diff hygiene commands pass.

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- None.

SPEC_STATUS: pass
SPEC_FINDINGS:
- None.
```

## Review Notes

- Standards sources checked: `AGENTS.md`, `CONTEXT.md`, `docs/adr/`, and the
  selected structural slice brief.
- Spec source checked:
  `docs/improve-codebase/2026-06-24-fbrain-cli-module-slice-brief.md`.
- The public seam remains `CliEnvironment`, `CliError`, `run_from_process`, and
  `run_with_env`.
- The selected slice did not introduce the parked daemon, HTTPS, keychain, or
  encrypted writeback work.
