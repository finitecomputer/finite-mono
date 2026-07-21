# Improve Context Ledger: Rust v1 Hard-Cut Spec Drift

## Run

- Run ID: `2026-06-24-rust-v1-hard-cut-spec`
- Loop: Improve Context
- Target repo: `finitecomputer/finite-brain`
- Base branch: `staging`
- Context branch: `feature/rust-portable-v1-core`
- Human owner: delegated to Codex for this round
- Started: 2026-06-24
- Current status: context patch verified locally

## Context Frame

- Starting concern: run another long context/codebase pass with agent control.
- Specific area of concern, if any: none named.
- Out of scope: runtime behavior, production deployment, broad spec rewrite.
- Known commands: `rg`, `sed`, `git diff --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo build`.
- Context surfaces inventoried: `AGENTS.md`, `README.md`, `CONTEXT.md`, `docs/adr/`, `docs/agents/`, `docs/specs/finitebrain-portability-spec.md`, prior improve-context and improve-codebase ledgers.
- Specs or PRDs inventoried: `docs/specs/finitebrain-portability-spec.md`.
- Source-of-truth notes: current Rust route catalog is `crates/finite-brain-server/src/lib.rs`; protected auth is `crates/finite-brain-server/src/protected_routes.rs`; authoritative storage is `crates/finite-brain-store/src/lib.rs` and ADR 0002.

## Audit Findings

| Finding | Artifact | Evidence | Decision |
| --- | --- | --- | --- |
| Spec still described `X-Actor-User-Id` and unauthenticated Brain creation as prototype bridges. | `docs/specs/finitebrain-portability-spec.md` | `create_brain_handler` and metadata routes call `validate_request_auth`; protected-route tests reject missing auth. | Update auth language to Rust hard-cut behavior. |
| Spec still described JSON metadata plus SQLite sync backup shape. | `docs/specs/finitebrain-portability-spec.md` | ADR 0002 says SQLite from day one; `BrainStore` owns SQLite schema for Brains, grants, sync, invitations, shares, and mounts. | Update storage and backup language to SQLite authoritative state. |
| Spec still listed legacy plaintext file routes. | `docs/specs/finitebrain-portability-spec.md` | `router_with_state` exposes encrypted object routes but no `/files/*` routes. | Remove legacy file route surface from Rust Portable v1 default flow. |
| Spec source map pointed at the previous TypeScript/Go prototype. | `docs/specs/finitebrain-portability-spec.md` | Current repo is the Rust workspace with `crates/finite-brain-*`. | Replace source map with Rust crate/module pointers. |

## Routing Decisions

- Accepted findings: the four spec drift findings above.
- Dropped findings: full route-surface generation is useful but larger than this small context patch.
- Parked findings: consider generating route documentation from the router in a future context or codebase slice.
- Source-of-truth conflicts: none; current code and ADR 0002 agree.
- Grilling sessions: not needed; these are evidence-backed drift fixes, not new terminology or policy.
- Human decisions: user delegated control for this round.

## Patch Packet

- Packet path: `docs/improve-context/2026-06-24-rust-v1-hard-cut-spec-patch-packet.md`
- Patch type: documentation-only
- Files changed: `docs/specs/finitebrain-portability-spec.md`, this ledger, patch packet.
- Evidence summary: route catalog, protected-route auth module, protected auth tests, ADR 0002, SQLite store schema.
- Non-context work parked: route-doc generation and further module extraction belong to Improve Codebase.

## Drift Check

| Check | Result | Notes |
| --- | --- | --- |
| Links | pass | No new external links; referenced local paths exist. |
| Paths | pass | Rust source-map paths exist in the current workspace. |
| Commands | pass | `git diff --check` passed. |
| Contradictions | pass | Stale prototype strings for JSON metadata, legacy file routes, and old source paths are absent from the spec. |
| Docs-only scope | pass | Current context patch changes markdown only. |

## PR And Handoff

- PR URL: `https://github.com/finitecomputer/finite-brain/pull/15`
- Commit SHA: `4614860`
- Review notes: context patch is evidence-backed and does not introduce implementation changes.
- Feature Dev handoff: none.
- Improve Codebase handoff: route-doc generation and next structural slice.
- Deployment handoff: none.
- Human-owned follow-up: none.

## Open Gates

- None.
