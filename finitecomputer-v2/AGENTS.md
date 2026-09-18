# Finite Computer v2 Agent Guide

This tree owns the self-serve SaaS product: Core, dashboard, Runner integration,
and Agent Runtime orchestration. Sibling products retain their own ownership.

Before editing here, read
[the v2 skill](../.agents/skills/finite-v2/SKILL.md) for product boundaries,
forbidden legacy surfaces, Runtime Management Pipe rules, and image ownership.
Load the root safety skill when changing persistence, compatibility, runtime
lifecycle, onboarding, or recovery.

- Core owns desired compute lifecycle; Runner performs it. `finite-agentd`
  accepts only typed, authorized agent-local actions.
- Runtime Management Pipe v1 is outbound health/release telemetry only.
  Product features, credentials, and skills controls stay in their owners.
- Preserve Finite Private limiter continuity. Other legacy bridges require an
  explicit compatibility contract, tests, and a delete condition.
- For persistence work, read `crates/finite-saas-core/PERSISTENCE.md` and test
  against real Postgres. Pure structural changes preserve wire/SQL contracts.
- For dashboard changes, read `apps/dashboard/AGENTS.md`.
- Core source and tests use external modules and bounded files. Run
  `just source-structure-check`; see `../docs/agents/source-structure.md`.
