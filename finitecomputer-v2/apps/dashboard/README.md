# finitecomputer-v2 dashboard

This app is the self-serve SaaS dashboard for Finite Computer v2.

Current intended scope:

- WorkOS login/logout
- Project and Agent Runtime creation
- Finite Private grant/status surfaces
- Agent Overview for the launched runtime
- BoxOne-parity web chat backed by a Finite Chat Hosted Web Device
- product-owned connection UX through focused services, stable APIs, and skills;
  never through Runtime Management Pipe feature commands
- Finite Sites publish/list/preview and Finite Brain product surfaces
- explicit issue/revoke UX for separate Sites and Brain Email Access
  Delegations; Brain also provisions Folder Key Grants to the agent npub
- Recovery Readiness, export, Runtime Retirement, and explicit Break-Glass
  Recovery disclosures
- signed-in access to the selected agent’s native Skills inventory; skill updates
  remain the explicit agent-local `finite skills sync` workflow

Out of scope for v2:

- OpenCode
- a dashboard-only chat transport outside Finite Chat
- legacy dashboard-managed Published Apps in place of Finite Sites
- `finitec publish`
- `finitec repo`
- host-local control-plane inspection or runtime shell/filesystem access
- product feature commands, feature-specific status, or skills desired state on
  the Runtime Management Pipe
- direct provider-volume deletion or a normal lifecycle button that performs
  Purge User Data
- a global "link my email to my agent" control or any flow that turns a product
  delegation into a Principal Link
- editing managed skill bodies, selecting arbitrary Git refs/URLs, uploading
  archives through Core, or treating GitHub `main` as the Runtime catalog

## Managed Skills Boundary

The canonical Runtime image bundles one tested Finite Skills baseline and copies
it once when a fresh agent initializes. The Skills page reads the selected
agent's `GET /api/skills?inventory=true` through the shared Core-authorized Hermes session.
It reads only the configured Hermes home/profile, on entry, agent change and
manual Refresh. It does not invoke inference, sync skills, enable hosted access,
or create a Core inventory/desired-state store. The enclosing agent sidebar
retains its existing independent Chat behavior; Skills does not depend on it.

The page displays native names, descriptions, categories and disabled state,
with search and an in-memory last-loaded snapshot. Request failures mark that
snapshot stale; access loss clears it. Account/organization/agent changes remount
the view and abort previous reads. The helper bounds requests to 15 seconds and
1 MiB, uses operation-local credentials, and retries native expiry once.
Dashboard rollback requires no data migration.

The canonical Hermes package applies a narrow patch to the pinned `29112bef`
route: opt-in inventory reads return `{inventory_version: 1, skills: [...]}`
with local/external and registered plugin metadata. The default route remains
unchanged for Hermes's existing editor/toggle UI. Older runtimes return the
legacy array; Finite shows an update-required state instead of a partial list.
The inventory invokes idempotent native plugin discovery, never forced reload.
Local/external edits follow Hermes's bounded scan cache; plugin metadata follows
the native registration lifecycle and requires the usual plugin reload/restart
after registration changes. Refresh does not reload tools or hooks.

FIN-87 remains gated on an appropriate FIN-57 runtime release and a signed-in
check of that deployed candidate. The prior 100-skill canary response proves
authenticated access only. Package contract tests cover authentication, legacy
list/toggle compatibility, profile isolation, plugin inclusion/filtering and
local refresh. No separate transport or Skills-only fleet rollout is required.

Existing agents update at their own pace through the explicit
`finite skills sync` command. This page does not poll, push, schedule, or report
automatic skill rollout status. Native authentication and Hermes's derived
scan/cache behavior remain owned by Hermes.

## Brain account boundary

Set `FC_BRAIN_UPSTREAM_URL` to the internal FiniteBrain origin. The dashboard
serves the first-party `/client` through its existing WorkOS gate; encrypted
Brain API operations still require their normal Nostr authorization. Do not
point this at an independently login-gated public URL or treat WorkOS as a
replacement for Brain Folder Key grants.

Hosted `/client` reaches its bounded Brain Identity Provider through
`POST /api/brain/identity-provider`. The route accepts only the versioned Brain
operation set from a server-sandboxed, opaque-origin `/client` frame. A genuine
iframe navigation receives a signed, expiring capability after WorkOS
verification. Each provider call also requires a short-lived proof for its
exact body, minted by the authenticated parent dashboard. The opaque frame
keeps the capability while the parent proves its WorkOS session is still live;
neither alone can invoke custody. Valid calls forward the bound WorkOS user plus
the public Brain origin to the internal Hosted Device.
`FC_HOSTED_WEB_DEVICE_URL` and
`FINITECHAT_HOSTED_API_TOKEN` must therefore be configured alongside
`FC_BRAIN_UPSTREAM_URL`. Logout or session expiry makes this bridge unavailable;
it never replaces Brain's Nostr authorization or Folder Key Grants.

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

For day-to-day web chat and recovery design, use the real dashboard UI with the
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

The fixture covers chat, the machine overview, restart/recovery presentation,
and Skyler’s Brain/Sites design previews. It sets the development-only
`NEXT_PUBLIC_FC_DESIGN_PREVIEWS=1`; ordinary dev servers and production keep those
unfinished routes disabled. `FC_WEB_DESIGN_SECOND_AGENT=1` adds Fern for agent
switching checks. The Skills browser regression intercepts owner/native replies
and runs with Chat unavailable; it does not prove production authorization.
Runtime-owned Stop and Connections behavior require the complete devfinity stack. The fixture and devfinity both
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
