# Gateway chat development

Run the real dashboard shell, sidebar, transcript and composer with hot reload,
using a Hermes `tui_gateway` endpoint for conversations. The fixture supplies
account and runtime navigation; conversation reads and writes go to the gateway
you configure. Choose a test agent or your own agent.

From this worktree's `finitecomputer-v2/apps/dashboard` directory, create the
ignored `.env.gateway.local` file (keep credentials out of commits):

```dotenv
HERMES_GATEWAY_WS_URL=wss://your-gateway.example/api/ws
HERMES_GATEWAY_TOKEN=your-connection-token
# Optional: defaults to 13485, separate from the normal design fixture.
FC_WEB_DESIGN_PORT=13485
```

Then run:

```sh
../../../scripts/with-dev-env pnpm gateway:dev
```

Open the gateway-chat URL printed at startup. React/TypeScript/CSS edits hot
reload. Restart the command when changing connection configuration. Ctrl-C stops
the dashboard and its fixture services; it does not stop or restart the agent.
The development dashboard binds to loopback. Its browser bundle includes your
chosen gateway credential, so do not publish this development build.

For a local Hermes gateway, use an isolated `HERMES_HOME` and launch the pinned
`nix run .#hermes-agent -- serve --port 9120` from the worktree root, with
`HERMES_DASHBOARD_SESSION_TOKEN` set in its environment. Configure the UI with
`ws://127.0.0.1:9120/api/ws` and the same token. Do not reuse the isolated home
for the normal Finite Chat gateway process.

Password-gated endpoints can use `HERMES_GATEWAY_USERNAME` and
`HERMES_GATEWAY_PASSWORD`. For cross-origin cookie login, set
`HERMES_GATEWAY_PROXY_TARGET` to the HTTPS origin and point
`HERMES_GATEWAY_WS_URL` to the local `/hermes-gateway/api/ws` rewrite. This proxy
is development-only. Static-token endpoints do not need it.

## Regression gate

```sh
../../../scripts/with-dev-env node --import tsx --test browser/hermes-gateway.browser.ts
../../../scripts/with-dev-env pnpm test
../../../scripts/with-dev-env pnpm typecheck
../../../scripts/with-dev-env pnpm lint
```

The browser test boots the real dashboard and replaces only the gateway wire.
It checks an empty gateway with a prefilled prompt, exactly one create/submit,
draft materialization while streaming, refusal with composer text retained,
and reconnect to the same persisted session. It uses no real agent credentials.
To reuse a running test fixture, set `GATEWAY_BROWSER_TEST_BASE_URL` (its gateway
URL must be `ws://127.0.0.1:19120/api/ws`).

## Release boundary

This adapter remains an additive development surface. It does not migrate or
rewrite Finite Chat history. Hosted gateway enablement and admin-only connection
issuance are a separate PR. Tool approval/clarification/password responses,
attachments, history pagination, session-list pagination, and multi-client turn
reconciliation still need implementation and protocol-backed testing before
this UI replaces the shipped chat surface. Unsupported actions return an error.
