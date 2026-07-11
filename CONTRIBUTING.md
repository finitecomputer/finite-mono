# Contributing to Finite Mono

Choose the smallest local loop that proves the change you are making. Web
design work does not require production access, an inference credential, or a
local Agent Runtime. The complete SaaS stack does.

## 1. Toolchain (one-time, ~5 min)

Everything is pinned by Nix — do not install Rust/Node/Postgres yourself.

1. Install Nix (with flakes enabled) — https://nixos.org/download
2. Install direnv — https://direnv.net
3. `git clone https://github.com/finitecomputer/finite-mono && cd finite-mono`
4. `direnv allow` — the flake dev shell provides rustc/cargo, just,
   process-compose, Postgres, and friends.

No direnv? Prefix commands with `scripts/with-dev-env`.

## 2. Choose a local loop

### Web dashboard and chat design

This is the normal loop for iterating on the shipped web UI. It runs the real
dashboard components and routes against deterministic local Core and Hosted
Device fixtures:

```
cd finitecomputer-v2/apps/dashboard
npm ci
cd ../../..
just dev web-design
```

Open
<http://127.0.0.1:13002/dashboard/machines/skyler-fixture/chat>. The seeded
conversation survives stopping and restarting the command. Exercise failure
and recovery UI from another terminal:

```
just dev web-design-state unavailable
just dev web-design-state recovering
just dev web-design-state healthy
```

`just dev web-design-reset` explicitly deletes only this local fixture's
conversation. None of these commands contacts WorkOS, a provider, an Agent
Runtime, or production. See the
[dashboard README](finitecomputer-v2/apps/dashboard/README.md#run-locally) for
the fixture boundary and useful routes.

The design fixture and complete SaaS stack both use port 13002, so normally run
only one at a time. To put the fixture on another port:

```
FC_WEB_DESIGN_PORT=13003 just dev web-design
```

### Complete local SaaS

Use this lane when a change must prove real Core, Runner, Hermes, Hosted Web
Device, or restart behavior. It requires Apple silicon on macOS 26 or newer,
Apple Container 1.1 or newer, and an operator-provided inference credential:

```
container system start
export FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY=<operator-provided-key>
just dev saas-smoke
just dev up
```

On later runs with a persisted agent, skip `just dev saas-smoke`. That first
smoke owns the stack while it runs, obtains and redeems a single-use local
Launch Code through the operator fixture, launches the local agent, proves real
chat and restart healing, and preserves the agent for the following interactive
`just dev up`.

Then open <http://127.0.0.1:13002/dashboard>. Devfinity supplies local WorkOS
and Stripe boundaries while retaining the real entitlement, Project, Runner,
Agent Runtime, and Hosted Web Device flow. Its complete browser-product spine
is the local WorkOS fixture, Postgres, Core, Finite Chat, Hosted Web Device,
Sites, Brain, dashboard, canonical Runtime image, optional local limiter, and
Runner under process-compose and Apple Container. It does not start unrelated
monorepo services such as Search. URLs land in
`.local-state/devfinity/runs/default/urls.txt`.

Never write the inference credential into the repository. If Skyler does not
need a real Hermes response, use the web-design loop instead. The complete
[devfinity runbook](docs/local-integration-harness.md) documents networking,
the direct-key fallback, durable state, cleanup, and prerequisites. Quit the
TUI (or Ctrl-C) to stop supervised host services; `just dev cleanup` recovers
orphaned host processes without deleting agent data.

## 3. Make a change and see it

Pick a product surface:

- **Dashboard** (Next.js): edit `finitecomputer-v2/apps/dashboard/`, the dev
  server hot-reloads inside the stack.
- **CLI** (`finitechat`, `fsite`, `fbrain`): `cargo run -p finitechat-cli --bin finitechat -- --help`
- **Server** (core / chat / sites): edit, then restart the stack.
- **iOS**: see `finitechat/ios/` + `finitechat-rmp` (XcodeGen; run from
  `finitechat/` so `rmp.toml` resolves).
- **Electron**: see `finitechat/` Electron app docs.

## 4. Gates before you push

Run the gate proportionate to the surface you changed:

```
just web-check       # dashboard unit tests, lint, and production build
just check           # cargo check --workspace --locked
just fmt             # rustfmt
just test            # cargo test --workspace --locked
just dev smoke       # portable services-only integration smoke (Linux CI)
just dev saas-smoke  # real Apple Runtime + Hosted Web chat + restart healing
```

CI (`.github/workflows/ci.yml`) runs fmt/clippy/tests against real Postgres,
dashboard lint/test/build, the finitechat Hermes bridge suite, and
skills/search checks on every PR.

## Rules worth repeating

- **This repo is public. Never commit a secret value.** Names and locations
  only (`infra/README.md` explains the secrets model).
- Release asset names and legacy install URLs are product contracts.
- Deploy changes go in `infra/`, not in shell history.
