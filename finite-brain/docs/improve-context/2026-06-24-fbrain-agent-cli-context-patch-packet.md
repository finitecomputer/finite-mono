# Context Patch Packet: fbrain Agent CLI

## Patch Frame

- Target repo: `finitecomputer/finite-brain`
- Context concern: align durable docs with the settled `fbrain` command name
  and current PR evidence after the Agent CLI feature slice.
- Patch type: documentation-only
- Branch: `feature/fbrain-agent-cli`
- Source evidence: `crates/finite-brain-cli/Cargo.toml`, `fbrain --help`,
  `README.md`, `CONTEXT.md`, and GitHub PR `#42`.
- Grilling needed: no

## Findings

| Finding | Evidence | Routed artifact | Action |
| --- | --- | --- | --- |
| Agent-facing examples should invoke `fbrain`, not require repo-root Cargo commands inside a Vault Working Tree. | The binary is named `fbrain`; `fbrain --help` returns the command surface; the user explicitly selected `fbrain` as invocation. | `README.md` | Replace Agent CLI examples with canonical `fbrain` commands and keep a short repo-development Cargo note. |
| Feature ledger PR evidence was stale. | `gh pr view` returned open non-draft PR `#42` from `feature/fbrain-agent-cli` to `staging`. | `docs/feature-dev/2026-06-24-fbrain-agent-cli-ledger.md` | Record the PR URL and concrete feature commit. |
| CLI crate module depth is real, but not a context patch. | `crates/finite-brain-cli/src/lib.rs` owns command parsing, state, signer, HTTP, sync, and admin flows. | Improve Codebase handoff | Park for the next loop. |

## Files Changed

- File: `README.md`
  - Why this artifact owns the fact: the README is the top-level local usage
    entrypoint.
  - Evidence: `crates/finite-brain-cli/Cargo.toml` declares binary name
    `fbrain`; the current CLI help output exposes the command families.
  - Terminology, spec, or ADR decision: none; this records the already-settled
    command name.
  - Change summary: use canonical `fbrain` examples and explain how to run the
    same command through Cargo during repo development.
- File: `docs/feature-dev/2026-06-24-fbrain-agent-cli-ledger.md`
  - Why this artifact owns the fact: the feature ledger tracks feature-run
    evidence and PR state.
  - Evidence: GitHub PR `#42` is open and non-draft against `staging`.
  - Terminology, spec, or ADR decision: none.
  - Change summary: replace pending PR evidence with the actual PR URL and
    feature commit SHA.
- File: `docs/improve-context/2026-06-24-fbrain-agent-cli-context-ledger.md`
  - Why this artifact owns the fact: Improve Context ledgers preserve audit and
    routing decisions.
  - Evidence: context inventory and command/PR checks from this run.
  - Terminology, spec, or ADR decision: none.
  - Change summary: record findings, routing, parked work, and drift checks.
- File: `docs/improve-context/2026-06-24-fbrain-agent-cli-context-patch-packet.md`
  - Why this artifact owns the fact: patch packets summarize the reviewable
    documentation-only edit set.
  - Evidence: same as the ledger.
  - Terminology, spec, or ADR decision: none.
  - Change summary: preserve the accepted edit set and handoff to Improve
    Codebase.

## Guardrails

- `CONTEXT.md` stays glossary-only.
- ADRs are only for hard-to-reverse, surprising, real trade-offs.
- Spec edits preserve accepted behavior unless the human explicitly decides
  otherwise.
- Agent docs hold operating rules, not domain essays.
- Wiki/repo-map edits maintain links and avoid duplicate pages.
- Temporary run state stays in the run ledger.
- Implementation, production, and broad architecture work are parked or handed
  off.

## Drift Check

- Links: referenced repo files exist.
- Paths: `crates/finite-brain-cli/Cargo.toml` and referenced run artifacts
  exist.
- Commands: `cargo run -p finite-brain-cli --bin fbrain -- --help` and
  `git diff --check` passed.
- Contradictions: old `cargo run` examples remain only in development notes,
  not as Vault Working Tree commands.
- Documentation-only scope: patch touches README and docs only.

## Parked Work

- Feature Dev: none.
- Improve Codebase: produce an architecture candidate report for the `fbrain`
  CLI module boundary, with human selection before implementation.
- Deployment: none.
- Future Improve Context: update CLI docs again after the resident daemon,
  platform secret backend, HTTPS transport, or encrypted writeback graduate from
  hardening work into implemented behavior.
