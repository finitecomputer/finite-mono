# Productize Organization Brain invitations in the Product Client

## Parent

Parent PRD: #47

## What To Build

Add Organization Brain invitation controls to the Product Client so an admin can
invite, inspect, accept, and revoke smoke-member access from the browser
workflow without falling back to the development Smoke UI.

## Acceptance Criteria

- [x] The Product Client exposes organization invitation controls near the
  existing access/share controls.
- [x] A connected admin can create a Brain invitation for a target npub through
  the existing protected invitation route.
- [x] An invited user can inspect and accept an invitation code from the Product
  Client.
- [x] A connected admin can revoke a pending invitation from the Product Client.
- [x] The UI makes invitation status, accept path/code, and error states visible
  without requiring the development Smoke UI.
- [x] Focused JS tests cover invitation request building and route helpers;
  server asset coverage verifies the create, accept, and revoke controls are
  served by the Product Client.

## Blocked By

None - can start immediately.
