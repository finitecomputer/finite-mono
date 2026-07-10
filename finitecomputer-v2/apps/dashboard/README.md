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

## Run locally

```bash
cd apps/dashboard
npm install
npm run dev
```

Then open `http://localhost:3000`.

The app assumes the repo root is two directories above the app. If you ever run
it from a different filesystem layout, set:

```bash
FC_REPO_ROOT=/absolute/path/to/finitecomputer-v2
```

## Useful routes

- `/` landing page
- `/dashboard` self-serve dashboard
