# ADR 0027: Daemon-local email proofs, no runtime Identity Authority

## Status

Accepted. Supersedes the Identity-Authority coupling in ADR 0020 (email
challenge/redeem via finite-identity, `satisfies-grant` consult in git-auth)
and the "fresh Identity Email Challenge" proof source in ADR 0026. Follows the
auth kernel (`docs/auth-kernel.md`): a product may only ever check a request
against its own tables, and email is how a capability reaches a human — never
an identity.

## Decision

All email proofs for Finite Sites are daemon-local. finitesitesd issues and
redeems its own 15-minute, single-use, hash-stored tokens
(`email_login_tokens`, delivered by the configured local `Mailer`) and never
calls another service at request time:

- `fsite auth login` / `auth link-email` / `auth sites-key request` ask
  finitesitesd (`POST /api/v1/email-auth/request`) for the challenge; the
  token arrives by email from Sites' own mailer.
- `fsite auth redeem` redeems at finitesitesd
  (`POST /api/v1/email-auth/redeem`), which records the mailbox-scoped Email
  Key and, when the signer is a registered native Principal, the Email Link.
- Sites Authorized Key register/revoke carry `{email, token}` directly and
  consume the daemon-local proof atomically with the mutation. The
  Identity-issued Mailbox Proof exchange is gone.
- Git-auth satisfies an email grant only from local rows: an active Email
  Link (`principal_email_links`) lets the linked native key mint a scoped
  credential via the verified-email path; Sites Authorized Keys and Email
  Keys keep their existing behavior; a revoked key record remains a
  tombstone that fails closed.
- The `first_publication` courtesy email and site access-request email are
  delivered by the local mailer. The identity notification relay
  (`IdentityNotifier`, `FINITE_IDENTITY_SITES_NOTIFICATION_TOKEN`) is
  removed; the `site_notification_outbox` drain is unchanged.
- The only remaining finite-identity call anywhere in Sites is NIP-05 name
  resolution (the directory). `finitesitesd serve` no longer reads
  `FINITE_IDENTITY_AUTHORITY` or `--identity-authority-url`. The operator-only
  `reconcile-identity` command named here as the one exception was a completed
  one-shot migration and has since been removed (technical-debt ledger item
  13); its Core cross-check endpoint no longer exists.

## Consequences

- A mailbox whose only proof lived at finite-identity (a VIP binding or
  Identity-side Principal Link, with no Sites-local key or link) must be
  re-proven once against the daemon; the emailed token is the same UX.
- `fsite auth redeem --link-native` no longer binds `@finite.vip` names at
  the Directory; name claiming is the Directory's own surface, not Sites'.
- `fsite auth git --email` tries this Finite Home's native key first and
  falls back to the mailbox-scoped Email Key on a 403, replacing the
  CLI-side `satisfies-grant` key selection.
- Deployments may keep exporting the old identity env vars; they are inert.
