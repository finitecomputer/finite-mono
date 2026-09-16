# Browser access to agent APIs

The first dashboard slice reads native Hermes `GET /api/status`. Existing chat
stays in place. Agent-owned skills, Brain and Sites listings come next; Connections
and chat integration follow. This crate is transport, not a new chat server or a
Core-owned copy of agent data.

An admin opens an agent's **Advanced → Agent status (internal)** page. Hosted
access starts disabled. Core checks the signed account identity and authorizes a
fresh ephemeral browser peer for the registered runtime generation. Browser WASM
then sends HTTP over Iroh to the runtime's fixed Hermes loopback target. The
response goes directly back to the browser, without passing through Core.

A status read removes its admission and closes its peer when finished. If the tab
vanishes, runtime-enforced expiry is the fallback. Enabling access still grants
full native Hermes access to admitted peers; the status-only UI does not narrow
the existing authorization contract.

## Local development

From the dashboard directory, `pnpm dev:iroh` builds WASM with the pinned Nix
`browser-iroh` shell and starts the normal hot-reloading Next server. Rebuild WASM
after Rust changes with `nix develop .#browser-iroh --command scripts/build-browser-iroh`
from the repository root. TypeScript/UI changes hot-reload normally.

Generated JS/WASM lives in dashboard `public/iroh` and is not committed. CI and
the dashboard image workflow run the same build script. The image build fails if
the assets are absent. Root Cargo.lock is the sole Rust lockfile; wasm-bindgen's
crate version matches the CLI in the pinned Nix shell.

Use the existing dashboard account/Core configuration. The agent must already
have a Core credential and registered Iroh endpoint. Existing fleet enrollment is
still a separate draft-PR gate; this UI does not silently enroll an agent.

## Real acceptance

The ignored Core test `real_core_agentd_iroh_hermes_acceptance` starts actual Core
HTTP, isolated Postgres, the production `agentd iroh` child and native Hermes.
Set `FINITE_TEST_BROWSER_SCRIPT` to the absolute dashboard
`scripts/agent-status-acceptance.ts` path and `FINITE_TEST_DASHBOARD_DIR` to the
dashboard directory to include the real Next/Chromium/WASM leg. Install the
locked dashboard dependencies and build WASM first. The test also requires:

- `FC_CORE_POSTGRES_TEST_URL`: disposable local Postgres admin connection.
- `FINITE_TEST_AGENTD_BIN`: absolute built agentd executable.
- `FINITE_TEST_HERMES_BIN`: working packaged Hermes executable.
- Node/Chromium and outbound HTTPS relay access.

Run `cargo test --locked -p finite-saas-core real_core_agentd_iroh_hermes_acceptance -- --ignored --nocapture`.
Only WorkOS's signing key/user lookup uses the existing test identity fixture;
the proof exercises real JWT verification and production admission routes, not a
mock Core or simulated status response. It does not prove live WorkOS login or
production fleet enrollment. No model credential or inference call is required.

## Hermes authentication boundary

`/api/status` is public **inside Hermes**. Its success proves Core/Iroh admission
and reachability, not access to protected Hermes APIs. The committed browser
client does not yet retrieve Hermes's native bootstrap token. The native-client
acceptance injects a test token; it does not prove browser token discovery.

The approved Iroh mode keeps Hermes on loopback and Core admission as the remote
boundary. The earlier isolated browser spike proved reading the native root
bootstrap, using `X-Hermes-Session-Token` for protected HTTP, native token WebSocket
authentication, and refreshing after a Hermes restart. That code still needs to
be brought into the dashboard path. Tokens stay in browser memory and never pass
through Core; admission revocation must reject even a browser holding the token.

The parallel Caddy spike uses a public Hermes URL and native gated authentication
(username/password/session secret), so its cookie/provider/ticket requirements
are not automatically requirements of this loopback mode. Coordinate endpoint
and evidence contracts, without silently changing the approved auth mode.

Next shared acceptance target: a protected read (prefer the native skills list),
401 without the Hermes token, success after native bootstrap through the real
dashboard, re-bootstrap after Hermes restart, and denial after Core revocation
with the old token still held. Browser-controller identity remains deferred.
