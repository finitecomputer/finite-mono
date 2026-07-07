# Improve Context Ledger: Rust v1 Product Client Drift

## Run

- Run ID: `2026-06-24-rust-v1-context`
- Loop: Improve Context
- Target repo: `finitecomputer/finite-brain`
- Base branch: `staging`
- Context branch: `feature/rust-portable-v1-core`
- Status: context patch applied locally

## Concern

The branch now contains a first-party Product Client, graph/replay seams, OKF
import execution, hardening, and a local/staging runbook, but the top-level
README and first workspace ADR still described the app crate as only a
development smoke app.

## Context Surfaces Checked

- `README.md`
- `CONTEXT.md`
- `AGENTS.md`
- `docs/adr/`
- `docs/runbooks/product-client-parity-local-staging.md`
- `docs/readiness/portable-v1-hardening.md`
- `docs/feature-dev/2026-06-24-rust-v1-product-parity-goal-ledger.md`

## Changes

- Updated `README.md` to name the Product Client, Smoke UI routes, and app
  runtime environment variables.
- Updated `CONTEXT.md` glossary language so FiniteBrain Policy includes the
  Product Client and hardening rules.
- Updated ADR 0001 to describe `finite-brain-app` as the application server
  binary rather than a tiny smoke server.

## Parked Notes

- The main checkout blocked on working-tree content reads during this run, so
  the patch was prepared in a fresh temporary clone at
  `/tmp/finite-brain-context-work`.
- Runtime wording in `crates/finite-brain-app/src/main.rs` still says
  "smoke server"; changing runtime output is implementation work, not this
  documentation-only context patch.
