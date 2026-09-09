# FIN-37: hosted control proof

This slice extends #860 with real Core owner authorization, HTTPS forwarding,
and an opt-in control listener inside `finite-agentd serve`. It does not enable
any production host. All runtime changes belong to FIN-57's combined R1 release.

Core exposes a narrow GET/POST `me/runtime-agent-control/{identifier}` endpoint.
It resolves current Project ownership and active runtime links from Postgres and forwards opaque request
bytes to the agent-owned Connections router. Core does not own feature schemas.
No browser-selected URL, cached ownership, chat identity or owner-claim ceremony
participates in authorization. Tokens never leave Core in a client response.

For this local component proof, `FC_CORE_AGENT_CONTROL_DIRECTORY` contains
private per-runtime JSON files named `<runtime_id>.json`, with `source_host_id`,
`source_machine_id`, `endpoint` (HTTPS origin), and `token`. The binding must match
Core's current runtime row. Files are read every request, fail closed if absent
or world/group-readable, and may be atomically replaced/revoked. An optional
`FC_CORE_AGENT_CONTROL_CA_FILE` adds a local test CA without bypassing TLS checks.
This explicit provisioned file is a temporary integration boundary, NOT an
automated runner provisioning or rotation protocol. Before R1, replace manual
file delivery with the reviewed runner/Core credential handoff and prove stale
placement, replacement, and revocation. Do not add another polling proxy process.

`FINITE_AGENTD_CONTROL_CONFIG` names JSON with `listen`, `runtime_id`, `token_file`.
The listener remains loopback-only pending runner ingress integration. The
existing token-file permissions and exact runtime targeting checks still apply.
When absent, the production startup sequence remains unchanged. When present,
local Hermes preparation runs independently; chat-only preparation retries in the
background. Chat delivery and HTTP operations share one mutation permit. The
runtime launcher separates local preparation, chat preparation and run-only so
restarts cannot overwrite settings concurrently with commands.

Recovery boot intents are explicitly not enabled in this opt-in topology yet;
qualify recovery before R1. Existing authorized-principal rows and chat command
wire/ledger bytes retain their authority and representation. Independent control
does not claim any Principal. HTTPS results use the spike's additive table and
fail-closed interrupted-operation semantics.

The dashboard direct path is explicitly selected by deployment configuration;
old runtimes retain the existing product path until the coordinated cutover.
There is no error-triggered fallback from direct control into chat. Remove the
legacy selection when FIN-37's rollout is qualified, and remove the scratch-only
control-serve command after its test callers move onto the integrated mode.

## Reproduce the local proof

Use the root pinned Nix development shell, install dashboard dependencies with
`pnpm install --frozen-lockfile`, and build the actual services:

```sh
scripts/with-dev-env cargo build -p finite-agentd -p finite-saas-core -p devfinity --locked
export FIN37_HERMES_ENV="$(nix build .#hermes-agent-runtime-python --no-link --print-out-paths)"
# Set FINITE_AGENTD_TEST_CADDY to the executable in your pinned Nix Caddy artifact.
scripts/with-dev-env node scripts/tests/fin37-hosted-control.mjs
```

If those package attributes are unavailable on the host, supply the equivalent
pinned Hermes environment and Caddy store paths explicitly. The proof requires
local loopback listeners. It uses disposable Postgres, private temporary tokens,
a local Caddy CA trusted only by this Core process, and a real Next development
server. Successful runs remove their temporary state; failed runs retain private
logs at the reported temporary directory. No production credentials are needed.

Verified locally on 2026-09-09:

- Dashboard API → Core → verified HTTPS/Caddy → integrated agentd control.
- Google credential disconnect removes the local credential; inference apply
  invokes pinned Hermes validation, persists settings, and restarts real Hermes.
- Chat initialization fails throughout; no agent chat config, chat identities,
  or project room memberships exist.
- Missing identity and wrong owner are rejected. Revoking ownership takes effect
  on the next request. Stale host bindings fail closed. Token rotation rejects the
  old token immediately; removing the Core target returns unavailable without
  falling back to chat.

Only external WorkOS uses devfinity's signed identity fixture. Initial ownership
and placement are seeded into real migrated Postgres; this is not an onboarding
proof. Hermes runs real `hermes serve --isolated` under agentd via a local launcher;
it does not qualify the production gateway plugin startup path. No remote Google,
Telegram, SimpleX or inference-provider network operation is claimed by this test.

Component validation also passed agentd tests and clippy, Core tests against real
Postgres, and dashboard tests/typecheck/lint/build. Existing launcher tests cover
legacy preparation; these are separate from the real service proof above.

## Gates before activation (FIN-57 / AGENT ROLLOUT R1)

1. Route runner HTTPS ingress to the appropriate VM/container listener; configure
   real DNS/TLS and per-runtime authentication, without WireGuard or chat.
2. Replace the local target-file provisioning seam with the runner/Core handoff;
   prove placement changes, replacement, revocation, and bounded credential scope.
3. Qualify recovery boots, fresh managed-skills initialization without chat,
   shutdown during mutation, old/new runtime coexistence, and all supported
   Connections operations against their real external services.
4. Ship the runtime prerequisites together in R1. Enable the dashboard HTTPS
   selector only for qualified runtimes. UI iteration itself needs no agent rollout.

This PR is a component proof, not permission to activate the new topology or a
claim that FIN-37 is complete. Keep the parent stack draft until its gates pass.
