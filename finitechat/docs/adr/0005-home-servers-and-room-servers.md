# ADR 0005: Home Servers, Room Servers, and the Sharded Future

Status: accepted 2026-06-12 (target topology recorded ahead of implementation;
only the guardrails in §5 are near-term work)

## Context

The product vision: **each room can live on its own server.** A group stays
"centralized" to a single ordering authority — which is exactly what keeps
ADR 0001's guarantees — but the app as a whole depends on no single server,
and any group can self-host the server for its room. This ships as an
advanced setting first; eventually the invite code itself carries the room's
address.

Because the Finite Chat protocol is open source, we expect an ecosystem of
independent apps. Each app vendor will ship its own default **home server**,
primarily because push delivery requires vendor-held APNs/FCM credentials
that cannot be given to arbitrary self-hosted servers.

A code audit (2026-06-12) confirmed the architecture is already shaped for
this. The properties we built for reliability are the same ones sharding
needs:

- **The room is the unit of ordering.** There is no global sequence; every
  room has its own log, seq space, membership projection, and welcome
  lifecycle. A per-room server is just a server with one room in it.
- **Identity is self-certifying.** `AccountId` is derived from the account
  public key; devices prove themselves via MLS credentials signed by that
  key. A brand-new server can host a room containing accounts it has never
  seen — there is no registration authority to federate.
- **The server is untrusted for content and membership truth** (ADR 0001).
  A hostile self-hosted room server can at worst deny availability of that
  one room. Blast radius per server: one room.
- **Wake hints never advance state** (pull-based sync), so push delivery can
  be relayed through untrusted hops without correctness risk.

The same audit found the cross-room coupling that a sharded deployment must
account for. Six pieces of server state are account-resident rather than
room-scoped: the KeyPackage inventory, push tokens, the revoked-device
fast-block list, link sessions, device liveness, and the account-room
directory. And the client holds a single `base_url` — room records carry no
server identity.

## Decision: the target topology

Two server roles, one binary:

- **Room server** — hosts ordered room logs, membership projections,
  welcome claim/activate state, and publish idempotency for the rooms it
  hosts. Self-hostable by anyone; holds no push credentials and no account
  registry.
- **Home server** — hosts the account-resident state: KeyPackage inventory,
  push tokens, device-linking sessions, device liveness, fast-block
  revocation, the account room, and the account's room directory. Typically
  operated by the app vendor (it is the push gateway).

A device has exactly one home server at a time. The rooms it belongs to may
live anywhere. Today's deployment is the degenerate case: one server playing
both roles, hosting everything — nothing changes until the advanced setting
ships.

**Push is the one genuinely new piece.** Room servers cannot send platform
pushes (no vendor credentials, and device tokens must never be handed to
arbitrary servers). The pusher therefore lives at the home server, and room
servers forward wake hints to it. The wake payload is frozen at
`{room_id, seq}` — no content, no token — which is what makes relaying it
through a stranger's server acceptable.

**Invite codes become room addresses.** The eventual invite format is
`(room server URL, room id, welcome claim credentials)`; joining a room is
discovering where it lives.

## Principles that must stay true

1. **Home servers are replaceable.** If a home server goes out of business,
   the user migrates: identity is key-derived and survives; KeyPackages are
   client-generated and republishable; push tokens re-register; the account
   room is re-established on the new home (clients hold all state — the
   server log is for convergence, not archive). Room memberships are
   untouched by home migration, because membership is MLS state living on
   room servers. The standing rule: **no feature may make home-server state
   the only copy of something the user cannot regenerate.**
   - Honest caveat: discovery. Peers who only know your old home cannot
     fetch fresh KeyPackages until they learn your new address. Contact and
     invite flows must treat the home address as mutable data attached to an
     identity, never as the identity.
2. **Identity never becomes server-issued.** `AccountId` stays derived from
   the account key. No home server gains the power to mint, deny, or reuse
   an identity.
3. **Wake hints stay relay-safe.** `{room_id, seq}` only. Adding content,
   tokens, or anything state-advancing to the wake payload would break the
   relay model and the pull-based-sync guarantee at once.
4. **Apps interoperate through the protocol, not through a vendor.**
   Per-room `protocol_version` + `required_capabilities` (ADR 0003 §1) is
   the only interop mechanism; no app-specific behavior enters the protocol.
5. **The cross-room state list is closed.** Account-resident server state is
   exactly: KeyPackage inventory, push tokens, revocation fast-block, link
   sessions, device liveness, account-room directory, and invite sessions
   (added by ADR 0006; ephemeral with a TTL — the migration story is "print
   a new invite"). New server state must be room-scoped, or be explicitly
   added here as home-scoped *with a migration story*. This is a review-time
   rule starting now.
6. **Revocation degrades gracefully, by design.** Server-side revocation is
   a fast-block convenience at the home server; real enforcement is MLS
   removal commits, which are per-room and work on any room server. Nothing
   may come to depend on the fast-block being globally visible.

## Route taxonomy

Recorded so the split stays legible as routes are added. Room-scoped routes
move with the room; home-scoped routes are account-resident.

| Scope | Routes |
| --- | --- |
| Room server | `/commits`, `/events`, `/activities`, `/sync/group`, `/sync/inbox` (server-local device inbox), `/welcomes/claim`, `/welcomes/ack`, `/application-effects/*`, `/rooms/admins`, `/rooms/leave`, `/rooms/report-invalid-commit` |
| Home server | `/key-packages`, `/key-packages/invite-availability`, `/key-packages/claim`, `/key-packages/claims`, `/key-packages/inventory`, `/key-packages/leases/expire`, `/push-tokens`, `/push-tokens/remove`, `/devices/revoke`, `/devices/liveness`, `/devices/liveness/get`, `/link-sessions/*`, `/account-rooms`, `/account-rooms/bootstrap`, `/account-rooms/list` |
| Both | `/health` |

Note `/welcomes/claim` is device-addressed but server-local: in a sharded
world a device claims welcomes from the room server named in its invite.
KeyPackage claims are the inverse: an inviter claims from the *invitee's
home server*, then publishes the commit + Welcome bundle to the room server.
The KeyPackage routes are already room-free, which is what makes that a URL
change rather than a protocol change.

## Highest-impact work now

Ordered by leverage per cost. Only item 1 is real code; the rest are
shape-of-future-work rules.

1. **Stop deepening the single-URL assumption in the client.**
   `PersistedRoomState` and room bootstrap gain an optional room-server
   address (absent = home server), serde-defaulted so existing stores need
   no migration; the sync and fanout ticks group rooms by server before
   dispatching (a no-op grouping today). This is the one place where waiting
   makes the work strictly larger — every new client feature threads through
   the room record and the tick loops.
2. **Reserve the invite-code format with the address field first.** No
   invite codes exist in the wild yet. When the invite format is designed,
   it is versioned and `(server URL, room id, claim credentials)` is the v1
   shape, so room addressing never needs a flag day.
3. **Build the pusher daemon relay-shaped.** When the deferred pusher
   (ADR 0003 §5) is built, its internal interface is "wake hint in, platform
   push out" at the home server — even while room server and home server are
   the same process. Then sharding push is plumbing, not redesign.
4. **Hold the guardrails in review:** the closed cross-room state list (§5
   above), room-free KeyPackage routes, the frozen wake payload, and
   key-derived identity.

Deliberately **not** now: the server-to-server wake-forwarding API, home
migration tooling, room re-rooting (a dead room server is recovered by an
admin re-creating the room on a new server and re-inviting — clients hold
history), and peer home-server discovery. Each has its sketch above; none
has a driver until the advanced setting ships.

## Consequences

- Today's single server is reframed as "home server hosting all rooms" — a
  valid degenerate topology, so nothing ships or changes immediately.
- The client gains a small amount of structure (room → server mapping) ahead
  of need, in exchange for never having to migrate stored room records.
- Self-hosting becomes a per-group choice with a one-room blast radius,
  rather than an all-or-nothing app decision.
- The multi-vendor ecosystem story (open protocol, per-vendor home servers
  for push) costs nothing today beyond the principles in §5 staying true.
