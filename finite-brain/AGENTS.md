# Agent Guide

## Agent skills

### Issue tracker

Issues and PRDs live in GitHub Issues for `finitecomputer/finite-mono`. See
the root `../docs/agents/issue-tracker.md`.

### Triage labels

Use the default Matt Pocock skill label vocabulary. See the root
`../docs/agents/triage-labels.md`.

### Domain docs

This component is part of the multi-context monorepo: read root
`../CONTEXT-MAP.md`, this component's `CONTEXT.md`, and relevant root or
component ADRs. See the root `../docs/agents/domain.md`.

### FiniteBrain agent skill

The FiniteBrain agent skill has one source:
`finite-skills/skills/software-development/finitebrain/SKILL.md` (monorepo
root). There is no component-local copy. Keep it aligned with `fbrain` CLI
ergonomics and Brain Working Tree conventions; `just skills check` and the
`finite-brain-cli` tests validate it.

### Asset source notes

FiniteBrain's LLM wiki surface is Markdown-first. Store non-Markdown source
bytes outside the Brain and represent each Asset with one Markdown Asset Source
Note under the containing Folder's `raw/` tree. Its frontmatter includes
`type`, `title`, and the canonical `resource` URI; `description` and known
`finite_asset` facts are optional. Agents should cite these notes from
synthesized `wiki/` pages instead of treating blob bytes as the primary
knowledge surface.

## Engineering Style

FiniteBrain Rust follows the Finite engineering style:

- Keep authoritative server state in schema, constraints, and transactions.
- Use typed error enums at crate boundaries.
- Make safety invariants executable through validation and tests.
- Prefer explicit control flow for protocol, storage, sync, and crypto-adjacent code.
- Put explicit limits on loops, batches, payloads, fanout, sync windows, and retry work.
- Keep compatibility hard cuts before first users unless real user data exists.
