# ADR 0029: Account Session Viewer Bridge

## Decision

The account dashboard proves the current WorkOS session's verified email to
Sites through `POST /internal/v1/viewer-sessions`, authenticated with a
server-only service credential. Sites issues a short-lived, single-use
redemption token for its host-scoped ExternalEmail Viewer Cookie. Direct HTML
visits and dashboard previews use the same account adapter without Hosted Chat
signing. Dashboard previews also require Core-confirmed access to the selected
Agent Runtime. Missing account evidence or an unavailable exchange returns
the visitor to the guest email challenge.
An unshared verified account reaches the existing Request Access page and can
try another email. Authentication never creates a share.

Sites checks publisher-email ownership, shares, and visibility on redemption
and every read. Native-only grants do not become email grants; existing NIP-98
clients retain their API.

## Configuration and coexistence

Set `FINITE_SITES_ACCOUNT_LOGIN_URL=https://finite.computer/site-auth` on Sites
only after the dashboard serves that route. Without it, direct visits retain
the email form.

`FINITE_SITES_VIEWER_SESSION_TOKEN` is the shared dashboard/Sites service
credential: exactly 64 lowercase hex characters, kept only in server
environments. An absent value disables the exchange endpoint.

`FC_SITES_UPSTREAM_URL` selects the single server-configured Sites origin for
`/internal/v1/viewer-sessions` (`site_url`) and Hosted Chat requester assertions.
Requests cannot select an upstream, and credential-bearing exchanges reject
redirects. Assertions are stored in the publishing registry; missing or failed
issuance omits optional requester context without interrupting Chat.

Previous finite.chat content hosts are navigation-only. The edge maps each
retained URL to its exact finite.site destination; the dashboard neither sends
credentials to the previous host nor guesses the destination from its name.
Unmapped hosts and retired auth/API/Git routes return 410. Viewing and publishing
use the same Finite Sites service at `finite.site`.

## Tokens and cookies

Account handoffs expire after 60 seconds and atomically consume a Site-bound
login-token row. Domain-separated hashes keep them distinct from reusable
15-minute email links. Issuance and outstanding tokens are bounded per Site/email.

Viewer routes share seven-day, host-scoped HttpOnly cookies: `SameSite=Lax` for
top-level visits and a separate `SameSite=None; Secure; Partitioned` cookie for
secure iframe access. Cookies do not cross domains or cache authorization.

Redemption redirects are `no-store` and `no-referrer`. URL validation constrains
handoffs to served Site origins and same-origin return paths. The explicit
email fallback cannot start another automatic account redirect.

## Compatibility and limits

The [old-writer fixture](../../crates/finitesitesd/tests/fixtures/legacy-email-v053/README.md)
tests persisted email access, cookies, and revocation across versions. Browser
tests use development account evidence, not live WorkOS. Qualify the actual
restored state and live account/guest access using the
[deployment and recovery runbook](../../../infra/runbooks/deploy-sites.md).

Changed account emails, aliases, key migration, and old-site redirects are not
handled by this bridge.
