# ADR 0028: Membership ownership and verified-login-email viewer rules

Status: proposed, 2026-08-31. Amends 0024 (verified-email viewer session
exchange), 0025 (bounded viewer sessions), and the 15-minute login-token TTL
in both `0026-reusable-email-viewer-links.md` and `limits.rs`. Follows root
ADR 0008 (Account-Linked Key membership).

## Decision

Two viewer rules feed one handshake, and owners never see an email
challenge:

- **Ownership rule**: a Project Output is viewable, at any visibility, by
  the controlling account — resolved from the owner / Originating Publisher
  npub through Core's Account-Linked Key membership resolution (short-lived
  cache). Agent-built sites are covered automatically because the agent's
  npub is an Account-Linked Key of the human's account; no owner email needs
  to be recorded at publish time.
- **Login-email rule**: a verified dashboard login email equal to an email
  share satisfies that share directly, with no challenge round-trip. The
  dashboard asks finitesitesd's existing internal viewer-session endpoint
  (service token) and completes the existing native-viewer-link redirect to
  set the host-scoped viewer cookie.
- **Magic links** (logged-out outsiders): single-use stays; TTL rises from
  15 minutes to 24 hours; re-request via `/_finite/request-link` stays
  frictionless and rate-limited.
- **Viewer cookie**: extends from 7 days to a 30-day sliding window. This is
  safe because `serve_path` re-runs `view_access(site, cookie)` on every
  request — the cookie is a session key into the revocable share table, not
  a bearer grant; share revocation lands on the next request regardless of
  cookie age.
- **TTL scope rule**: the 15-minute, single-use, hash-stored login-grade TTL
  remains mandatory for any token that grants mutation or account powers.
  Only view-scoped tokens may carry the 24-hour TTL.

## Why 24h is deliberate

Industry norms for magic links are 5–30 minutes, but those links grant
account control. Sites' links grant view access to one shared output, are
single-use and hash-stored, are continuously re-authorized against the
revocable share table on every request, and are per-email/per-IP
rate-limited at mint. The compensating controls carry the security; do not
"fix" the TTL back down without revisiting this reasoning.

## Consequences

- Email remains delivery-plus-view-scope in Sites (Sites Email Principal,
  per 0026), consistent with the auth kernel: email never names an actor who
  mutates; ownership now resolves through membership instead of recorded
  emails.
- finitesitesd gains one cross-service read (Core membership resolution,
  cached, fail-closed-for-unknowns-only per root ADR 0008) alongside its
  existing NIP-05 name resolution.
- Existing 15-minute tokens issued before deployment simply expire; no
  migration state is introduced.
