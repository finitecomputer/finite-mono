# Hosted Hermes local qualification

These run real Caddy and pinned Hermes. They do not enable hosted access or
perform production mutations. Credentials are generated in memory, synthetic
Hermes homes are isolated, and no model provider is used.

## Runner-rendered native TLS proof

Build the current Runner, then from the repository root:

```sh
scripts/with-dev-env cargo build -p finite-saas-runner --locked
RUNNER_PROOF_BINARY=target/debug/finite-saas-runner \
  bash scripts/proofs/hosted-hermes-caddy-native.sh
```

On macOS the launcher uses the existing OrbStack `nixos` VM. On Linux it needs
root permission to create disposable network/mount/PID namespaces. Dependencies
come from the repository flake; first realization may download/build packages.
The runtime uses the pinned upstream minimal Python environment and bundled
plugins, without building Hermes's frontend or optional integrations.

The production Runner renders the Caddy JSON, which the proof consumes unchanged.
Only loopback exists inside the network namespace, so the native `0.0.0.0` bind
cannot expose a port on the host's external network. Caddy terminates real local
TLS; its temporary CA is trusted by the proof's Node process only and is never
installed in a system trust store. A synthetic nonloopback Hermes `public_url`
engages native auth while the TLS edge uses `localhost`.

The proof checks native login, prefix-scoped Secure/HttpOnly cookies, single-use
WS tickets, actual expiry, reconnect without relogin, host/prefix isolation,
client-prefix overwrite, and credential omission from real proxy-error logs.
It sends the production dashboard `Origin` header unchanged using Node's native
WebSocket client: loopback-bound Hermes rejects it; the isolated guest-style
`0.0.0.0` bind accepts it. This is a protocol proof, not an actual browser test.
Native REST bearer authentication is checked separately: `/api/auth/me` rejects
anonymous/invalid sessions and accepts a real native session. The configured
production origin receives CORS permission and an Authorization preflight;
unlisted origins do not inherit Hermes's native localhost CORS policy.

It does not qualify Core issuance/pull, provider/Kata host-port allocation,
production DNS/TLS, native Desktop, the dashboard UI, or model-turn durability.

## Actual-browser component proof

After installing the dashboard's locked dependencies through the repository
development environment, run `scripts/proofs/hosted-hermes-caddy-browser.mjs`
with Node24 and these explicit tool paths in the environment:

- `RUNNER_PROOF_BINARY`: built Runner executable.
- `CADDY_BIN`: repository-pinned Caddy executable.
- `HERMES_PROOF_BINARY`: `hermes-agent-minimal-runtime/bin/hermes` from the flake.
- `HERMES_PROOF_SOURCE`: the locked `hermes-agent` input source path.
- `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH`: an installed Chrome/Chromium executable.

This launches a fresh browser profile against unchanged Runner-rendered Caddy
configuration and real native Hermes. Only the temporary fixture certificates
are trusted in that browser profile; system trust is unchanged. It proves the
allowed origin can read public status and perform a protected native identity
read; anonymous/invalid sessions get401 and a real native bearer gets200. A real
`SKILL.md` in the synthetic agent home also appears through protected
`/api/skills`. A second, unlisted page origin is blocked by the browser's CORS
enforcement even for public status. Services and homes are removed on exit.

This component page is deliberately separate from dashboard/Core authorization
acceptance. It uses a loopback backend with a local browser Origin; the namespace
proof above covers the production Origin and guest bind. Neither test proves
production publication, revocation, or the full dashboard enablement flow.

## Integrated dashboard access proof

`hosted_browser_composition` runs the production Core routers against disposable
Postgres, agentd's real desired-state pull/supervisor, pinned native Hermes,
Runner-rendered Caddy HTTP routes, the built Next dashboard API route, and the
shipped browser request helper in Chromium. A real `SKILL.md` in the synthetic
Hermes home must appear through protected `/api/skills`.

Build the Runner/agentd and dashboard using the pinned development environment,
start disposable Postgres, then set these tool paths (the same pin as the
component proof): `RUNNER_PROOF_BINARY`, `AGENTD_PROOF_BINARY`, `CADDY_BIN`,
`HERMES_PROOF_BINARY`, `HERMES_PROOF_SOURCE`, and
`PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH`. Run from the repository root:

```sh
FC_CORE_POSTGRES_TEST_URL=postgresql://postgres@127.0.0.1:55439/postgres \
  scripts/with-dev-env cargo test -p finite-saas-core hosted_browser_composition \
  --locked -- --ignored --nocapture
```

The test checks an ordinary owner, signed-out/other-owner/wrong-agent denial,
native anonymous/invalid-token rejection, exact browser CORS, applied enable,
real 61-second expiry and automatic renewal, applied disable, and re-enable
rotation while retaining the skill. It reserves the native fixed port 8642 and
fails if occupied. Run this proof alone; do not rebuild the dashboard while it
is serving. Generated credentials use inherited pipes/environment only.
Services are stopped on exit; successful scratch homes are removed. A failure
retains private diagnostics in the printed scratch directory; never publish
raw logs or credential files.

The account adapter is the dashboard's existing local development-account path;
Core verifies signed JWTs with its isolated WorkOS source. This does not exercise
live WorkOS OAuth. Unrelated gateway/Chat children are lifecycle fixtures, not a
Chat continuity proof. Caddy uses a temporary local certificate issuer and test
DNS resolution, while rendered HTTP routes/CORS remain unchanged. Only those
fixture certificates are trusted by Core and the fresh browser, never the OS.
Existing-agent enrollment, fleet/Kata publication, restore and model-turn
continuity remain separate qualification work.

## Address reuse negative control

```sh
CADDY_BIN=/nix/store/<repo-pinned-caddy>/bin/caddy \
  scripts/with-dev-env node scripts/proofs/hosted-hermes-routing.mjs
```

Expected exit code is **1** with a `blocked:` verdict. Synthetic independent
HTTP backends intentionally detect credential disclosure after an acknowledged
route withdrawal and backend address reuse. These backends are adversarial
receivers, not mocked Hermes behavior. The confirmed-Caddy-exit control must
prevent the disclosure. Exit **2** means the proof itself failed. Route reload
acknowledgement must never be interpreted as an address-reuse fence.

`hosted-hermes-auth.mjs` also provides the standalone loopback auth fixture and
optional local browser page. `hosted-hermes-lifetime.mjs` covers retained versus
disposable native drafts over ingress restart; it explicitly does not establish
accepted model-turn durability. Both accept `HERMES_PROOF_BINARY` pointing to a
full repository-pinned native Hermes wrapper. The lifetime fixture requires the
same `CADDY_BIN` environment setting.
