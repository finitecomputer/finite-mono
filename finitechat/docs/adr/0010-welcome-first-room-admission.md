# ADR 0010: Welcome-First Room Admission

Date: 2026-07-08

Status: Accepted

Supersedes the invite-session admission parts of ADR 0006 and the invite-session
parts of ADR 0007.

## Context

Finite Chat already has the protocol primitive we want: an active room member
claims a target device KeyPackage, submits an MLS Add Commit, and the delivery
service releases the corresponding Welcome only after the Commit is durably
ordered. The recipient joins by claiming and activating that Welcome.

The invite-code layer added a second admission protocol: create invite session,
submit join request, inviter polls/accepts, server tracks session/request state,
then the invited device finalizes through a special path. That layer made the
relay act like it held room admission state, multiplied version skew, and made
basic product flows fragile.

The relay must go back to being dumb: it orders encrypted room events, stores
KeyPackages and Welcomes, and exposes bounded sync. It does not own room social
authority, invite sessions, join approvals, or pending human workflow state.

## Decision

Room admission is only MLS Add plus Welcome.

- An existing active member adds a device by claiming a published KeyPackage
  and committing an MLS Add.
- A device becomes a room participant only after it claims, activates, and acks
  its Welcome.
- A non-joining invited device is just an unclaimed or unactivated Welcome. It
  must not poison the room, block future adds, or require server cleanup before
  the room can continue.
- Server-visible invite sessions, join requests, invite accept/reject states,
  invite status polling, and invite-specific sync hints are removed.
- Product "invite" UX may still exist later, but it must compile down to one
  of these primitives:
  - publish or refresh KeyPackages for a device/account;
  - share an account/device locator;
  - have an active member perform AddRoomMembers;
  - deliver or deep-link to an already released Welcome.

## Required Regression

The core protocol must prove this exact case:

1. Alice creates a room.
2. Bob has published a KeyPackage.
3. Alice adds Bob.
4. Bob never claims or activates the Welcome.
5. Carol has published a KeyPackage.
6. Alice adds Carol.
7. Carol claims and activates the Welcome.
8. Carol can send to the room and Alice can read it.

This is the contract that prevents stale invitation state from breaking future
room admission.

## Acceptance Criteria

- Core Rust tests cover the required regression above using the same runtime
  AppState/AppAction path the products use.
- Client/server route tests prove Welcome release is coupled to durable Commit
  acceptance and does not require invite-session state.
- CLI tests create a room, add a non-joining account/device, add a second
  account/device, and exchange messages.
- Daemon/Hermes tests use AddRoomMembers and Welcome activation, not invite
  create/join/respond endpoints.
- Electron can add an agent or human by npub/account locator when the target
  has KeyPackages available.
- iOS uses the same AppState/AppAction room/topic/chat model and the same
  add/welcome admission path.
- The server route table has no `/invites/*` admission endpoints and no durable
  invite-session table.
- Sync streams may watch both known rooms and the Device's own durable inbox.
  An inbox high-watermark hint carries no Welcome bytes or admission authority;
  it only wakes the same bounded claim, activate, ack, and ordered-pull repair
  path used after reconnect. Invite-session-specific hints remain deleted.

## Non-Goals

- This ADR does not define people/contact lists.
- This ADR does not require account-level push notifications for "please add
  me" requests.
- This ADR does not define a future pretty invite link format. A future link is
  acceptable only if it is a locator or Welcome delivery UX over this admission
  model, not a second admission protocol.

## Implementation Order

1. Add the core stale-add regression and keep the existing pending-commit sync
   regression.
2. Delete invite-session DTOs, server state, routes, durable storage, and sync
   hint watches.
3. Delete client/core AppState fields and actions that exist only for
   invite-session admission.
4. Replace CLI, daemon/Hermes, Electron, and iOS flows with AddRoomMembers and
   Welcome activation.
5. Run bottom-up tests: core, client/server route tests, CLI integration,
   daemon/Hermes, Electron, and iOS simulator.
