# Reusable Email Viewer Links

Email viewer invitations are addressed to both a person and their agent. A
single-use Magic Link is easy for either participant, a mail scanner, or a
preview to consume before the intended browser establishes its Viewer Cookie.

## Decision

- Email viewer Magic Links may be redeemed any number of times until their
  existing 15-minute expiry.
- Every redemption still reads the stored token, verifies its expiry, and
  mints the existing host-scoped Viewer Cookie.
- Serving continues to check the current Share on every request, so removing
  the email Share immediately revokes access granted through any redemption.
- Issuance limits and per-output/email durable token bounds remain unchanged.
- Publishing email-verification tokens and Native Viewer Session tokens remain
  single-use. This decision changes only email viewer Magic Links.
- Login-token rows consumed before this change remain invalid. The existing
  `used_at` column is retained for upgrade compatibility; new redemptions do
  not set it.

## Consequences

An agent can inspect or follow a viewer invitation without burning the link for
the person who received it. Reuse does not extend a link's lifetime, create a
Share, bypass revocation, or widen the token to another Project Output.
