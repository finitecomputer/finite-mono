# ADR 0004: Protocol Surface Simplifications

Status: accepted 2026-06-11 (grill session against the core guarantee)

## Context

After the perf phase, six simplification candidates from code-level analysis
were stress-tested against the core guarantee — chat that always works:
durable, crash-atomic, exactly-once, membership-correct. Two of the six turned
out to be latent always-works defects, not just complexity. A standing posture
was also set: **Marmot interop is kept only when it is free.** We upstream our
transport contract when it costs nothing; we never bend the product surface to
preserve interop.

## Decisions

1. **Fanout checkpoint surface deleted.** The seven `/fanouts/*` routes, the
   `http_fanout_plans` table, the CLI commands, and their tests go. The
   production worker never called them: resume/retry safety lives in the
   client's durable `LinkFanoutState` plus idempotent `/commits`, proven by
   the response-loss and reprepare tests. Lost-local-state mid-fanout is the
   recovery-via-relink case (ADR 0003 §7), which starts a fresh fanout.
2. **Typed rooms accept only typed routes.** The raw `/messages` route leaves
   the product surface entirely (today, application-shaped raw payloads
   bypass membership and revocation checks — an injection door into typed
   room logs). The raw publish remains an internal state method so the
   upstream conformance suite still proves the durable server; the route, its
   CLI commands, and the Marmot-engine-over-our-routes interop test (with its
   heavy simulator dev-dependency) are dropped under the interop-only-if-free
   posture.
3. **One typed event route, delivery policy required.** `/events` and
   `/application-events` merge; every send carries an explicit
   `ApplicationDeliveryPolicy`. This also fixes a real bug: the production
   client used the no-effects route, so push/unread counts were never
   recorded for real sends.
4. **Direct rooms dissolved as a server concept.** The `/direct-rooms` route,
   stored account-pair state, third-account rejection, and the separate
   direct-room device cap are deleted. A DM is an ordinary room with two
   accounts; multiple named pair rooms are a feature (per-topic rooms with a
   person or agent), so server-enforced pair uniqueness protected a
   preference we do not hold. Lanes within a room remain **Topics**
   (CONTEXT.md). Membership authority stays out of the relay: any active
   member may create invite sessions and submit structurally valid membership
   commits. Product surfaces may choose how to present people/contact controls,
   but the server must not make itself the room authority.
5. **Scoped idempotency capacity rule deleted now.** The
   `MAX_IDEMPOTENCY_RECORDS_PER_ROOM_DEVICE` check permanently blocked a
   sender after 4,096 lifetime messages in a room (records never expire) — a
   landmine aimed exactly at long chats — and required the wrapper to
   deserialize typed payloads to derive scope. Its memory protection was
   cosmetic (growth is unbounded across rooms/senders regardless). The real
   bound is Phase E horizon expiry (perf-plan), which is time-based and
   typed-knowledge-free.
6. **Welcome lifecycle is claim + activate.** The `activated: false` terminal
   failure state is deleted: a failed activation simply stays pending
   (pending-cannot-send is already enforced) and is retryable; a transient
   local failure no longer permanently bricks a device's join. The
   Welcome-release coupling (no Welcome before durable accepted commit) is
   untouched and remains sacrosanct.

## Consequences

Positive: five route families and two projection states leave the product
surface; one rejection family and one terminal state disappear; two
always-works defects are fixed by deletion; the publish wrapper stops peeking
inside typed payloads; the conformance suite is unaffected.

Negative / accepted: no server-resumable fanout for devices that lose local
state mid-link (relink instead); duplicate pair rooms can exist when clients
race (acceptable — multiple pair rooms are legal anyway); a declined
invitation is not remembered (a future decline feature would be modeled as
removal, not welcome state).

Execution order (each step keeps the suite green): 5 (delete capacity rule),
3 (merge event routes + client policy plumbing), 6 (drop failed-ack state),
2 (gate + remove raw route surface), 4 (dissolve direct rooms), 1 (delete
fanout surface). The relay authority boundary (ADR 0003 §2, as amended here)
should land before or with step 4, since dissolved direct rooms rely on
active-member relay invariants rather than server-enforced room admins.
