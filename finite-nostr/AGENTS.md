# Agent Guide

## Agent skills

### Issue tracker

Issues and PRDs live in GitHub Issues for `finitecomputer/finite-nostr`. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the default Matt Pocock skill label vocabulary. See `docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repo: read root `CONTEXT.md` and root `docs/adr/` when present. See `docs/agents/domain.md`.

## Engineering Style

`finite-nostr` follows the Finite Rust engineering style:

- Typed errors at crate boundaries.
- Explicit validation for protocol inputs.
- No FiniteBrain-specific policy in reusable Nostr primitives.
- No hidden defaults for security-relevant operations.
- Tests for valid, invalid, replay, and malformed event cases.

