# ADR 0004: Build A First-Party Product Client

Status: accepted

FiniteBrain Rust v1 will use a first-party browser Product Client served by the
Rust app/server as the primary trusted user workflow. The client may borrow
interaction ideas from the previous SilverBullet-based prototype, but it will
not embed or reuse SilverBullet as a compatibility target because this parity
run is a hard cut: the client should be shaped around the Rust API, NIP-07,
client-side decryption, local indexes, OKF flows, and working-tree projection.

## Considered Options

- Reuse or embed a SilverBullet-style editor surface.
- Build a first-party Rust-served browser app as the primary Product Client.

## Consequences

- The Smoke UI can remain a development harness, but it is not the product
  workflow.
- Product parity work can define client modules around FiniteBrain terms:
  Vault, Folder, Page, Folder Key Grant, Graph View, Replay, OKF, and Vault
  Working Tree.
- Legacy editor/runtime behavior is not a compatibility requirement.
