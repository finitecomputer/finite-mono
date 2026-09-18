# finitecomputer-v2 dashboard

This app is the self-serve SaaS dashboard for Finite Computer v2.

The dashboard owns Account Auth, Agent creation and lifecycle controls,
Hosted Web Chat, connection setup, Sites previews and account administration.
Core owns account/runtime state; product services retain their own authorization.

The image supplies the Managed Skills Baseline. Existing agents update only
through explicit `finite skills sync`; the dashboard is not an updater or a
second Runtime filesystem/configuration store.

Brain invitation and approval routes use bounded server-side signed requests.
WorkOS authentication does not replace Brain membership or Folder Key Grants.
The removed `/client` browser UI is not a current dashboard surface.

## Sites account viewer boundary

Direct visits through `/site-auth` and dashboard previews share the verified
account-email exchange without Hosted Chat signing. Sites enables automatic
direct-visit handoff with
`FINITE_SITES_ACCOUNT_LOGIN_URL=https://finite.computer/site-auth` after this
dashboard route is available. Missing account evidence or an unavailable
exchange retains the guest email challenge; an unshared verified email can
request access or try another email.

`FC_SITES_UPSTREAM_URL` selects the single Sites registry for viewing and
Hosted Chat publishing assertions (`https://finite.site` in production).
Failed assertion issuance omits optional context so Chat remains available.
Previous finite.chat content links are navigation-only: the edge redirects
mapped hosts to their new Site, where Sites owns sign-in. The dashboard never
exchanges credentials against those hosts or guesses a destination name.
Give the dashboard and Sites the same dedicated
`FINITE_SITES_VIEWER_SESSION_TOKEN`. The dashboard may exchange a signed-in,
verified account email for a one-time viewer link. Dashboard previews also
require Core to confirm access to the selected Agent Runtime. Sites still
owns the share list and viewer cookie: the exchange never adds a share, and
revoking the email's Sites access takes effect on the next content request.
Token, cookie, and compatibility rules are defined in
[Sites ADR 0029](../../../finite-sites/docs/adr/0029-account-session-viewer-bridge.md).

The service token is server-only. It must not use a `NEXT_PUBLIC_` name, enter
a browser response, or be shared with an Agent Runtime.

Local `http://*.sites.localhost` previews are disabled by default. Local
development may set `FC_SITES_ALLOW_LOCAL_OUTPUTS=1`; production ignores that
flag so chat content cannot turn the dashboard into an iframe for a service on
the user's own machine.

## Run locally

For local web chat development, use the real dashboard UI with the
deterministic local Core and Hosted Device fixture:

```bash
cd finitecomputer-v2/apps/dashboard
npm ci
cd ../../..
just dev web-design
```

Open
`http://127.0.0.1:13002/dashboard/machines/runtime_web_design/chat`. Conversation
state survives stopping and restarting the command. In another terminal:

```bash
just dev web-design-state unavailable
just dev web-design-state recovering
just dev web-design-state healthy
just dev web-design-reset
```

These commands change only the local fixture under
`.local-state/web-design-fixture/`. They never contact a provider, Agent
Runtime, or production service. The fixture backs the canonical dashboard
components and routes; it is not a second UI and does not prove runtime
acceptance.

The fixture is intentionally scoped to chat, the machine overview, restart
presentation, and bounded recovery states. Runtime-owned Stop and Connections
behavior require the complete devfinity stack. The fixture and devfinity both
default to port 13002, so run only one at a time or choose another fixture port:

```bash
FC_WEB_DESIGN_PORT=13003 just dev web-design
```

For environment-backed Core or WorkOS development, run `npm run dev` from this
directory and open `http://localhost:3000`.

Before handing off web changes, run `just web-check` from the repository root.
It performs the locked dashboard install, unit tests, lint, and production
build. `npm run test:browser` adds the Chrome-backed browser regression suite
that CI runs; it requires a local Chrome installation.

The app assumes the repo root is two directories above the app. If you ever run
it from a different filesystem layout, set:

```bash
FC_REPO_ROOT=/absolute/path/to/finitecomputer-v2
```

## Useful routes

- `/` landing page
- `/dashboard` self-serve dashboard
- `/dashboard/machines/runtime_web_design/chat` durable design-fixture chat
- `/dev/sticker-sheet` development-only visual source of truth for dashboard
  tokens, typography, controls, and statuses
