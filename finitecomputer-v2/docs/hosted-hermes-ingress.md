# Hosted Hermes through Caddy

This implementation supplies the shared Core → agent → browser authenticated
connection on the PR #914 Caddy foundation. Account eligibility is current
Project ownership, not operator/admin status. It does not build feature pages,
cut over chat or roll the fleet. The production configuration selects Lat5 for
a bounded canary; configuration, deployment and per-agent enablement are separate.
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
port projection. The opt-in Runner reconciler uses the same renderer for disposable output;
this is not an operator-maintained route database.

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

The command alone is **not safe production publication**. The opt-in lifecycle
below supplies its ownership and process-lifetime preconditions. The Lat5 host
configuration enables this capability at `https://agents-lat5.finite.computer`,
with only `https://finite.computer` allowed as a browser origin. Other Runner
hosts keep it disabled. Core's host map contains only Lat5, and its dedicated
runtime router is proxied verbatim from `https://runtime-api.finite.computer`
to `127.0.0.1:4201`. This does not expose Core's private/account router.

The selected canary is Lat5 Canary Retry; per-agent access remains default off
and requires current-owner authorization and applied readiness. Host configuration
does not change the Runtime Artifact default or upgrade any agent. FIN-57 records
the deployed state, exact selected Runtime, immutable artifacts, recovery boundary
and activation gates. In particular, reconcile unrelated changes between the live
host and the proposed closure before activation; source configuration alone is
not evidence that the canary is deployed or qualified.

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
Caddy downtime. `hosted_hermes_lifecycle` supplies this boundary around Kata's
shared create/start/stop/rename/remove operations, including upgrade and recovery
helpers. Read-only provider commands and independent Borg recovery IO keep their
existing paths; unknown provider operations fail closed when ingress is enabled.

`GET /api/core/v1/hosted-hermes-route-targets` authenticates the Runner and
projects only that host's current, enabled, applied, completed assignments.
Runner joins those identifiers to canonical container ownership labels, the
runtime's writable `/data` bind, and current native listener readiness. It reads
saved ports through `nerdctl port` in every containerd namespace, including stopped
and foreign containers; stopped `inspect` output alone loses port bindings.
Hermes gets an explicit loopback port in 30000–31023, outside nerdctl's random
allocation range. This does not change the broader finite-agentd port allocator.

The host lock serializes Runner invocations. A marker written before each
provider mutation survives an interrupted command until a fresh, quiescent
systemd invocation confirms proxy exit. Runner's `ExitType=cgroup` and
`KillMode=control-group` prevent a delayed nerdctl/CNI child from overlapping the
next invocation; containerd-owned agents remain in their own service cgroups.
Before removal, Runner computes a successor projection without the retiring
container, confirms the old proxy's exit, and starts that successor **before**
provider removal. Core reads, inventory scans, listener probes, guest shutdown,
and slow removal therefore stay outside the host-wide stop/start interval.
A failed projection or unconfirmed process exit blocks address release.

`finite.hostedHermes.enable` enables the dedicated NixOS unit and the Runner
configuration file. Set `publicOrigin` to the exact Core host origin and set
`allowedOrigins` explicitly; `listenAddress`/`listenPort` configure the listener,
and the module opens that TCP port in the host firewall. DNS, host reachability,
certificates, and production Kata behavior still require deployment qualification.
`scripts/finite-status` reports hosted ingress separately on Runner hosts and
the opt-in module adds the dedicated unit to journal collection. Runner writes an
atomic diagnostic `status.json` in its private `/run` directory after reconciliation;
allocation, publication and recovery never read this record. The status reader
requires evidence less than 60 seconds old, matching actual proxy PID/invocation,
a stable observation and no unfinished mutation. It distinguishes not configured,
no eligible routes, serving, failed reconciliation, stale/missing evidence and
changed processes. Green is recent local routing evidence, not an end-to-end
account, native API or DNS/TLS availability claim. Older collectors must be
upgraded before activation; older evidence without the probe is unknown.
Caddy has no timer, automatic restart, resume, or boot activation. Only a fresh
Runner projection starts it. The pre-start gate compares the configured immutable
Runner executable with the last publisher: changing or rolling back the binary
requires confirmed Caddy exit before that Runner can mutate compute. An older
Runner cannot republish hosted ingress. Re-enabling a capable Runner reconstructs
routes from Core and containerd. Removing the opt-in NixOS module removes/stops
the dedicated service as part of system activation; do not roll back only unit
files by hand while leaving an independently managed proxy alive.

The disposable Linux proof in
`scripts/proofs/hosted-hermes-process-lifetime.py` invokes the actual Kata adapter
and lifecycle from an ignored Rust test executable. Build its tools with
`scripts/proofs/hosted-hermes-lifecycle-tools.nix`, build the Runner library tests
with `cargo test --locked -p finite-saas-runner --lib --no-run`, then supply
`--tools <tool-output> --runner-test-binary <test-executable> --disposable-fixture`.
It owns temporary systemd units, a separate containerd daemon, and synthetic data;
never run it on a fleet host. This qualifies process/address behavior with runc,
not the full Core lease/guest protocol or production x86 Kata/DNS/TLS rollout.

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
`enrolled:false` and cannot be enabled until enrollment during an authorized
Kata image upgrade, as described below.

### Existing-agent enrollment (FIN-90)

An opted-in Runner requests
`POST /api/core/v1/runtime-control-requests/{request_id}/runtime-credential`
before invoking the Kata upgrade adapter. Core requires that Runner's current,
unexpired upgrade lease and exact host/machine/Project binding, an active link,
and the unique running primary creation record with the current owner.
The primary record (`relocation_spec IS NULL`, unique per Project) is a stable
origin reference; it is not placement authority. This does
not authorize an upgrade itself or enable native serving. No account API,
RuntimeSpec, health report or status command exposes the bootstrap secret.

Core inserts into the existing credential table with serving disabled. An
exact retry, including a replacement worker's live lease, returns the same
secret without changing native credentials or applied generation. Revoked,
changed-owner, moved, inactive or ambiguous assignments fail closed without
repair. Historical relocation records are not candidates for that primary reference.
No date/ID ordering chooses an assignment. Missing or mismatched primary
records fail closed. Future relocation still revokes the old credential;
re-enrolling a revoked/reassigned identity requires the separate FIN-39
relocation/recovery contract, not an automatic repair here.

The Kata adapter carries the reserved pair in the existing transient private
environment file during the already-planned image upgrade. Both absent means
initial installation; both matching means replay. Partial, duplicate or
different values fail before compute replacement. A matching-image retry must
also prove the installed pair matches; image identity alone cannot acknowledge
delivery. No second restart, live guest file editing, new service or credential
authority is introduced. Local Hermes auth conflicts remain the existing
agentd readiness errors; enrollment never repairs those settings.

Ordinary restart/recovery and unconfigured Runner upgrades preserve installed
reserved variables. Automatic failed-upgrade rollback restores the previous
container and its previous environment; a newly enrolled Core row remains disabled/pending
until an agent applies settings. Retry redelivers the same secret. Missing
upgrade bootstrap support on an older Core (404), network failure, throttling
or 5xx leaves the operation retryable before any guest mutation. Old Runners
do not call the endpoint; old agents ignore the optional variables. Enrollment
does not prove that an old image implements native serving: applied readiness
remains required before any native session can be granted.

`scripts/finite-status --json` adds `fleet_convergence.hosted_enrollment` with
Core-recorded provider, artifact, matching creation/primary counts and credential state.
It reads no credential values. An older schema is reported as `schema_absent`;
guest configuration remains explicitly unknown until qualified. The supported
implementation cohort is Kata with one current matching primary creation and no
revoked/conflicting credential row; other providers and missing/conflicting primary history are
not silently included. No production enrollment occurs by running this probe.

The missing-bootstrap insertion is a compatibility bridge for pre-capability
agents. Retire that insertion branch when all supported active assignments
have Core bootstrap records, old images can no longer launch, and the existing
recovery path has proven that restored assignments carry their bootstrap.
Keep exact-assignment credential re-delivery/retry while upgrades can need it;
removing the bridge must not remove recovery material or introduce rotation.
Track that removal gate under FIN-39/FIN-57, not a separate preparatory rollout.

### Authoritative writers and readers

| State | Writer | Readers |
| --- | --- | --- |
| Assignment bootstrap | Core, on authenticated Runner creation or upgrade lease | Runner private launch/upgrade delivery; Core runtime authentication |
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
Existing enrollment adds no schema or history migration and preserves the same
credential authority and durable home. Keep the
Core database (including recoverable native/bootstrap material) in its existing
backup boundary; dropping that table is not a rollback procedure. Production
empty-target restore and integrated mixed-version rollout remain FIN-39/FIN-57
qualification work. Core database recovery must include the credential table;
the existing compute environment is needed to preserve its installed copy.
Never roll back by dropping the table or regenerating credentials on retry.

Rollback after activation must first apply hosted disable and withdraw ingress;
rolling back Core alone cannot stop already-running hosted clients. Full
address-reuse fencing remains mandatory before any production publication.
