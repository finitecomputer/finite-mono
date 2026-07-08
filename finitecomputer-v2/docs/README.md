# Documentation

- [Carry-over manifest](carry-over-manifest.md): what was copied from legacy
  finitecomputer and what must be cut.
- [Service dependencies](service-dependencies.md): separate repos and services
  that v2 deploys or integrates.
- [Legacy cleanup manifest](legacy-cleanup-manifest.md): what can be removed
  from the original finitecomputer repo after v2 owns it.
- [Finite stack deployment lanes](finite-stack-deployment.md): v2-owned deploy
  authority for Core/dashboard, hosted Finite Chat, runtime, and coordinated
  releases.
- [Hermes runtime test matrix](hermes-runtime-test-matrix.md): local,
  Docker, remote Docker, and Phala proof ladder for the real hosted-agent
  runtime.
- [Runtime control contract](runtime-control-contract.md): dashboard/Core
  lifecycle controls, runner operations, and known-good chat recovery.
- [Runtime recovery and observability plan](runtime-recovery-and-observability-plan.md):
  phased startup reports, plugin/config audits, recover-chat boot mode,
  rollback, break-glass export, logs policy, and recovery material.
- [Auth and key custody brief](auth-key-custody-brief.md): shareable decision
  brief for WorkOS account auth, `finite-auth`, Agent Root Secret, and user
  recovery key decisions.
- [Billing v0](billing-v0.md): Stripe Checkout, promo codes, Core
  entitlements, Finite Private limits, and destroy offboarding.
- [Existing user import bridge](existing-user-import-bridge.md): parked notes
  on the carried-over claim/import path for existing smoke/box1/TRF machines.
