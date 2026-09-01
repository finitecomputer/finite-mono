# Account-Linked Key membership — requirements and decisions

2026-08-31 grill session (Paul + ZCode) over the auth/anti-sybil direction
for Finite Sites, FiniteBrain, and Finite Chat. Decisions are recorded in
root ADR 0008 and Sites ADR 0028; terminology in `finitecomputer-v2/CONTEXT.md`.

## Corrected threat model

The opening premise — "sites, brain, and chat CLIs are unauthenticated" — is
wrong in a useful way. `fsite` and `fbrain` sign every request with NIP-98
(method, URL, body hash, replay-cached, rate-limited). The actual exposures:

1. **Unverified principals** — anyone can mint unlimited npubs and
   self-register; request authentication provides zero sybil resistance.
2. **Chat's genuinely unsigned surface** — `FINITECHAT_REQUIRE_SIGNED_REQUESTS`
   defaults false; `/sync/*` and `/blobs/*` are public with no limits; blob
   upload is 32 MiB per anonymous request.
3. **No network-layer protection** — chat, brain, identity, finite.computer
   resolve straight to one origin box; only finite.chat is behind Cloudflare.
4. **The viewer pain** — 15-minute single-use magic links as the only path
   for owners to see their own sites.

## Converged requirements

- R1 Anti-sybil/DoS resistance on mutating and capacity-consuming surfaces,
  npub-native operation preserved (no login-per-CLI-action).
- R2 A Finite user's npub, their local agents' npubs, and their hosted
  agents' npubs are all covered by one account-level membership.
- R3 Outsiders shared with can join "the club with caveats": downstream of
  the whitelist, cut-off-able, with the inviting paid user identifiable.
- R4 A logged-in Finite user never re-authenticates or solves an email
  challenge to view their own site; a share to a user's login email is
   satisfied by that login.
- R5 Membership never confers product authority; per-product grants stay
  the only source of what a principal may do.
- R6 No user-data loss from any cut-off; no product bricked by any rollout
  step (Don't Break Chat; deny-never-purge).
- R7 Volumetric DoS on the origin is absorbed somewhere that is not the
  origin application.

## Decisions

| # | Question | Decision |
|---|----------|----------|
| 1 | What makes an npub a member? | Explicit link to a verified WorkOS account (**Account Key Link**); hosted agents via their Project. Free verified accounts are members; payment becomes quota tiers later. |
| 2 | Where does the roster live? | Core is the single writer, serving read-only membership resolution; products verify per-mutation through short-lived caches (identity-ADR-0009/0004 pattern). |
| 3 | What do unknowns get? | Mutations gated (registration, publish, brain creation, chat room/account/blob); public site reads and MLS bootstrap stay open. |
| 4a | Core unreachable? | Fail closed for unknown principals only; cached members and existing grant-holders keep working. |
| 4b | Cut-off speed? | Per-mutation checks, short cache TTL; denial lands everywhere within one TTL; never purges data. |
| 5 | Local agent coverage? | **Key Attestations** signed by the user's root key — one ceremony per human, no dashboard visit per device/agent. |
| 6 | Outsiders? | **Sponsorship** = one-hop attestation of an outside key; non-transitive, capped per account, consumption sponsor-attributed, sponsor npub carried as provenance. |
| 7 | Sites viewing? | Ownership via membership (works for agent-built sites) + verified login email satisfies email shares; magic links 24h single-use; viewer cookie 30-day sliding, re-checked per request; 15-min TTL reserved for mutation-grade tokens. |
| 8 | Brain? | Membership gates personal-brain creation and sponsorability (sponsorship auto-issues the existing `fbit-` invite token); every brain still requires its own grant; ADR-0046 provenance intact. |
| 9 | Chat? | Enforce signed requests via dual-accept → new-accounts → global; membership-gate room/account creation and blob upload; rate-limit `/sync/*`; key-packages/pairing stay open. |
| 10 | Edge? | Cloudflare pure L3/4 absorption on all four hostnames, origin locked to CF ingress, zero filtering; optional Turnstile on the two anonymous email-request endpoints. |

## Team transcript coverage (Meet bhp-zokb-rnt, 2026-08-31)

Every concern raised in the team call, and where it landed:

| Transcript concern | Where addressed |
|---|---|
| Anyone can create a site / store anything on our servers (Alex, Skyler) | D3 mutation gates + existing per-owner quotas |
| The agent must auth as a finite user (Alex) | D5 attestations; hosted agents auto-covered via Project |
| Sybil + DoS resistance is the hard requirement (Alex) | R1 — the whole design |
| "One enemy can take us down" (Alex, Skyler) | D10 CF absorption + D9 chat lockdown |
| Log in once, see it all — including from your phone (Skyler) | D7; the dashboard→site redirect handshake is device-agnostic |
| Non-Finite friend can view, and the viral "you could edit this" moment (Paul) | View: D7 email shares + magic links. Edit: D6 sponsorship (one attestation) + the product's own collaborator grant |
| "Once you're in, npubs can flow" (Skyler) | The layering rule; attestations never touch a browser |
| nsec as a second factor on the Google auth (Alex) | Same shape: account = entry factor, key signature = action factor |
| Derived keys from a root key (Paul) | Key Attestations give the identical one-root-many-children shape without HD-derivation machinery — children are ordinary independently-minted keys |
| No new thing users have to understand (Skyler) | No new user-visible credential: existing Google login + existing npubs |
| URL-driven scraping / endpoint spam / CORS scans (Alex) | D9 rate limits, D10 absorption; public reads expose only public content |

## Layering (the one-sentence version)

Membership is the right to **enter**; each product's grants are the right to
**act**; the edge absorbs packets and decides nothing.

## Open items

- Whether the Hosted Web Device can sign with the user's User Nostr
  Identity (enabling one-click dashboard approval of attestations) or only
  its own device key — flagged in `finitecomputer-v2/CONTEXT.md`; affects
  ceremony UX, not the model.
- Sponsorship cap default (N keys per account) and whether sponsored keys
  share the sponsor's quotas or get their own smaller tier.
- Root-key loss orphans all attestations beneath it until a fresh key is
  linked; compounds the known identity key-loss recovery gap (identity ADR
  0001) and should be addressed with it.
- Rollout sequencing proposal (each step independently revertable):
  1. Repo goes private (Alex had this staged before the fire; removes the
     read-the-source attack vector Alex and Skyler flagged) + CF absorption
     + origin locks (R7, zero product change)
  2. Core resolver + dashboard linking ceremony + sites viewer rules (R4)
  3. Sites mutation gates + brain creation gate (R1)
  4. Chat dual-accept window, then enforcement flip (R1, R6)
- The Hermes bridge stays loopback-bound and out of scope.
- Orgs (Paul: "orgs would be really nice"): WorkOS organizations already
  exist in Core's Account Auth. Membership is per-account today; org-scoped
  key pools, per-org sponsorship caps, and org-level kill switches are a
  follow-on that does not change the anchor.
- Anonymous publish-then-claim (the Cloudflare Pages pattern Paul raised) is
  consciously rejected: storage exposure exists before the claim. If viral
  signup ever needs it, it requires a quota-boxed unclaimed-output path —
  a separate decision.
- Micropayment / lightning-credit entry (Skyler's "sell sites as a service")
  is consciously deferred as off-direction per the team's call.
- Root-key linking ceremony UX: dashboard click vs an emailed code typed
  straight into the CLI (Paul hates browser pop-ups from CLIs; the model is
  identical either way). Expectation to set: first-run publish on a fresh
  machine requires the one ceremony; hosted agents skip it entirely.
- Values-level question to settle with Alex: his stated North Star is
  "eventually everything is authenticated." This design keeps public-site
  reads anonymous by design (publishing = the web). Deliberate divergence —
  confirm he accepts public reads as outside the gate.

## Production through-line (writers and readers of new state)

- Core writes: `account_key_links`, attestations (with sponsor provenance),
  project→agent bindings (existing). Core reads: WorkOS verification.
- Products read: membership resolution (cached, TTL minutes), never write it.
- finitesitesd reads additionally: nothing else new; still writes all shares
  and viewer sessions locally.
- Caddy/CF: ingress-only config; no route knowledge, unchanged doctrine.
