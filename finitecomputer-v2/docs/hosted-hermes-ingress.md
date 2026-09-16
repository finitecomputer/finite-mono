# Hosted Hermes through Caddy

This branch implements the shared Core → agent → browser authenticated
connection on the PR #914 Caddy foundation. Account eligibility is current
Project ownership, not operator/admin status. It does not build feature pages,
cut over chat, publish production routes or roll the fleet.
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
Core owns account authorization and hosted-access intent; necessary
credential state is not duplicated in Caddy or the dashboard.

No dependency on Finite Chat control, Iroh, a new Python service, or the
WireGuard network is added by this draft. Existing chat and SimpleX startup are
preserved; native serving is an optional separately supervised child.

## Location discovery implemented here

`GET /api/core/v1/me/runtimes/{runtime_id}/hosted-hermes-location` requires a
verified account that is the current Project owner. Admins have no ownership bypass.
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
cross-origin cookie credentials are not enabled. This is a uniform browser-origin
policy on the complete native surface, not an edge endpoint allowlist. New browser
operations must qualify their methods and headers against that policy.
CORS does not replace native
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
native session grant from Core; a later WebSocket consumer obtains the native
one-use ticket with that grant. This slice does not implement chat reconnect UX. A public proxy to Hermes's
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

**Before activation:** qualify the integrated pull/status/auth path on existing
agents, implement existing-Agent credential delivery, finish the integrated Kata
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

## Shared access implementation

Account API (existing private Core listener, signed WorkOS identity):

- `GET /api/core/v1/me/runtimes/{id}/hosted-access`: safe enrollment/intent/application state.
- `PUT` at the same path: `{enabled, expectedGeneration}`. Only current owners
  can change intent. Repeating the same intent at the current generation is a
  no-op; stale writes return conflict. Default off is rollout state, not an
  admin role requirement.
- `POST /api/core/v1/me/runtimes/{id}/hosted-hermes-session`: real native
  password/cookie exchange, then `{baseUrl, accessToken, expiresAt}`. Core
  rechecks account identity and current assignment/intent after native IO.

The dashboard's same-origin `/api/agents/[runtimeId]/hermes-access` adapter
checks current machine access and request Origin for writes. It forwards the
signed account session to Core. It does not proxy agent product data.
`readHostedHermesJson` obtains an operation-local grant and performs a bounded
native GET; one 401 triggers one reauthorization/retry. No browser persistent
credential cache. A caller must abort pending operations on account/agent
change and ignore obsolete UI results.

Runtime API: setting `FC_CORE_RUNTIME_BIND` starts a **separate listener** with
only `GET /api/core/v1/runtime/hosted-hermes` and `POST .../report`. The edge must
proxy this listener whole. This change supplies no infrastructure activation.
The authenticated Runner's live creation lease provisions a scoped credential,
injected through reserved `FINITE_CORE_URL`/`FINITE_CORE_CREDENTIAL` names when
`FC_RUNNER_RUNTIME_CORE_URL` is configured. Existing unenrolled agents report
`enrolled:false` and cannot be enabled; upgrade enrollment is release follow-up.

### Authoritative writers and readers

| State | Writer | Readers |
| --- | --- | --- |
| Assignment bootstrap | Core, on authenticated Runner creation lease | Runner delivery; Core runtime authentication |
| Native enablement/credential generation | Core, after current-owner authorization | Assigned agent pull; Core native login |
| Applied generation/status | Assigned agent, after native auth or process exit | Core account state and session eligibility |
| Native session cookies | Hermes | Core's disposable memory cache |
| Short access token | Hermes; Core returns only the native access token | Browser's direct agent API requests |
| Routing configuration | Existing renderer from trusted projection | Caddy; production lifecycle integration remains separate |

Migration `0030_runtime_hosted_hermes.sql` adds one Core table. Credentials bind
creation, runtime, source host/machine and owner. Restart/stop-resume preserve
that assignment; revocation, relocation, owner change or inactive Project links
fail closed. This is not a generic feature-state store. Native signing/password
generations remain stable across process restart. Disable clears Core's native
material and immediately denies new grants; applied disable waits for child
exit. Re-enable creates new credentials and signing material.

Core keeps native access/refresh/provider cookies only in bounded memory,
serialized per runtime, keyed by current assignment/generation/location. Native
cookie middleware renews the session; Core never reproduces Hermes token
encoding. The browser receives a full native session for that agent, not a
read-only or conversation-scoped token. Existing bearer use can continue until
its native expiry (60 seconds). Core permits 30 seconds of server clock skew
when checking native expiry after a successful protected read; the browser never
uses its device wall clock as an authorization decision. Already-open sockets require applied process
shutdown. Core outage retains the last applied agent configuration and issues
no new grants. These bounds must remain visible in later revocation UX.

The optional agent child uses the same durable Hermes home without rewriting
configuration or chat history. Disabled/auth-conflicting plugins or a stored
password hash taking precedence over plaintext cause failed readiness, not
silent repair. The original daemon's bridge-readiness fatal deadline remains;
this implementation does not claim independence from every Finite Chat outage.

### Compatibility and rollback boundary

The new table is additive. Old launchers create no bootstrap row, old agents
ignore the new optional launch variables. An opted-in Runner hitting an older
Core without bootstrap support keeps the creation retryable and does not launch
or terminally fail it; activate Core support before opting in Runner. Unconfigured new components keep
the existing chat path. Existing credentials are not retroactively invented.
No new chat database, home copy or history migration is introduced. Keep the
Core database (including recoverable native/bootstrap material) in its existing
backup boundary; dropping that table is not a rollback procedure. Production
existing-agent delivery, empty-target restore and mixed-version rollout remain
FIN-57 qualification work.

Rollback after activation must first apply hosted disable and withdraw ingress;
rolling back Core alone cannot stop already-running hosted clients. Full
address-reuse fencing remains mandatory before any production publication.
