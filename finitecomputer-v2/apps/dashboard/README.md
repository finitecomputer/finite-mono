# finitecomputer-v2 dashboard

This app is the self-serve SaaS dashboard for Finite Computer v2.

Current intended scope:

- WorkOS login/logout
- Project and Agent Runtime creation
- Finite Private grant/status surfaces
- Agent Overview for the launched runtime
- Finite Chat invite display with no PIN
- Connections and Skills status only where the v2 runtime actually supports it

Out of scope for v2:

- dashboard chat
- OpenCode
- dashboard-managed Published Apps
- `finitec publish`
- `finitec repo`
- host-local control-plane inspection

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
