# ADR 0002: Prove Plain Docker On Latitude Before Tinfoil

Date: 2026-06-30

Status: accepted

## Context

The requested target is "one of our Latitude boxes." Local SSH config exposes
two Latitude hosts:

- `lat1` / `finite-lat-1`
- `lat2` / `finite-lat-2`

The existing `smoke` host is `ovh-vps-smoke`, which is an OVH VPS, not a
Latitude host. `lat2` already has Docker active. `lat1` has k3s active and
Docker inactive.

Tinfoil may be a later privacy target, but it changes persistence, image,
attestation, public config, debug, and secret-handling constraints.

## Decision

Use `lat2` as the first target and prove the stack with plain Docker before any
Tinfoil packaging attempt.

## Consequences

- The first proof is fast, inspectable, and reversible.
- Docker logs, bind ports, and health checks remain easy to debug.
- Tinfoil work is deferred until we know the service set and resource needs.
- Any claim that the services are Tinfoil-ready must be supported by a separate
  follow-up ADR and smoke evidence.

