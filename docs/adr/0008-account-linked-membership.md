# ADR 0008: Account-Linked Key membership — Core issues, products verify, mutations gate

Status: proposed, 2026-08-31 (grill session; implementation not started).

## Context

After the auth-kernel cut, every product authenticates each request
cryptographically (NIP-98 over method, URL, body hash), but principals are
permissionless: anyone can mint unlimited npubs and self-register
(`fsite auth register` is open; chat additionally runs with
`FINITECHAT_REQUIRE_SIGNED_REQUESTS=false` and public sync/blob routes). Keys
are free, so request authentication gives no sybil resistance — the exposure
Paul named as "unauthenticated CLIs" is really *unverified principals*.

We need anti-sybil and DoS resistance without abandoning npub-native
operation, and without rebuilding the central grant-resolution authority the
auth kernel just deleted.

This ADR deliberately reintroduces a product→Core runtime dependency
immediately after the auth-kernel merge removed all of them. The
reconciliation with the auth-kernel settlement ("a product may only ever
check a request against its own tables", Sites ADR 0027) is definitional:
**membership is a principal attribute, not a grant.** Products still resolve
*what a request may do* exclusively from their own tables; Core only answers
*whether the asking key belongs to the club*, through a cache with defined
failure semantics. One cross-service read, one shape, no grant ever crosses
that boundary.

## Decision

- **Membership anchor**: an npub is an **Account-Linked Key** when it resolves
  to one Account Auth identity (WorkOS account) through exactly one of:
  a root **Account Key Link** (dashboard ceremony, key-holder-initiated, one
  root user key per human), a **Key Attestation** by an Account-Linked Key,
  or a hosted Agent Principal Key through its Project (Runner-registered, no
  ceremony). Definitions live in `finitecomputer-v2/CONTEXT.md`.
- **Core is the single writer and sole resolver** of membership truth,
  serving read-only npub→membership resolution to products over HTTP — the
  same consumption pattern as identity's NIP-05 directory (identity ADRs
  0009, 0004): per-mutation checks through short-lived caches.
- **Attestations are exactly one hop.** An attested key never attests.
  Attesting a key you do not control is **Sponsorship**: capped per account,
  consumption attributed to the sponsor, sponsor npub carried as provenance
  in every resolution. No transitive web-of-trust.
- **Gate mutations, keep public reads open.** Unlinked, unsponsored keys are
  refused at registration/mutation surfaces (sites project init and publish,
  brain creation and redemption, chat account/room creation, blob upload).
  Public site serving and shared-site viewing flows stay as they are.
  Membership is the right to enter; each product's own grants decide the
  right to act (matches root ADR 0004: an Agent Principal Key gets no
  product access merely for belonging to an account).
- **Failure semantics**: Core unreachable → fail closed for unknown
  principals only. Cached members and existing grant-holders keep working,
  including writes. Core-down closes the front door, not the house.
- **Cut-off semantics**: revoking an Account Key Link (or a single
  Sponsorship edge) denies access and mutation within one cache TTL and
  never purges data (root ADR 0001 recoverability doctrine).
- **Chat hardening**: enforce `FINITECHAT_REQUIRE_SIGNED_REQUESTS` via a
  dual-accept → enforce-for-new-accounts → enforce-globally rollout that
  never bricks existing members mid-session; membership-gate room/account
  creation and blob upload; rate-limit the currently-unlimited `/sync/*`
  reads; MLS bootstrap routes (key-packages, pairing) stay open and
  rate-limited so admission never depends on the club.
- **Edge**: Cloudflare L3/4 absorption in front of chat, brain, identity,
  and finite.computer — pure proxy, zero filtering, origin locked to CF
  ingress. The edge still proxies and never filters; membership stops
  application-layer sybil, CF stops volumetric floods.

## Consequences

- Three products gain a runtime dependency on Core, bounded to one cache
  shape with fail-closed-for-unknowns-only semantics. New-principal growth
  halts during a Core outage; ongoing traffic does not.
- Revocations and account disables land platform-wide within one cache TTL,
  with sponsor provenance giving operators an account-shaped kill switch and
  a blame trail for abusive outsiders.
- Free verified accounts are members today; payment can later become a quota
  tier on the same anchor without redesign.
- If a root Account Key Link's key is lost, its attestations orphan until a
  fresh key is linked; this compounds the known identity key-loss recovery
  gap (identity ADR 0001) and belongs in its resolution.

## Rejected shapes

- **Identity Directory hosting the roster**: re-grows the names-only
  Directory into a grant authority and requires Core→Directory push
  machinery of exactly the kind the auth kernel deleted.
- **Push replication of the roster into each product**: the departure-facts
  pattern again — replay, compaction, mixed-version drift, three copies.
- **Transitive attestations (depth N)**: one lax sponsor mints an unbounded
  tree; blame dilutes geometrically; cut-off becomes graph surgery.
- **Payment-gated membership as the anchor**: kills casual/viral entry;
  billing belongs on quota tiers, not on the right to enter. Micropayment
  (lightning-credit) entry was likewise considered and deferred as
  off-direction.
- **Anonymous publish-then-claim** (the Cloudflare Pages pattern): storage
  exposure exists before the claim is made; sponsorship covers the viral
  case after the gate instead.
- **Full Cloudflare WAF/edge filtering**: violates "the edge proxies, never
  filters" and re-creates CLI/server contract skew at the edge.
