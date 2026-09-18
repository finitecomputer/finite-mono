# Finite Computer v2 Agent Guide

This tree owns the self-serve SaaS product: Core, dashboard, Runner integration,
and Agent Runtime orchestration. Sibling products retain their own ownership.
Read [README.md](README.md#hard-cut-rules) for retained legacy boundaries and
[service ownership](docs/service-dependencies.md) before changing product boundaries.

- Core owns desired compute lifecycle; Runner performs it. `finite-agentd`
  accepts only typed, authorized agent-local actions, delegating product behavior
  to independent tools. Product permissions belong to Sites and Brain, not a
  global human-agent Principal Link.
- Runtime Management Pipe v1 is outbound health/release telemetry only.
  Product features, credentials and skills controls stay in their owners.
- Preserve Finite Private limiter grants, issued keys, reservations and audit
  history. Other legacy bridges require an explicit compatibility contract,
  tests and a delete condition recorded in Linear. Prefer proven code; remove
  compatibility only after proving supported existing-state readers and recovery.
- Build and promote one canonical Runtime image; Docker, Kata and Phala prove
  the same digest. Keep Hermes and baseline toolchains pinned through the root
  flake. For image or bridge changes, read [service ownership](docs/service-dependencies.md)
  and [the image runbook](../infra/runbooks/runtime-image.md).
- Skills updates are explicit agent-owned actions. Core, Runner and Runtime
  Management Pipe never select, push or activate a skills revision.
- Preserve recovery material across compute retirement; data purge requires
  separate, retention-gated authorization. Describe the product as O1
  operator-minimized with audited Finite-assisted recovery; a TEE alone does
  not justify an operator-blind claim.
- For persistence work, read `crates/finite-saas-core/PERSISTENCE.md` and test
  against real Postgres. Pure structural changes preserve wire/SQL contracts.
  Core's `lib.rs` exports types, `api.rs` assembles routes and `store.rs` sets up
  storage; open the relevant child module for the implementation.
- For dashboard changes, read `apps/dashboard/AGENTS.md`.
- Core source and tests use external modules and bounded files. Run
  `just source-structure-check`; see `../docs/agents/source-structure.md`.
