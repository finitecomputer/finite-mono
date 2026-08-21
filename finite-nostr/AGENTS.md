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

## Engineering Style

`finite-nostr` follows the Finite Rust engineering style:

- Typed errors at crate boundaries.
- Explicit validation for protocol inputs.
- No FiniteBrain-specific policy in reusable Nostr primitives.
- No hidden defaults for security-relevant operations.
- Tests for valid, invalid, replay, and malformed event cases.
