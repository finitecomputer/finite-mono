# Structural Slice Brief: Deepen The fbrain CLI Module

## Selected Candidate

- Candidate: Deepen the fbrain CLI module.
- Candidate report path:
  `/var/folders/dq/xkm6n6s1687cdwkx1tcthdxh0000gn/T/architecture-review-20260624T213358Z.html`
- Why this candidate: it preserves the existing public CLI runner seam while
  moving command-support implementation into named internal modules.
- Recommendation strength: Strong.

## Design

- Module: `finite-brain-cli`.
- Interface: existing public `CliEnvironment`, `CliError`,
  `run_from_process`, and `run_with_env`.
- Seam: the current runner seam used by `src/main.rs` and the crate tests.
- Adapters: none introduced; internal modules remain in-process
  implementation.
- Leverage: command handlers and tests continue to exercise one CLI runner
  interface while lower-level responsibilities become easier to locate.
- Locality: argument parsing, output formatting, HTTP transport, local state,
  signing, and model types stop living as one large implementation body.

## Scope

- In scope:
  - Split `crates/finite-brain-cli/src/lib.rs` into internal modules.
  - Preserve public exports and command behavior.
  - Preserve JSON/text output shape, local file layout, and signed server route
    usage.
  - Keep existing tests at the CLI behavior surface.
- Out of scope:
  - Resident daemon process.
  - File-watch encrypted object writeback.
  - HTTPS transport.
  - Platform keychain or secret backend.
  - New command behavior or output contracts.
  - Unselected architecture candidates.
- Files likely to change:
  - `crates/finite-brain-cli/src/lib.rs`
  - internal files under `crates/finite-brain-cli/src/`
  - `docs/improve-codebase/2026-06-24-fbrain-agent-cli-architecture-ledger.md`
  - `docs/improve-codebase/2026-06-24-fbrain-cli-module-slice-brief.md`
  - final review packet for this slice
- Behavior changes approved: none.
- Parked follow-up candidates:
  - Carve a Local Agent Runtime state module.
  - Deepen admin command construction.
  - Deepen server admin mutation helpers.

## Tests And Checks

- Interface-level tests: `cargo test -p finite-brain-cli`.
- Targeted checks: `cargo fmt --check`, `cargo check -p finite-brain-cli`,
  `cargo clippy -p finite-brain-cli --all-targets -- -D warnings`.
- Full relevant suite: `cargo check --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `cargo build`, `git diff --check`.
- Visual or manual checks, if any: not applicable for CLI refactor.

## Implementation Contract

- Implement one bounded structural slice.
- Preserve behavior unless explicitly approved.
- Keep tests at the selected interface where possible.
- Do not expand into unselected candidates.
- Route feature, deployment, or context work to the right loop.

## Human Gates

- Interface or seam decision: resolved; keep existing public runner seam.
- Behavior change: not approved.
- ADR conflict: none found.
- Ownership or risk concern: none found.
- Review escalation: required if review finds a severe standards/spec issue.
