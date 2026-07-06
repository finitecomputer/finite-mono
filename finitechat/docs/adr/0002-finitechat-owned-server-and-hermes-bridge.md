# ADR 0002: Finite Chat Owns The Generic Server And Hermes Bridge

Status: accepted for v1 seed

## Context

The first finitecomputer chat integration proved that Hermes can talk through a
Finite-style relay, but the adapter lived in finitecomputer and spoke
finitecomputer-specific commands. That is the wrong long-term ownership line:
Finite Chat is intended to be a standalone open-source chat product, with
finitecomputer as the first consumer.

finitecomputer still needs product-specific behavior: hosted deployment,
dashboard projections, WorkOS login, private inference, hosted web routes, and
runtime command handlers. Those are product concerns, not core chat transport.

## Decision

Finite Chat owns the generic room server/relay contract, the durable protocol
types, the local store shape, and the Hermes platform bridge contract.

finitecomputer consumes Finite Chat in one of two ways:

- import the Rust crates directly for the first canary/hard cut;
- run the Finite Chat server or daemon side-by-side once the process boundary is
  worth the operational cost.

The generic Finite Chat server owns:

- server-ordered room logs and opaque MLS envelope storage;
- KeyPackage leasing, Welcome release, device liveness, and membership interval
  caches;
- SQLite as the default self-hostable store, with Postgres left as a future
  deployment option;
- encrypted attachment blob references and blob-store verification;
- generic durable application kinds, ephemeral activity, runtime command
  request/result DTOs, and runtime state snapshot DTOs;
- the `finitechat-hermes` JSON bridge contract used by Hermes adapters.

finitecomputer owns:

- account provisioning, WorkOS session handling, and hosted project lifecycle;
- hosted-runner admin such as route rendering, hostname reservation, auth
  policy, runner image changes, and emergency pod work;
- Finite Computer command handlers and runtime capability projections;
- dashboard and hosted web product UI;
- Electron/native app packaging decisions for the Finite Computer product.

The Hermes adapter is a Finite Chat integration. Python stays thin: translate
Hermes callbacks into `finitechat hermes` JSON requests, translate poll events
back into Hermes `MessageEvent`, and ack only after Hermes accepts the event.
Rust owns validation, cursoring, storage, encryption, attachment materialization,
and protocol projection.

## Consequences

Positive:

- finitecomputer can hard cut away from plaintext chat without defining a
  private fork of the protocol;
- self-hosted and bring-your-own-agent users get the same Finite Chat transport
  as hosted finitecomputer users;
- the Hermes plugin can be tested in this repo without a finitecomputer checkout;
- Electron and native clients can consume the same primitives as finitecomputer.

Negative:

- Finite Chat now owns more public API surface before the first shipped daemon;
- finitecomputer needs adapter code around product-specific command handlers
  instead of hiding them in the chat server;
- the first canary may embed crates before the standalone server process exists.

This is acceptable because the embedded API is daemon-shaped and has a clear
delete condition: extract a standalone server/daemon after the DTOs, sync loop,
command ledger, and store layout survive the finitecomputer hard cut.
