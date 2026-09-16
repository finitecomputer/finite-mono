# Hosted Hermes through Caddy

This draft establishes Core-owned location discovery and a Caddy configuration
renderer for native `hermes serve`. It does **not** enable hosted access, issue
native connection tickets, change Agent startup, or publish production routes.
The execution plan and release gates live in
[FIN-39](https://linear.app/finitecomputer/issue/FIN-39/provide-hermes-web-authentication-and-desktop-connection-details)
and the Agent rollout manifest in
[FIN-57](https://linear.app/finitecomputer/issue/FIN-57/agent-rollout-r1-batch-pull-based-connections-and-hermes-access-into).

## Boundary

The intended connection path is:

```text
Dashboard -- authenticated discovery / authorization --> Core
Dashboard -- native Hermes HTTPS / WebSocket ----------> Caddy --> Hermes
```

The dashboard supplies a stable Core Runtime ID. It never selects a Runner,
constructs a Runner hostname, reads container inventory, chooses a port, or
calls a Runner management endpoint. Core resolves placement from its current
Runtime record and returns an opaque URL. A hostname appearing in that URL is
transport data, not a placement policy for the dashboard to interpret.

Caddy terminates HTTPS and proxies the complete native Hermes surface. It does
not implement user authorization, store connection intent, or own a second
routing registry. Its generated configuration can be discarded and rebuilt.
Native Hermes owns the protocol, authentication sessions and chat history.
Core will own account authorization and hosted-access intent; necessary
credential state is not duplicated in Caddy or the dashboard.

No dependency on Finite Chat control, Iroh, a new Python service, or the
WireGuard network is added by this draft. Existing chat and SimpleX startup are
unchanged.

## Location discovery implemented here

`GET /api/core/v1/me/runtimes/{runtime_id}/hosted-hermes-location` requires a
verified account that is both an internal admin and the current Project owner.
It accepts the stable Runtime ID only. Core's private deployment configuration
`FC_CORE_HOSTED_HERMES_ORIGINS_JSON` maps its source-host identifiers to HTTPS
origins. That map is never sent to the dashboard.

The response contains `runtimeId`, `baseUrl`, and `availability`, with
`Cache-Control: no-store`. A configured URL has the shape
`https://agents.lat3.finite.computer/runtimes/<runtime-id>/`. The origin and path
are opaque to clients. Missing origin configuration returns `not_configured`;
a configured location returns `unqualified`.

**A location is not an access grant or readiness report.** This API deliberately
cannot return `ready`, a password, a ticket, or an enabled status. Do not connect
a product enable button to it. Native auth and current-assignment application
must be qualified before the connection flow can issue a usable
WebSocket URL and single-use native ticket.

## Caddy renderer implemented here

`finite-saas-runner render-hosted-hermes-caddy --manifest <file>` writes Caddy JSON
to stdout. It does not start Caddy, discover containers, reserve ports, call
Core, or mutate running services. The manifest describes one HTTPS origin,
an explicit listener and private admin Unix socket, and the runtime-to-loopback
port projection. It is intended as disposable output of the future Runner
reconciler, not an operator-maintained route database.

For example, a derived manifest has this shape (these are configuration names
and synthetic identifiers, not live routing):

```json
{
  "public_origin": "https://agents.lat3.finite.computer",
  "listen": "0.0.0.0:443",
  "admin_socket": "/run/finite-hermes-caddy/admin.sock",
  "allowed_origins": ["https://finite.computer", "http://localhost:3000"],
  "routes": [{"runtime_id": "runtime_example", "host_port": 30000}]
}
```

The renderer validates input before emitting configuration. Native requests
under `/runtimes/<runtime-id>/` are forwarded to that runtime's loopback port;
the external prefix is removed and `X-Forwarded-Prefix` is set by the proxy.
Unknown routes are not sent to an arbitrary default upstream. A dedicated
Caddy instance keeps hosted-chat lifecycle changes separate from other sites.

`allowed_origins` is an explicit, canonical origin list (empty by default).
HTTPS is required except for explicitly named loopback development origins.
Caddy preserves the request Origin, replaces upstream CORS response headers,
and handles browser preflight for the native route surface without a per-route
allowlist. Browser REST uses the native bearer with `credentials: "omit"`;
cross-origin cookie credentials are not enabled. CORS does not replace native
authentication: `/api/status` is public, while protected native reads must
reject anonymous and invalid credentials independently of origin.

The command alone is **not safe production publication**. No deployment module
or existing launch path invokes it in this draft.

## Address lifetime is a release gate

The real Caddy counterexample in `scripts/proofs/hosted-hermes-routing.mjs`
shows why route withdrawal is insufficient. A request already accepted on an
upstream keepalive connection can retry after withdrawal and reach a different
process that has acquired the old port, including the original credentials.
A passing new-request 404 check misses this failure.

The approved boundary is confirmed exit of the dedicated Caddy process before
an upstream address can be reassigned, followed by reconstruction from current
Core placement and owned container metadata. Existing stopped-container port
bindings remain reservations. Integrating that boundary requires all canonical,
upgrade, rollback and recovery paths, interrupted child commands and startup
reconciliation. Keep long guest-stop and readiness waits outside host-wide
Caddy downtime. The renderer does not yet implement that lifecycle boundary.

## Native auth and browser qualification

The pinned Hermes supports public username/password auth, native session
cookies and single-use WebSocket tickets. The ordinary dashboard must hide
native login, keep passwords and refresh tokens server-side, and request a
fresh ticket from Core for each connect/reconnect. A public proxy to Hermes's
ungated mode is not an authenticated service.

Disabling access must stop hosted connections once applied; an offline agent
remains pending. Ordinary disconnects should preserve accepted work. These
semantics require real process and accepted-turn tests, not just a connection
handshake or credential rotation.

A WS ticket does not authorize REST media/uploads. At the current Hermes pin,
REST CORS and WebSocket Origin checks also differ. An HTTP Upgrade request with
a supplied Origin header is useful protocol evidence but is not a browser
CORS test. Complete those checks against the actual dashboard origin before
claiming full chat support. Hermes Desktop setup is optional testing convenience.

**Before activation:** implement authenticated pull and truthful applied status,
existing-Agent credential delivery, native auth handoff, the integrated Kata
publication fence, real-browser acceptance, mixed-version/recovery tests and
DNS/TLS qualification. These require the explicitly reviewed R1 Agent capability
rollout, default off. No fleet rollout or production activation is part of this
PR. Dashboard iteration after that capability rollout should not require a new
Agent rollout for each UI change.

## Local component proof

See [the proof instructions](../../scripts/proofs/README.md) for the reproducible
Runner-rendered native TLS/auth test and the address-reuse negative control.
The tests distinguish Node protocol evidence from actual-browser qualification,
local temporary CA trust from deployed TLS, and retained draft sessions from
accepted model-turn durability.
