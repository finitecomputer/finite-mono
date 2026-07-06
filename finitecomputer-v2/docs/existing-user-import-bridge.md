# Existing User Import Bridge

Status: parked, but do not forget.

Date: 2026-07-03

## What Existed

The carried-over SaaS Core already has an import bridge for existing hosted
machines:

- operators reconcile host-owned records into `project_import_candidates`;
- records are keyed by `source_host_id` and `source_machine_id`;
- candidates are claimable by verified email;
- claiming creates a v2 Project, Agent Runtime, runtime link, and room
  membership without moving the underlying machine;
- a source-host relay endpoint can let Core/dashboard ask the legacy host for
  status or lifecycle actions.

The live v2 database proved this with the `smoke` source host. On 2026-07-03 it
still had four claimed `smoke` imports:

- `test@finite.vip` owned `smoke-studio`;
- `paul@finite.vip` owned `fire-finite`;
- `paul@finite.vip` owned `paul-smoke`;
- `paul@finite.vip` owned `paul-with-key`.

This is close to the SaaS story for existing box1/TRF users: they sign into the
dashboard, Core recognizes their verified email, shows claimable existing
machines, and can eventually offer a migration path to a new Phala runtime.

## Why It Is Parked

v2 launch should not depend on dashboard chat, legacy machine operations,
OpenCode, or dashboard-managed connection state. The simple launch path is:

1. WorkOS login.
2. Billing or operator grant.
3. Create a new Phala-backed agent.
4. Show the Finite Chat invite.
5. User talks to the agent from the iOS app.

Existing-user import is still valuable, but it is a migration bridge, not the
core product shape.

## Reuse Conditions

Only unpark this bridge when all of these are true:

- the imported machine is represented as read-only legacy state unless a
  specific migration action is being performed;
- Finite Private grants and API-key hashes are preserved;
- users can tell the difference between a legacy imported machine and a new
  Phala runtime;
- migration does not require dashboard chat;
- destructive actions have a backup/grace story;
- the source-host relay protocol is documented and tested against box1/TRF, not
  just `smoke`.

## Cleanup Rule

Deleting old smoke/self-serve test users must not delete Finite Private grants,
API-key hashes, reservations, usage counters, or audit events. If a Core user has
a `finite_private_grants` row, preserve the user row and relink it by verified
email when WorkOS creates a fresh user id.
