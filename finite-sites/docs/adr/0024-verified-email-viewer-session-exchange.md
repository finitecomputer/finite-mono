# Verified Email Viewer Sessions Reuse Sites Magic Links

The SaaS dashboard previews Finite Sites in an iframe. A signed-in account may
already be on an output's email Share list, but an iframe cannot complete the
ordinary email inbox flow conveniently. WorkOS proves the dashboard account's
email; it does not own Sites grants or Sites viewer sessions.

## Decision

- Finite Sites exposes `POST /internal/v1/viewer-sessions` on its API plane.
  The endpoint is disabled unless `FINITE_SITES_VIEWER_SESSION_TOKEN` is set
  to exactly 32 random bytes encoded as 64 lowercase hex characters. It
  accepts only that dedicated Bearer credential, compared in constant time.
  The credential is shared only by Sites and the dashboard server, is never
  logged, and can be generated with `openssl rand -hex 32`.
- The request contains an exact canonical Finite output root URL, a verified
  email, and a bounded same-origin `return_to` path. Sites resolves the output
  through its own registry and rejects external, noncanonical, API, Git, and
  multi-label hosts.
- The endpoint calls the existing `request_login_for_site` operation. It
  succeeds only for a published `shared` output when the normalized email is
  already on that output's Share list. It never adds a Share or changes
  Visibility.
- The response is the existing 15-minute, single-use Magic Link. No new
  durable session, account-to-Sites principal link, or dashboard-owned cookie
  scheme is introduced. Redeeming it sets Sites' existing Viewer Cookie and
  redirects to the validated `return_to` path.
- Viewer-session issuance has a bounded per-output/email in-memory budget.
  Durable login-token rows are pruned when consumed or expired, and at most
  eight simultaneously redeemable links are retained for one output/email.
  This permits reloads and a few concurrent tabs without turning token state
  into an unbounded database or memory surface.
- Secure deployments also emit a second `__Host-` Viewer Cookie with the
  `Partitioned` attribute. The ordinary cookie is `SameSite=Lax; Secure` for
  top-level links; only the partitioned twin is `SameSite=None; Secure;
  Partitioned` for dashboard iframe storage. Both carry the same signed value
  and logout emits expiry headers for both names.
- `view_access` continues to check the Share table on every request. Removing
  the email therefore revokes both cookies immediately.

## Dashboard boundary

Before calling Sites, the dashboard derives the verified email from its
WorkOS session and asks Core whether that account can access the selected
Agent Runtime. It validates the preview URL, calls Sites server-to-server,
and gives only the one-use redeem URL to the iframe. Copy and open actions keep
using the original output URL. The Sites service token never enters client
JavaScript, Runtime state, chat, or logs.

## Consequences

This is narrow product composition: account auth supplies a verified fact,
while Sites remains authoritative for output identity, sharing, token minting,
cookies, and revocation. A dashboard outage does not alter Sites access, and a
Sites deployment can evolve its viewer implementation without adding feature
commands to an Agent Runtime or runtime-management image.
