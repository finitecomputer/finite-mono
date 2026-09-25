---
name: finite-safety
description: Trace Finite compatibility, onboarding, routing, and recovery contracts when changing persisted state, protocols, identities, Agent Runtime lifecycle, public HTTP routes, edge routing, or deployment topology, or investigating a production repair.
---

# Finite change contracts

Read the owning code and tests, relevant retained component contracts, and
`docs/monorepo-doctrine.md`. Follow the root `AGENTS.md` for the GitHub–Linear
boundary; keep new plans and decisions in Linear.

## Chat and onboarding

Chat availability and durable history are the primary product promise. Trace
the production through-line: name each writer and reader, retained state, and
the relevant existing-state and mixed-version edges. An all-candidate test is
not compatibility proof. Prefer fewer authoritative paths; reject a design
whose compatibility and recovery contracts cannot be made clear and affordable.

Account enrollment, Agent admission, launch, identity readiness, binding, and
chat readiness are separate steps in the new-user promise. Prove the affected
end-to-end path. Runner drain and capacity are product availability state.

Services own their public routes in code. Expose one public router on a
dedicated listener and have Caddy proxy it verbatim; do not duplicate the
service contract in an edge route allowlist.

## Recovery and production repair

Follow `docs/adr/0001-recoverability-precedes-operator-blindness.md`. Do not
remove a Recovery Authority, couple compute teardown to data purge, or claim
stronger operator-blindness until the same Recovery Set restores onto an empty
target. A TEE and a Provider Durable Volume are not backups.

Before proposing a production migration or repair, gather read-only evidence,
reproduce the failure, prove the change on synthetic state, and identify the
backup and rollback boundary. Production mutation requires explicit user
authorization. Selection, sort order, or identifier order never authorizes
choosing or rewriting durable user state; ambiguity fails closed.

Use `scripts/finite-status` for platform/fleet status before and after every
rollout. Add missing incident probes there instead of retaining ad-hoc operator
queries. Inspect snapshot SQLite only with `scripts/snapshot-sqlite` or a
scratch copy. Deployment definitions and runbooks live in `infra/`; nothing is
built on a production host.
