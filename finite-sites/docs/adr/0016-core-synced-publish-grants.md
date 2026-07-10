# Core-Synced Publish Grants

Finite Sites publishing authorization is a local **publish grant cache** keyed
by Publishing Key pubkey. Self-registration is the v0 default source, the existing
operator allowlist is a manual override source, and Core becomes the future
paid-entitlement source. `finitesitesd` checks this local cache for Project
creation and Project Output allocation and never calls Core in the hot path for
serving site traffic.

The grant cache records:

- Publishing Key pubkey;
- source (`self`, `operator`, or `core`);
- optional note;
- optional expiry;
- grant/update/revoke timestamps.

Any active, unexpired grant for a pubkey allows that owner to create Project
Repositories and Project Outputs. Revoking all grants stops new Project Init
on the next request. Already-published outputs keep serving from the registry
and blob store even when Core is unavailable or a grant expires.

The deployed `allow`, `disallow`, and `allowed` operator commands remain as
compatibility surfaces. Internally they now mutate `operator` grants. Existing
`allowed_pubkeys` rows are copied into the grant cache on registry open, and an
operator revoke removes the legacy row so it cannot reappear on restart.

**Considered Options**

- Call Core on every publish mutation: simpler entitlement freshness, but it
  couples Sites availability to Core and makes Core a latency dependency for
  an otherwise local NIP-98 mutation.
- Keep only the operator allowlist: good enough for VIP onboarding, but it
  cannot express paid access, expiry, or future non-Finite `npub` payments.
- Local publish grant cache synced from Core: keeps Sites isolated, preserves
  already-published serving independence, supports operator comp grants and
  future Core-paid grants; chosen.
