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
- signed-in access to the Finite Skills catalog and guidance for the explicit
  agent-local `finite skills sync` workflow

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
it once when a fresh agent initializes. The dashboard catalog is informational:
it does not read Runtime files, claim which revision an existing agent has
installed, store a Core desired revision, or request activation through Runtime
Management.

Existing agents update at their own pace through the explicit
`finite skills sync` command. The dashboard may explain that workflow, but it
does not poll, push, schedule, or report automatic rollout status.

Current code still loads a local checkout or the old split repository's GitHub
`main` and hides the page from normal SaaS users. That is migration scaffolding,
not accepted product behavior.

## Brain account boundary

Set `FC_BRAIN_UPSTREAM_URL` to the internal FiniteBrain origin. The dashboard
serves the first-party `/client` through its existing WorkOS gate; encrypted
Brain API operations still require their normal Nostr authorization. Do not
point this at an independently login-gated public URL or treat WorkOS as a
replacement for Brain Folder Key grants.

## Sites account preview boundary

Set `FC_SITES_UPSTREAM_URL` to the internal Finite Sites origin and give the
dashboard and `finitesitesd` the same dedicated
`FINITE_SITES_VIEWER_SESSION_TOKEN`. The dashboard may exchange a signed-in,
verified account email for Sites' existing one-time viewer link only after
Core confirms that account can access the selected Agent Runtime. Sites still
owns the share list and viewer cookie: the exchange never adds a share, and
removing the email from the output revokes the cookie on the next request.

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
`http://127.0.0.1:13002/dashboard/machines/skyler-fixture/chat`. Conversation
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

For environment-backed Core or WorkOS development, run `npm run dev` from this
directory and open `http://localhost:3000`.

Before handing off web changes, run `just web-check` from the repository root.
It performs the locked dashboard install, unit tests, lint, and production
build.

The app assumes the repo root is two directories above the app. If you ever run
it from a different filesystem layout, set:

```bash
FC_REPO_ROOT=/absolute/path/to/finitecomputer-v2
```

## Useful routes

- `/` landing page
- `/dashboard` self-serve dashboard
