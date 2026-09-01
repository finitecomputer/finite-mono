# ADR 0028: Viewer auth is one gate vouch; Sites stops proving emails

Status: proposed, 2026-09-01. Revises the 2026-08-31 shape of this ADR
(internal email-assertion mint + 24h magic links), amends 0024
(verified-email viewer session exchange), 0025 (bounded viewer sessions),
and the viewing use of `0026-reusable-email-viewer-links.md` and ADR 0027's
`email_login_tokens`. Related: root ADR 0008 (membership remains the
actor/entry concern).

## Decision

Sites' auth concepts reduce to two, one per kind of participant:

- **Actors sign.** Every mutation — publish, git, share management — is
  NIP-98, unchanged.
- **Viewers gate.** A browser that hits a non-public output without a
  session is redirected to the deployment's **Auth Gate**, the human
  authenticates there (AuthKit/Google for Finite's hosted gate today), the
  gate redirects back with a signed **Vouch**, and finitesitesd verifies
  the vouch and sets its own host-scoped cookie. The gate cannot set the
  cookie (different domains); it vouches, Sites mints its own session.

The gate is a **contract, not a host**. Products are configured with a gate
origin and a gate public key; Finite's hosted deployment points at Finite's
gate, and a self-hosted deployment points at its own instance backed by any
IdP. Swapping or self-hosting the gate is configuration, never code, and no
product learns a vendor's name. This swappability is the reason to prefer
one Finite gate contract over per-product OIDC (chosen 2026-09-01 with
self-hosting as the deciding criterion).

The Vouch:

- names a **verified email attribute** (decision 2026-09-01) — Sites
  compares it to its own share rows and consults nothing else;
- is short-lived, single-use, and bound to the output origin it was issued
  for (a vouch for one site is not a passport to others);
- is verified offline against the pinned gate key — the same trust shape as
  NIP-98: a signature over a statement, no runtime call to the gate;
- is versioned so it can grow an npub claim later without Sites changing;
  until then, npub shares are satisfied by signing clients only.

What this deletes when implemented (the negative-diff inventory):

- the viewing mailer path — `/_finite/request-link`, emailed login tokens
  for viewing, per-email/per-IP link budgets;
- `/internal/v1/viewer-sessions` (zero callers today) and
  `/internal/v1/native-viewer-sessions` plus the public
  `/_finite/auth/native-session` route and its nonce anti-replay;
- the hosted-device `authorizeViewerSession` signer (RFC 0001 flow) and the
  dashboard's proof plumbing — the dashboard preview, direct URLs, and
  iOS/Electron webviews all use the one gate path;
- ADR 0027's daemon-local email proofs survive **only** for the CLI actor
  path (`fsite auth login`, `link-email`, sites-authorized-keys); the
  15-minute login-grade TTL stays mandatory there.

## Consequences

- Trust substitution, said out loud: a gate-vouched email means the gate's
  IdP vouches for the human controlling that address — Google's account
  security replaces our emailed-token secrecy. This is the accepted
  "Disneyland gate" decision; humans without Google authenticate at the
  gate by whatever backend it offers (AuthKit email OTP is the gate's
  business, not Sites').
- The viewer cookie keeps its existing semantics: a session key into the
  revocable share table, re-checked by `view_access` on every request.
- Sites' viewer identity stays split by design — email attribute for
  browsers, npub for signing clients — until vouches grow an npub claim.
- Not covered, deliberately: routing CLI actor proofs through the gate
  (device-flow) is a separate future decision; owners of agent-published
  sites rely on the auto-created share for the human's email/npub.
- This ADR records direction only; implementation sequencing is not decided.
