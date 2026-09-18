# ADR 0011: Hosted web chat uses a revocable device

Status: accepted 2026-07-09

Finite Computer's launch web-chat surface is a Finite-operated **Hosted Web
Device** with its own server-held device key and durable store, authorized
through Account Auth and presented honestly as trusted web chat rather than
browser E2EE. This preserves the proven dashboard experience without making
Electron or iOS a launch dependency, while Electron and future native clients
enroll as separate revocable Devices with local custody; no device or daemon is
room authority, and server/device restart recovery is a release-gated product
invariant.

This decision does not make WorkOS an account signer. Finite Chat device
credentials are signed by the human's Nostr account key, so launch still needs
an explicit first-login account-key bootstrap and custody contract for Hosted
Web enrollment, later Electron linking, and bring-your-own-nsec behavior. The
device key named above is not that account nsec; until the account-key contract
is accepted, Hosted Web enrollment remains an open launch dependency.

The Hosted Web Device is also part of the SaaS Recovery Set. O1 honestly treats
its Finite-operated process and restored store as potentially accessible during
audited Finite-assisted recovery. Store loss must restore a usable Device and
retained-history/export path; silently minting an unrelated chat account or
showing server ciphertext as recovered data is not acceptable.

ADR 0012 tightens the continuity rule: the Hosted Web Device also owns the
encrypted Project/Principal-to-canonical-Room binding. Navigation selection is
only a cursor, bootstrap opens an already-valid binding before Runtime contact,
and recovery cannot mint a replacement Room. The authenticated Project-creation
workflow writes a durable one-time bootstrap authorization; ordinary chat load,
restart, deploy, upgrade, and recovery cannot create it. That authorization
advances to a sealed staged journal: the exact Room create request and MLS group
id are durable before any server mutation, the claimed Agent KeyPackage is
durable before Room creation, and the exact prepared add-member commit is
durable before submit. A crash after server acceptance but before local MLS
group save replays only that exact Room request. Missing authorization or
ambiguous unbound retained state fails closed without automatic selection or
mutation. Durable protocol sync and reconnect processing for already-authorized
membership remain normal operation: they are not legacy migration and cannot
choose or repair a binding.

## Browser transcript views

Each dashboard tab reads its own transcript through the existing
`GET /v1/app/state` and `GET /v1/app/updates` routes. Optional `room_id`,
`topic_id`, `chat_id`, and `limit` query parameters select a read-only view of
the authenticated Device's existing projection. Selection and messages in
each snapshot describe the same view, including the first event after an SSE
reconnect. A view read never saves navigation, creates a binding, changes the
Device revision, or emits updates to other tabs. Explicit navigation actions
still save the Device's default cursor for the next initial load.

Requests without a view keep the existing selected-transcript contract:
state reads and the initial SSE event read the last published snapshot without
waiting for the runtime actor. Subsequent unscoped events retain the existing
update result and published-state fallback. Scoped reads wait for the actor to
finish any in-flight command before projecting a coherent view; the dashboard
keeps its last coherent transcript during that wait. No persisted data or
response schema changes. Deploy the Hosted Web Device before
the dashboard for independent live tab views. An older dashboard keeps using
unscoped snapshots; an older service ignores the query parameters, so the new
dashboard retains its last coherent transcript when that service returns a
different chat. Independent live updates require the scoped service. Either
component can roll back without a history or identity migration.
