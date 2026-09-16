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

Validated Site hostnames select the fixed server-configured origin and request
field for `/internal/v1/viewer-sessions`:

- Legacy finite.chat and docs.finite.chat Sites use `FC_SITES_UPSTREAM_URL`
  with `output_url`.
- Static finite.site and v2.finite.chat Sites use `FC_SITES_V2_UPSTREAM_URL`
  with `site_url`. Explicitly enabled local development Sites use this setting.

Requests cannot supply an upstream; v2 failures never retry against legacy.
Hosted Chat requester assertions use `FC_SITES_V2_UPSTREAM_URL`: issuance
writes a token hash into the publishing registry, so a token from the legacy
registry cannot authorize Project Init on v2. Missing/failing v2 configuration
omits requester context without interrupting Chat; no fallback or redirect
may send the exchange to another origin. Deploy this change with the v2
publishing Runtime. Legacy viewer selection and request spelling remain until
their retained consumers are retired.

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
source restore and live account/guest access using the
[cutover runbook](../../../infra/runbooks/deploy-sites.md).

Changed account emails, aliases, key migration, and old-site redirects are not
handled by this bridge.
