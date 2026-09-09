# Hosted Hermes gateway — team preview

This opt-in feature lets an admin connect Hermes Desktop or the local chat UI
to an Agent they already own. It exposes the pinned Hermes API and WebSocket
protocol at `r-<runtime-id suffix>.agents.lat3.finite.computer`. Every HTTP route,
including HTML/assets, requires the Agent's gateway credential. Possessing that
credential grants the Hermes API's full Agent access.

The implementation is separate from chat UI PR #845. No host imports the new
Nix module yet; merging this change alone does not open ports, change DNS, or
activate hosted access. The dashboard controls remain hidden while
`FC_HOSTED_GATEWAY_RUNNER_DOMAINS` is unset.

## Ownership and request path

1. The admin dashboard checks fresh ordinary Project access and admin membership
   before issuing any command. It resolves the runtime's `source_host_id` through
   the existing admin Core API and a server-side hostname mapping. Core gains no
   gateway schema, token store, or public ingress endpoint.
2. The existing Hosted Web Device sends typed, authorized Finite Chat commands
   to `finite-agentd`. Enablement and the 256-bit random token are persisted
   atomically, mode `0600`, in `/data/agent/agentd/hosted-gateway.json`. Credentials
   appear only in the dedicated command response and no-store admin API, not
   general connection or fleet status. The command ledger retains responses
   under its existing per-Agent authorization/storage contract.
3. A dedicated supervised child runs pinned `hermes serve --isolated` on
   `127.0.0.1:9120` with the existing `HERMES_HOME`. The Finite Chat sidecar and
   regular Hermes gateway retain their existing supervision and identity.
4. HTTPS goes directly to the runner's Caddy, which proxies its entire public
   listener to a local unprivileged gateway service. There is no WireGuard hop
   and no Core request on this data path. The control plane still uses its
   existing infrastructure; this feature does not migrate that network.
5. A root helper derives a credential-free route projection from local nerdctl
   ownership labels, canonical container name, running state, exact writable
   `/data` mount, and public `agent/config.json` account ID. Recovery helpers,
   ambiguous matches, missing identities, and invalid addresses are omitted.
   This is a disposable projection, not a new runtime registry. It refreshes
   every two seconds and expires after fifteen seconds if refresh fails.
6. The proxy reaches the container's own port 8080 over the runner-local CNI
   bridge. The `/gateway/` ingress compares the attributed account ID with the
   local identity, authenticates the credential, then forwards to loopback.
   Health/contact routes keep their existing behavior. No unauthenticated HTML
   can expose Hermes' locally injected session token.

Lat3's existing nerdctl bridge uses `10.4.0.0/24`; the declared `finite` network
uses `10.89.0.0/16`. Both local subnets are accepted. Live Kata inspection can
report `unknown-eth0` instead of a network name; exactly one address in these
subnets is required. Overlay, public, and other addresses are refused. The
public service cannot read Agent state or access the containerd socket.

## Activation boundary

Production activation requires explicit authorization for the chosen runner,
a CI-built NixOS closure, and the canonical runtime image promotion flow.
Start with lat3 and one named team Agent; do not silently include lat4 or upgrade
all existing Agents.

- Provision DNS-only wildcard `*.agents.lat3.finite.computer` to lat3's public
  address. Do not proxy this through Cloudflare or point it at the app-plane
  host. Port 80 serves ACME validation and 443 serves TLS. Caddy uses
  [on-demand TLS](https://caddyserver.com/docs/automatic-https#on-demand-tls);
  its private `ask` endpoint permits only an exact, currently routable Agent
  hostname. No DNS API credential is needed on the runner.
- In the authorized runner's NixOS definition, import
  `../../modules/finite-agent-gateway.nix` and set
  `finite.agentGateway.domain = "agents.lat3.finite.computer"`. Build in CI,
  promote the exact closure, and follow the existing runner rollout procedure.
  Run `scripts/finite-status` before and after every rollout.
- Promote the canonical runtime image containing this change and upgrade only
  the selected team Agent through the existing runtime upgrade path. Preserve
  its entire Recovery Set and existing rollback image/closure. This change
  neither substitutes a data backup nor creates a new recovery authority.
- Set dashboard server environment `FC_HOSTED_GATEWAY_RUNNER_DOMAINS` to a JSON
  object mapping `finite-lat-3` to `agents.lat3.finite.computer`. Then deploy the
  dashboard through its existing lane. The variable contains no credential.
- On the Agent overview, expand **Advanced → Hosted gateway**, load controls,
  enable access, and check the connection once startup completes. Copy the
  WebSocket URL and session token separately. Never paste the token into an
  issue, PR, terminal command line, or public URL.
- Verify a real Desktop connection, list existing sessions, send a team test
  message, and check that ordinary Finite Chat and retained history still work.
  Disable access and verify the open gateway connection closes and the old
  credential is refused. Re-enable and verify a different credential is issued.

The local UI from #845 reads ignored `.env.gateway.local` values
`HERMES_GATEWAY_WS_URL` and `HERMES_GATEWAY_TOKEN`; run `pnpm gateway:dev` in that
PR's dashboard checkout for hot reload. A native client that wants the HTTP
server origin uses the same hostname with `https://` and no `/api/ws` suffix.
The admin status reports local Hermes readiness; public DNS/TLS and native
Desktop acceptance are rollout gates, not claims made by that status flag.

## Compatibility and rollback

Enablement defaults off on an existing volume with no gateway config. Old
runtime images reject the new command; the admin UI displays that error without
claiming enablement succeeded. New images keep the existing Finite Chat
protocol, identity, sidecar, and durable history paths. The isolated web process
uses the pinned Hermes session store; it does not bootstrap a second identity
or replace the normal chat gateway.

Disable clears the credential before restarting only the hosted child. New
requests are refused immediately; the command waits for the old process group
to terminate and its replacement to run, closing existing sockets. Re-enabling
mints a fresh credential. Old command-ledger responses can retain a revoked
token, which no longer grants access.

Before rolling the runtime image back, disable hosted access. Otherwise a
retained enabled config would reactivate access on a later upgrade. Removing
the dashboard environment hides the UI but does **not** revoke credentials;
disable each enabled Agent or stop the runner gateway service for emergency
network isolation. Rolling back the runner's closure removes this ingress
without deleting Agent data. Moving an Agent to another runner changes its
public connection address; clients must copy the new address.

This preview has no remote-agent discovery, custom DNS migration, or general
customer access UI. Gateway WebSocket frames are bounded at 32 MiB. Native
Desktop and external TLS acceptance require the explicitly authorized canary
rollout; they cannot be proven with a local fixture.

## Local validation

Use the repository's pinned environments:

- `scripts/with-dev-env cargo test -p finite-agentd --lib --locked`
- `scripts/with-dev-env cargo clippy -p finite-agentd --all-targets --locked -- -D warnings`
- In `nix develop .#hermes-bridge-ci`, run `$HERMES_AGENT_PYTHON -m unittest discover -s finite-agentd -p test_hosted_gateway.py` and `$HERMES_AGENT_PYTHON finite-agentd/smoke_hosted_gateway.py`.
- In the dashboard directory, use `../../../scripts/with-dev-env` with `pnpm test`,
  `pnpm lint`, `pnpm typecheck`, `pnpm build`, and
  `node --import tsx --test browser/agent-creation.browser.ts`.

The forwarding tests use real loopback HTTP/WebSocket servers, including an
immediate first frame, binary frames, credential rejection, identity mismatch,
stale inventory, deployed Kata interface metadata, and ambiguous ownership.
The smoke starts the actual pinned Hermes against an empty temporary home,
checks REST and `session.list`, refuses anonymous HTML, and checks captured
output for credential leakage. Browser coverage includes the real admin API,
non-admin denial, cross-origin rejection, unowned-runtime denial, startup,
disabling, and unsupported old-runtime responses.
