# Issue Session

## Issue

- Issue: https://github.com/finitecomputer/finite-mono/issues/219
- Fixed point before session: `b5f8ba9`
- Worker session: `/root/ticket_219_worker`
- Commit: `53953b6`
- Status: implementation and repository integration complete; repeat
  independent standards/spec review and pushed-commit checks pending

## Inputs

- Spec issue: https://github.com/finitecomputer/finite-mono/issues/216
- Ticket: https://github.com/finitecomputer/finite-mono/issues/219
- Relevant glossary terms: Brain Role, Folder Access Readiness, Organization
  Brain Collaboration, Managed Agent Email, Brain Working Tree
- Relevant ADRs:
  `finite-brain/docs/adr/0034-make-organization-brain-collaboration-a-desired-state-operation.md`
- Prototype answer and source branch, if any: none

## Implementation

- Public interfaces used:
  - Managed `finitebrain` skill and its `fbrain` CLI reference.
  - Built `fbrain` process with two independent Finite Homes against the real
    signed Brain HTTP router.
  - `just skills check` and `just brain-product-matrix`.
- Behaviors covered:
  - Normal Agent-to-Agent Organization Brain sharing uses only
    `collaborators ensure-admin` with the canonical Managed Agent Email.
  - The managed skill delegates identity resolution to `fbrain`, inspects
    `complete`, `partial`, or `indeterminate`, and gives honest same-command
    retry guidance without exposing identity keys or grant material.
  - Low-level member, admin, and Folder-grant operations are explicitly
    advanced primitives that cannot prove complete collaboration.
  - Both first-party skill/reference copies are byte-identical and the static
    delivery gate enforces the collaboration contract and rejects an ad hoc
    NIP-05 curl probe.
  - Fresh Alpha and Beta Finite Homes prove pre-existing restricted knowledge,
    native email collaboration, Beta open/read/edit/sync, and Alpha
    sync/read-back through the built CLI and signed server.
  - The smoke emits a minimal boundary-labelled JSON artifact; a separate
    verifier requires the complete proof and rejects secret-bearing material.
- `tdd` used: yes. The static delivery test first failed for skill-copy drift
  and every missing collaboration behavior, then passed after both managed
  copies were updated. The existing process acceptance seam was extended
  vertically from recipient read through recipient edit and inviter read-back.
- Commands run during implementation:
  - `just skills check`
  - `diff -ru finite-brain/skills/finitebrain finite-skills/skills/software-development/finitebrain`
  - `FINITE_BRAIN_COLLABORATION_SMOKE_REPORT=target/brain-product-matrix/organization-collaboration.json cargo test --locked -p finite-brain-cli --test fbrain_process_acceptance built_fbrain_process_two_independent_homes_open_restricted_collaboration -- --nocapture`
  - `scripts/check-brain-collaboration-smoke-report.py target/brain-product-matrix/organization-collaboration.json`
  - `python3 -m py_compile scripts/check-brain-collaboration-smoke-report.py`
  - `cargo fmt --all -- --check`
- Full suite command: `cargo test --locked -p finite-brain-cli`
- Full-suite note: the first full CLI-suite attempt completed its test binaries
  but wedged in a zero-progress Rust doctest process and was terminated after
  six minutes. The focused public acceptance seam was rerun cleanly afterward.
- Final repository integration gate: `just brain-product-matrix` now runs the
  Alpha/Beta collaboration proof and report verifier before the existing
  disposable real-Hermes product matrix.
- Orchestrator verification:
  - `DEVFINITY_READY_TIMEOUT_SECS=1800 just dev smoke` passed. The longer
    readiness bound was required only for the cold sequential Rust build; the
    resulting stack and prescribed smoke completed cleanly.
  - `just brain-product-matrix` passed after correcting its browser helper to
    prefer an installed Playwright browser over a stale package-manager PATH
    shim. The successful run covered the Alpha/Beta collaboration proof,
    hosted Hermes/managed-skill setup and reconciliation, Product Client
    browser paths, recovery, and a fresh Agent turn.
  - Pull-request checks on the pushed commit remain pending. This issue must
    not be approved or closed before those pass.

## Review

- Review fixed point: `b5f8ba9`
- Standards findings: approved with no material findings.
- Spec findings: integration evidence pending; the first pass also found
  inaccurate early-failure facts and a missing-holder retry gap.
- Worthy fixes applied: every artifact fact that represents completed work is
  now derived from passed boundaries; partial guidance uses a supplied holder
  email when available and otherwise asks another current Folder reader,
  without inventing or exposing identity. The matrix browser launcher now
  selects a real Playwright executable before PATH shims that may outlive their
  removed application bundle.
- Findings ignored with reasons: none

## Risks

- The deterministic Alpha/Beta seam uses synthetic local identities and an
  isolated in-memory Brain store. It proves the cryptographic and signed HTTP
  product path without touching production or durable user state.
