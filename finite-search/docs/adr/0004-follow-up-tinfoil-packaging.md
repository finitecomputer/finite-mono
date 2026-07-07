# ADR 0004: Treat Tinfoil As A Follow-Up Packaging Target

Date: 2026-06-30

Status: accepted

## Context

Tinfoil Containers are attractive for privacy and attestation, but their model
is different from ordinary Docker operations. Prior Finite Tinfoil notes call
out public config, pinned images, no persistent disk assumptions, secret
handling concerns, and restore/backup questions.

SearXNG is relatively small. Firecrawl is heavier because it involves an API,
browser automation, Redis, queue components, and optional Postgres/FoundationDB
pieces.

## Decision

Do not make Tinfoil the first deploy target. First prove normal Docker on
Latitude. Then evaluate which subset of the stack is a good Tinfoil candidate.

## Consequences

- The first milestone can complete without attestation or enclave-specific
  state design.
- Tinfoil readiness remains an explicit follow-up, not an implied property.
- Before a Tinfoil deployment, image digests, public config, state model,
  secrets, and network exposure must be reviewed.

## 2026-07-01 Evaluation Update

After the live `lat2` Docker proof, SearXNG is still the best first Tinfoil
candidate. It is a small single-container service with low memory usage and
mostly stateless behavior.

Firecrawl should not be first. The working deployment uses API, Playwright,
Redis, RabbitMQ, Postgres, FoundationDB, queue workers, writable scratch, and
open web egress. Tinfoil supports multi-container configs, healthchecks, and
restart policies, but its public-inbound/no-persistent-disk model needs a
separate design before Firecrawl can be called ready.

See `docs/tinfoil-evaluation-2026-07-01.md` for the current evidence and next
prototype boundary.
