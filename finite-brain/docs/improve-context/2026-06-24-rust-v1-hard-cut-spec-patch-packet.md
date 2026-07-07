# Context Patch Packet: Rust v1 Hard-Cut Spec Drift

## Patch Frame

- Target repo: `finitecomputer/finite-brain`
- Context concern: the portability spec still described old prototype bridges after the Rust hard-cut branch moved protected routes and authoritative state forward.
- Patch type: documentation-only
- Branch: `feature/rust-portable-v1-core`
- Source evidence: `crates/finite-brain-server/src/lib.rs`, `crates/finite-brain-server/src/protected_routes.rs`, `crates/finite-brain-store/src/lib.rs`, ADR 0002.
- Grilling needed: no

## Findings

| Finding | Evidence | Routed artifact | Action |
| --- | --- | --- | --- |
| Auth bridge language was stale. | Protected handlers call `validate_request_auth`; tests reject missing auth. | `docs/specs/finitebrain-portability-spec.md` | State that protected routes derive actor identity from Nostr auth. |
| Backup/storage language was stale. | ADR 0002 and `BrainStore` use SQLite as the authoritative store. | `docs/specs/finitebrain-portability-spec.md` | Describe SQLite backup/restore shape instead of JSON metadata plus sync. |
| Legacy plaintext file route language was stale. | Router has encrypted object routes and no `/files/*` routes. | `docs/specs/finitebrain-portability-spec.md` | Remove legacy plaintext route surface from Rust hard-cut default flow. |
| Source map pointed at old prototype files. | Current repo source is the Rust workspace. | `docs/specs/finitebrain-portability-spec.md` | Replace with current Rust crate/module map. |

## Files Changed

- File: `docs/specs/finitebrain-portability-spec.md`
  - Why this artifact owns the fact: it is the Portable v1 implementation contract.
  - Evidence: current router, protected-route module, SQLite store, ADR 0002.
  - Terminology, spec, or ADR decision: no new decision; records already-implemented Rust hard-cut behavior.
  - Change summary: auth, storage, route-surface, compatibility, and source-map drift corrections.

## Guardrails

- `CONTEXT.md` stays glossary-only.
- ADRs are only for hard-to-reverse, surprising, real trade-offs.
- Spec edits preserve accepted behavior unless the human explicitly decides otherwise.
- Agent docs hold operating rules, not domain essays.
- Temporary run state stays in the run ledger.
- Implementation, production, and broad architecture work are parked or handed off.

## Drift Check

- Links: pass.
- Paths: pass.
- Commands: `git diff --check` passed.
- Contradictions: stale prototype strings searched and absent from the spec.
- Documentation-only scope: pass so far.

## Parked Work

- Feature Dev: none.
- Improve Codebase: continue with one bounded structural slice after context patch.
- Deployment: none.
- Future Improve Context: consider generating or maintaining route-surface docs from the router so the spec cannot drift as easily.
