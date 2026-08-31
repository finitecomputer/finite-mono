# Documentation

- [Carry-over manifest](carry-over-manifest.md): what was copied from legacy
  finitecomputer and what must be cut.
- [Service dependencies](service-dependencies.md): separate repos and services
  that v2 deploys or integrates.
- [Finite stack deployment lanes](finite-stack-deployment.md): v2-owned deploy
  authority for Core/dashboard, hosted Finite Chat, runtime, and coordinated
  releases.
- [Hermes runtime test matrix](hermes-runtime-test-matrix.md): local,
  Docker, Kata, and Phala proof ladder for the real hosted-agent runtime.
- [Runtime control contract](runtime-control-contract.md): generic
  dashboard/Core lifecycle controls and Runner operations.
- [Runtime Management Contract v1](runtime-management-contract-v1.md): the
  narrow outbound Runtime→Core health/release telemetry boundary.
- [Finite managed-skills delivery contract](../../finite-skills/docs/runtime-delivery-contract.md):
  one editable source, fresh-agent bundled availability, future explicit
  `finite skills sync`, and user-skill isolation.
- [Runner Contract v1](runner-contract-v1.md): Core-selected placement,
  provider-neutral lifecycle, Kata-first and Phala-fast-follow conformance.
- [Runtime recovery and observability plan](runtime-recovery-and-observability-plan.md):
  deferred Recovery Snapshot/key-backup TODOs plus startup reports,
  recover-chat, rollback, Break-Glass Recovery, and logs policy.
- [Identity Boundary v1](identity-boundary-v1.md): active separation of WorkOS
  Account Auth, human Finite Chat identity, Devices, and per-agent Finite
  Identity keys.
- [Stripe billing runbook](../../infra/runbooks/stripe-billing.md): live Stripe
  readiness, webhook/Core reconciliation, dunning, cancellation/refund, and
  secret rotation.
