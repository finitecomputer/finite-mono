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
