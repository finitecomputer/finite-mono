# Agent Guide

## Agent skills

Follow the product-authority, issue-tracker, and retained-documentation
guidance in the root `../AGENTS.md`.

## Engineering Style

`finite-nostr` follows the Finite Rust engineering style:

- Typed errors at crate boundaries.
- Explicit validation for protocol inputs.
- No FiniteBrain-specific policy in reusable Nostr primitives.
- No hidden defaults for security-relevant operations.
- Tests for valid, invalid, replay, and malformed event cases.
