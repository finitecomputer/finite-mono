# Browser access to agent APIs

The dashboard slice reads native Hermes `GET /api/status` and authenticated
`GET /api/skills`. Existing chat stays in place. Skills UI integration, Brain and
Sites listings come next; Connections and chat integration follow. This crate is transport, not a new chat server or a
Core-owned copy of agent data.

An admin opens an agent's **Advanced → Agent status (internal)** page. Hosted
access starts disabled. Core checks the signed account identity and authorizes a
fresh ephemeral browser peer for the registered runtime generation. Browser WASM
then sends HTTP over Iroh to the runtime's fixed Hermes loopback target. The
response goes directly back to the browser, without passing through Core.

Each read removes its admission and closes its peer when finished. If the tab
vanishes, runtime-enforced expiry is the fallback. Enabling access still grants
full native Hermes access to admitted peers; the read-only UI does not narrow
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

`/api/status` is public **inside Hermes**. The protected skills read closes the
separate authentication gap: the browser retrieves `/` through the admitted
tunnel, parses the native JSON token assignment without executing HTML, and
sends `X-Hermes-Session-Token` to `GET /api/skills`. The token lives only for that
read. A 401 triggers one fresh bootstrap; other HTTP errors are surfaced.
Gated/public Hermes mode fails explicitly, rather than falling back to passwords
or silently weakening its authentication.

The parallel Caddy spike uses native gated authentication with a public URL.
Both spikes align on `/api/skills` as the payload contract; `/api/auth/me` needs a
session principal and is not a loopback-token compatibility test. Browser-
controller identity remains deferred.

## Client contract for the skills UI

Import `readAgentSkills(projectId, abortSignal)` from
`src/lib/agent-api-client.ts` in the dashboard. It returns native skill metadata:
`name`, `description`, optional `category`, `enabled`, `usage`, and `provenance`
(`agent`, `bundled`, or `hub`). Hermes scans the selected default profile's local
and configured external skill directories, including disabled skills, and owns
its cache/provenance semantics. This is the agent's live inventory, not the
Finite catalog or an indication that its agent process is currently running.

`readAgentJson(projectId, path, abortSignal)` is the same authenticated read-only
transport for additional native APIs. It bounds work to 30 seconds, retries
transport setup while admission propagates, and closes the ephemeral peer on
completion/cancellation. It does not support writes, streaming or renewal yet.
The current dashboard admission route is admin-only; fleet enrollment and
owner-facing UI remain separate gates.

The acceptance creates a real `SKILL.md` in the disposable agent home and checks
its name and description through the real dashboard. A separate browser peer
proves skills returns 401 without a token and 200 with native bootstrap. The
fixture restarts the actual Hermes child (which generates its own new token),
checks the previous token is rejected, and re-reads the skills through the UI.
Disabling access rejects a peer still holding a valid token.
