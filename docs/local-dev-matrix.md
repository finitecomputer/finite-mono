# Local Development Matrix

> Status: imported from `finite-eng-docs` during Phase 7 on 2026-07-06. This
> document has not been fully revalidated after the monorepo import. Treat it as
> orientation background, not an authoritative current runbook.

Date: 2026-07-02

This is the monorepo component map of Finite development environments. It is an
orientation layer, not a replacement for component-owned runbooks.

Status: static inventory from README files, runbooks, manifests, justfiles,
package scripts, and CI files. The commands below were not all re-run during
this pass. Treat each command as "documented by the owning repo" until a dated
verification note says otherwise.

## Fast Route By Contributor Goal

| Goal | Start in | Primary loop | Notes |
| --- | --- | --- | --- |
| Self-serve SaaS dashboard/Core UI | `finitecomputer-v2/apps/dashboard` | `npm ci`, `npm run dev` | Current v2 product surface. Good for WorkOS/dashboard/Project/Finite Private UI. Does not prove real runtime launch. |
| v2 Agent Runtime proof or SaaS launch readiness | `finitecomputer-v2` plus `finitechat` | `cargo test --workspace`, dashboard checks, then follow `docs/hermes-runtime-test-matrix.md` | Product proof is native Finite Chat plus real Hermes, `fsite`, `fbrain`, local Docker, remote Docker, Phala, then dashboard-controlled launch. |
| Legacy dashboard chat UI or designer pass | `finitecomputer` | `nix develop`, `just chat-local-bootstrap smoke-finite`, add provider key, `just chat-local-up smoke-finite skyler@finite.vip 3100` | Best current legacy dashboard-to-agent loop. Uses local relay plus MicroSandbox runtime. Not the v2 product chat path. |
| Legacy hosted platform/runtime/control plane | `finitecomputer` | `nix develop`, Cargo commands, root `just` recipes | box1/TRF/smoke and migration bridge lane. Most operator and deployment paths require host secrets or SSH. |
| Native encrypted chat protocol/server | `finitechat` | `cargo run -p finitechat-server -- serve 127.0.0.1:8787 --sqlite .state/finitechat.sqlite3` | Local server and simulator are explicit dev overrides. Production default is `https://chat.finite.computer`. |
| iOS app build or simulator work | `finitechat` | `ios/ci_scripts/ci_post_clone.sh`, `cargo run -p finitechat-rmp -- run ios` | Requires Xcode. Physical phone work also needs a paired phone and signing team. |
| Hermes chat bridge canary | `finitechat` | `cp .env.example .env`, set provider key, run `scripts/hermes-phone-canary.py ...` | Real-Hermes proof is stricter than echo/adapter smokes. |
| FiniteBrain vault, Product Client, or `fbrain` CLI work | `finite-brain` | `cargo test --workspace`, local `finite-brain-app`, Product Client at `/client` | Trusted-client knowledge surface. Keeps Vault/Folder policy in `finite-brain`; generic Nostr primitives stay in `finite-nostr`. |
| Search/extract service work | `finite-search` | `scripts/check-static.sh`, SSH tunnel to `lat2`, service smoke scripts | Current proof is remote-host oriented. A no-SSH local stack is not yet the primary path. |
| Managed skill edits | `finite-skills` | Edit Markdown skill tree, follow `skills/AGENTS.md` | No top-level linter or smoke command exists yet. v2 runtime proof should happen through the `finitecomputer-v2` runtime matrix once that lane is stable; legacy proof still happens through `finitecomputer`. |
| Reusable Nostr primitives | `finite-nostr` | `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings` | Small Rust crate. No repo-local toolchain pin. |
| Reporting snapshots/site data | `reporting` plus legacy `finitecomputer` | `python3 ../finitecomputer/scripts/bootstrap_ai_training_stats.py`, then `python3 ai-training-stats/build_site_data.py` | Generator ownership is currently split; live probes depend on local env/SSH availability. |

## Repo Inventory

### `finitecomputer-v2`

Owns the self-serve SaaS product: WorkOS dashboard, Core, Projects, runner
launch state, Finite Private grant/status surfaces, runtime image/deploy lanes,
and hosted Finite Chat deploy coordination.

Documented tools:

- Rust/Cargo workspace for `finite-saas-core`, `finite-saas-runner`,
  `finite-private-limiter`, and copied `finite-core` support code.
- Next.js dashboard under `apps/dashboard`, using npm and `package-lock.json`.
- WorkOS/AuthKit for SaaS auth when configured.
- Deployment manifests under `deploy/finite-computer`, which still need v2
  renaming and pruning.
- Hosted Finite Chat deploy script under
  `scripts/deploy_finitechat_server_lat1.sh`.
- Runtime image build path for the Agent Runtime, currently packaging
  `finitechat`, the Hermes `finitechat` plugin, `fsite`, and `fbrain`.

Primary dashboard loop:

```bash
cd finitecomputer-v2/apps/dashboard
npm ci
npm run dev
```

Open:

```text
http://localhost:3000
```

Documented checks:

```bash
cd finitecomputer-v2
cargo test --workspace
cd apps/dashboard
npm test
npm run lint
npm run build
```

Product/runtime proof:

- Follow `docs/hermes-runtime-test-matrix.md`.
- Rung order is local real-Hermes adapter, runtime image in local Docker,
  runtime image in remote Docker, Phala CVM, then dashboard-controlled SaaS
  launch.
- Acceptance is native Finite Chat talking to real Hermes, plus `fsite`,
  `fbrain`, Finite Private, durable state, and restart evidence.

Friction:

- This repo was just split from the SaaS branch and intentionally starts too
  large. `docs/carry-over-manifest.md` is the active cleanup map.
- There is no root `just` facade or Nix shell visible yet. The current local
  loops are Cargo, dashboard npm scripts, and deployment/runtime runbooks.
- The dashboard still has carry-over machine/control-plane routes and labels in
  code, even though v2 vocabulary should say Project, Agent Runtime, Runner,
  Hosted Pairing, and Finite Chat Invite.
- `deploy/finite-computer` and the runtime template still carry legacy
  `finitec`/relay/gateway assumptions. Treat those as bridge code with delete
  conditions, not the final v2 product contract.
- Full SaaS launch proof is not a single local command yet; use the runtime
  matrix to avoid overclaiming dashboard-only checks.

### `finitecomputer` legacy

Owns the existing whiteglove product, dashboard relay path, broad `finitec` and
`finited` operations, MicroSandbox compatibility loop, host workspaces, fleet
operations, and migration bridge behavior for box1/TRF/smoke users.

Documented tools:

- Nix flakes for the main dev shell.
- Rust/Cargo workspace for `finitec`, `finited`, and support crates.
- `just` at repo root for local chat and fleet wrappers.
- Next.js dashboard under `apps/dashboard`, using npm and `package-lock.json`.
- MicroSandbox for the product-shaped local dashboard-to-agent loop.
- Docker-like Linux build images during chat-local bootstrap.

Primary legacy local chat loop:

```bash
cd finitecomputer
nix develop
just chat-local-msb-check
just chat-local-bootstrap smoke-finite
# Add at least one model/provider key to .state/chat-local/.env.
just chat-local-up smoke-finite skyler@finite.vip 3100
```

Open:

```text
http://localhost:3100/dashboard/chat/machines/smoke-finite
```

Documented checks:

```bash
nix fmt
cargo test --workspace
cd apps/dashboard
npm test
npm run lint
npm run build
```

Chat-specific checks after `just chat-local-up` is running:

```bash
scripts/relay_e2e.sh
FC_CHAT_BROWSER_BASE_URL=http://localhost:3100 scripts/chat_browser_e2e.sh
```

Contributor-facing `just` command map:

| Command | Use it? | Purpose |
| --- | --- | --- |
| `just chat-local-msb-check` | Yes | Verify MicroSandbox is installed and reachable. |
| `just chat-local-bootstrap smoke-finite` | Yes | One-time setup: dashboard deps, Rust binaries, Hermes checkout, local token, MicroSandbox runtime staging. |
| `just chat-local-up smoke-finite <email> 3100` | Yes | Normal full local dashboard-to-agent loop. |
| `just chat-local-clean` | Sometimes | Delete only local chat state, then rerun bootstrap. |
| `just chat-local-down smoke-finite` | Sometimes | Stop a stale MicroSandbox runtime from a previous run. |
| `just chat-local-msb-logs smoke-finite` | Sometimes | Inspect sandbox/Hermes/finitec logs. |
| `just chat-local-build` | Debug only | Bootstrap substep for Rust binaries. |
| `just chat-local-mint-token` | Debug only | Bootstrap substep for local machine token generation. |
| `just chat-local-msb-prepare` | Debug only | Bootstrap substep for MicroSandbox runtime staging. |
| `just chat-local-msb-probe` | Debug only | Basic MicroSandbox runtime sanity probe. |
| `just chat-local-relay`, `just chat-local-msb-runtime`, `just chat-local-dashboard` | Debug only | Split-terminal version of `chat-local-up`. |
| `just chat-local-finitec-relay`, `just chat-local-hermes` | Break-glass only | Host-Hermes fallback when MicroSandbox itself is blocked. |
| `just dashboard-dev` | Not for chat acceptance | Plain Next.js dev server for non-chat dashboard work. |
| `just tinfoil-*`, `just fleet-*`, workspace recipes | No for normal contributors | Maintainer/operator paths requiring production-like context. |

Friction:

- The best UI loop requires Nix, MicroSandbox, npm, local provider keys, and
  enough host capability for MicroSandbox. MicroSandbox requires Apple Silicon
  on macOS or KVM on Linux.
- `apps/dashboard/README.md` still documents a plain `npm install` and
  `npm run dev` flow. That is useful for some admin UI work but conflicts with
  the chat-local runbook for legacy dashboard chat work.
- Fleet recipes are mixed into the same root `justfile` as contributor recipes.
  They require production-style SSH/secrets and should be hidden from ordinary
  external contributor onboarding.
- Do not use this lane to prove the new self-serve SaaS product unless a
  migration document explicitly says the behavior is still bridged through
  legacy.

### `finitechat`

Owns encrypted chat protocol, local store, server, CLI, iOS app, Hermes bridge,
and canary evidence.

Documented tools:

- Rust workspace with `rust-version = "1.88"` in `Cargo.toml`.
- Released `finitechat` CLI binaries for linux-x86_64, macos-aarch64, and
  macos-x86_64, with sha256 verification in the README install block.
- Xcode, Xcode command-line tools, generated Xcode project, and signing for iOS.
- Python scripts checked with Ruff, BasedPyright, and unittest.
- `finitechat auth` for the shared Finite identity contract and
  `finitechat hermes` for agent init, invite, bridge service, and plugin
  install.
- Optional Docker/runtime canaries for Hermes bridge promotion.
- `.env.example` for Hermes model keys, physical-device IDs, remote Docker, and
  restic/Tinfoil canary settings.

Local server and simulator:

```bash
cd finitechat
cargo run -p finitechat-server -- serve 127.0.0.1:8787 --sqlite .state/finitechat.sqlite3
FINITECHAT_SERVER_URL=http://127.0.0.1:8787 cargo run -p finitechat-rmp -- run ios
```

Agent CLI and Hermes onboarding:

```bash
finitechat --help
finitechat auth status
finitechat hermes init --server https://chat.finite.computer
finitechat hermes install
```

The CLI/agent flow uses the shared Finite identity at
`$FINITE_HOME/identity/identity.json`, falling back to
`~/.finite/identity/identity.json`. The Hermes plugin name and install
directory are `finitechat`, not `finite-platform`.

Friend self-build path:

```bash
ios/ci_scripts/ci_post_clone.sh
cargo run -q -p finitechat-rmp -- doctor
cargo run -q -p finitechat-cli -- http --server https://chat.finite.computer health
open ios/FiniteChat.xcodeproj
```

Documented checks:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo test -p finitechat-server --test http_routes
cargo test -p finitechat-server --test http_persistence
cargo test -p finitechat-server --test http_conformance
cargo run -p finitechat-rmp -- doctor
cargo run -p finitechat-rmp -- bindings swift
cargo run -p finitechat-rmp -- test ios-simulator
uvx --no-config ruff format --check .
uvx --no-config ruff check .
uvx --no-config --with hermes-agent basedpyright
python3 -m unittest discover -s tests -p '*test*.py'
```

Friction:

- Several valid loops exist: local server, simulator, phone canary, remote
  Docker canary, Tinfoil handoff, and production server health. They answer
  different questions and need clearer "choose this first" routing.
- Old CLI/agent stores may still mention `account-secret.hex` or
  `finite-platform`; current CLI/agent flows use the shared identity file and
  the `finitechat` Hermes plugin.
- Rust is pinned in docs/CI metadata, but the checkout has no local
  `rust-toolchain.toml`.
- Python target version is `py311` in `pyproject.toml`, while CI currently uses
  Python 3.13 with pinned tool packages. That may be fine, but should be stated.
- Physical-device proof requires Apple signing, a paired phone, and provider
  keys, which makes it unsuitable as the first external contributor loop.

### `finite-brain`

Owns the encrypted Vault/Folder knowledge system, Product Client, Smoke UI,
`fbrain` CLI, Vault Working Tree sync, and FiniteBrain-specific access, sync,
asset, source-note, and OKF policy.

Documented tools:

- Rust/Cargo workspace for core domain, SQLite store, HTTP server, app binary,
  and CLI.
- Static Product Client under `crates/finite-brain-server/src/product-client.*`.
- Development Smoke UI under `crates/finite-brain-server/src/smoke-ui.*`.
- `fbrain` CLI for trusted agent Vault Working Trees.
- Hosted smoke service at `https://brain.smoke.finite.computer`.

Primary local server loop:

```bash
cd finite-brain
FINITE_BRAIN_ADDR=127.0.0.1:3015 \
FINITE_BRAIN_PUBLIC_BASE_URL=http://127.0.0.1:3015 \
FINITE_BRAIN_DB=.dev-data/finite-brain.sqlite3 \
cargo run -p finite-brain-app
```

Open:

```text
http://127.0.0.1:3015/client
http://127.0.0.1:3015/smoke/ui
```

Documented checks:

```bash
cd finite-brain
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
node --check crates/finite-brain-server/src/product-client.js
node --check crates/finite-brain-server/src/smoke-ui.js
node crates/finite-brain-server/src/product-client.test.js
```

Friction:

- Product Client and Smoke UI are both served by the app, but only the Product
  Client is the user workflow. Do not treat Smoke UI behavior as production UX.
- `fbrain` uses the shared Finite identity location, so local tests that touch
  identity should avoid printing or committing signer state.
- No production FiniteBrain URL is canonized yet; use the smoke URL only where
  the repo-owned docs do.

### `finite-search`

Owns self-hosted SearXNG and Firecrawl operations/integration for agent web
tools.

Documented tools:

- Shell smoke scripts.
- `just` recipes for check, doctor, and smoke wrappers.
- Docker Compose on the `lat2` host.
- SSH tunnels from an operator machine to host-local service ports.
- A local Docker smoke for the SearXNG Tinfoil bundle.

Quick checks:

```bash
cd finite-search
scripts/check-static.sh
just doctor lat2
ssh -L 18080:127.0.0.1:8080 -L 13002:127.0.0.1:3002 lat2 -N
SEARXNG_URL=http://127.0.0.1:18080 scripts/smoke-searxng.sh
FIRECRAWL_URL=http://127.0.0.1:13002 scripts/smoke-firecrawl.sh
```

Friction:

- The happy path assumes SSH access to `lat2`.
- Static checks are easy to run, but full service proof is not currently a
  no-access local onboarding path.
- SearXNG has a small local Compose profile; Firecrawl uses an upstream
  checkout plus a Finite override, so a one-command local stack is not present.

### `finite-skills`

Owns the Finite-managed Hermes skill baseline.

Documented tools:

- Markdown skill contracts under `skills/`.
- Python helper scripts inside some skills.
- Runtime sync through managed Hermes runtimes.

Rules:

- Most managed skills must use the `-finite` suffix in the directory and
  `name:` field.
- `grill-me` is the documented exception.
- User-local skills belong under `~/.hermes/skills`, not this baseline.

Friction:

- No top-level `just check`, package manifest, linter, or CI workflow is visible.
- Broken references, naming mistakes, missing metadata, and script syntax issues
  are not caught by a repo-level command.
- External contributors cannot easily prove that a skill edit will load in the
  managed runtime without involving a v2 runtime-matrix proof or the legacy
  `finitecomputer` harness.

### `finite-nostr`

Owns reusable product-neutral Nostr primitives.

Documented tools:

- Cargo only.
- GitHub Actions run fmt, tests, clippy, and build.

Checks:

```bash
cd finite-nostr
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Friction:

- No local Rust toolchain pin is present.
- CI uses the stable Rust toolchain, while dependent repos may assume a specific
  Rust version.

### `reporting`

Owns generated reporting outputs and site data for AI training stats.

Documented tools:

- Python standard-library CSV/JSON scripts.
- Current CSV generator in legacy
  `finitecomputer/scripts/bootstrap_ai_training_stats.py`.
- Site-data builder and unittest in `reporting/ai-training-stats`.

Commands:

```bash
cd finitecomputer
python3 scripts/bootstrap_ai_training_stats.py
cd ../reporting
python3 ai-training-stats/build_site_data.py
python3 -m unittest ai-training-stats/test_build_site_data.py
```

Friction:

- Source generation currently lives in legacy `finitecomputer`; output
  transformation lives in `reporting`.
- Optional live probes silently become skipped evidence when SSH/env context is
  missing. That is acceptable for reporting, but contributors need the skipped
  probe count surfaced directly in the command output.

## Cross-Component Fragmentation

The current fragmentation falls into a few concrete buckets:

1. There is no checked-in top-level workspace contract. `/Users/alex/Projects/finite`
   is a folder of repos, not a repo with a `justfile`, tool pins, or bootstrap.
2. Command runners differ by repo. Legacy `finitecomputer` and `finite-search`
   use `just`; `finitecomputer-v2` currently uses Cargo, npm scripts, and
   deployment shell scripts; `finite-brain` currently uses Cargo plus
   repo-specific Node checks; `finitechat`, `finite-nostr`, `finite-skills`,
   and `reporting` do not expose the same facade.
3. Toolchain pinning is uneven. Legacy `finitecomputer` has a Nix dev shell and
   package-age guardrails. `finitechat` states Rust 1.88 and pins CI, but the
   local checkout does not enforce it. `finitecomputer-v2` has Rust and npm
   lockfiles but no root contributor facade yet. `finite-nostr` uses ambient
   stable Rust.
4. Contributor loops and operator loops are mixed. This is most visible in
   legacy `finitecomputer`, where local chat recipes and production fleet
   recipes live in the same command surface. v2 avoids some of that by being
   newly split, but its deployment lane is still mostly operator-runbook shaped.
5. Docs route by subsystem more than by contributor profile. A designer, Rust
   contributor, search operator, and iOS tester need different first commands.
6. Validation is uneven. Some repos have strong CI/local checks; `finite-skills`
   has almost none at the repo level.
7. Full-fidelity local work often needs privileged context: model keys,
   MicroSandbox capability, SSH to `lat2`, Apple signing, phone hardware, or
   production-like host secrets.

## Unification Ideas

### 1. Add A Workspace Facade

Create a small checked-in workspace control surface, either by promoting
`finite-eng-docs` or by adding a dedicated workspace layer. It should
not own product code. It should own only clone/update/bootstrap docs and
monorepo command orchestration.

Suggested commands:

```bash
just doctor
just list-repos
just setup ui-chat
just setup chat-rust
just setup search-local
just check all-light
just dev ui-chat
```

The facade should print prerequisites and route into repo-owned commands rather
than reimplement them.

### 2. Standardize Per-Repo Command Names

Every repo should expose the same small command vocabulary where possible:

| Command | Contract |
| --- | --- |
| `just doctor` | Check local prerequisites without mutating state or printing secrets. |
| `just setup` | Install/cache local dependencies from lockfiles or pinned tools. |
| `just dev` | Start the lowest-cost useful local development loop. |
| `just check` | Run the lightweight checks expected before opening a PR. |
| `just smoke` | Run the smallest realistic integration smoke for that repo. |
| `just clean-state` | Delete only repo-owned generated local state. |

Repos can add profile arguments, for example `just dev chat-local`,
`just dev dashboard-admin`, `just smoke ios-simulator`, or
`just smoke search-tunnel`.

### 3. Pin Tools Once

Pick one contributor-facing pinning mechanism and use it consistently. Nix can
remain the strongest path for legacy `finitecomputer`, but v2 and external
contributors need a lighter monorepo story too.

Recommended minimum:

- Add `rust-toolchain.toml` to Rust repos that require a specific compiler.
- Add `packageManager` and npm version expectations to dashboard `package.json`.
- Use `npm ci` in docs, not `npm install`, for locked dashboard setup.
- Add `uv.lock` or documented `uvx` pins for Python-heavy scripts when the tool
  versions matter.
- Add `.env.example` or `.envrc.example` files for nonsecret variable names,
  with comments pointing to the owning runbook.

### 4. Split Contributor And Operator Paths

Keep operator actions available, but make them clearly second-class in the
external contributor path.

For legacy `finitecomputer`, the root `just --list` should group or wrap
commands as:

- contributor: dashboard, chat-local, local checks;
- maintainer: runtime image, local relay, MicroSandbox internals;
- operator: fleet deploy, backups, host secrets, production SSH.

For `finitecomputer-v2`, add a small root facade once the runtime matrix
commands are stable enough to wrap. The first-run path should never require
choosing between hosted deploy docs and dashboard UI docs.

### 5. Provide A Real Remote Design Sandbox

The legacy dashboard designer loop is high fidelity but heavy, and the v2
dashboard loop does not yet prove the real runtime. Add a hosted or
remote-runner design sandbox for people who cannot run the full runtime stack
locally.

Target shape:

- disposable machine/runtime, not production user state;
- real native Finite Chat/runtime/Hermes path for v2, or real relay/runtime
  path for legacy work, not mocked chat messages;
- seeded demo credentials and a reset button;
- dashboard preview URL per branch or per short-lived session;
- clear statement that final acceptance still uses the same real path.

This would make visual contributions possible from machines without KVM,
provider keys, or the full platform checkout.

### 6. Make Search Locally Reproducible

Add a `finite-search` local stack profile that does not require `lat2`:

```bash
just local-up
just smoke-stack
just local-down
```

It can start with SearXNG-only and add Firecrawl when the upstream checkout
wrapper is stable enough. Keep SSH tunnel smokes as operator validation, not the
first external contributor path.

### 7. Add A Skill Linter

Give `finite-skills` a lightweight repo-level check:

- validate `SKILL.md` presence and required metadata;
- enforce `-finite` suffix except documented exceptions;
- verify relative reference paths exist;
- syntax-check bundled Python helpers;
- report unmanaged or colliding skill names;
- optionally check forbidden placeholder patterns outside examples/templates.

That unlocks a real `just check` and CI workflow for skill contributors.

### 8. Align Local Checks With CI

Each repo's README should have one "before PR" command that matches CI as
closely as practical. Heavy canaries can stay opt-in, but the light checks
should not require reading workflow YAML.

Suggested first-pass targets:

- `finitecomputer-v2`: `just check` or equivalent for Cargo tests, dashboard
  lint/tests/build, and a clearly named runtime-matrix smoke when available.
- Legacy `finitecomputer`: `just check` for `nix fmt --check` or `nix fmt`,
  Cargo tests, dashboard lint/tests/build.
- `finitechat`: `just check` for Cargo fmt/clippy/tests plus Python lint/tests.
- `finite-brain`: Cargo fmt/test/clippy/build plus Product Client and Smoke UI
  JavaScript syntax checks.
- `finite-search`: existing `just check`.
- `finite-skills`: new linter.
- `finite-nostr`: Cargo fmt/test/clippy/build.
- `reporting`: build site data against latest run plus unittest.

## Suggested Cleanup Order

1. Fix conflicting contributor docs first: `finitecomputer-v2` dashboard docs
   should use `npm ci`, point runtime acceptance to
   `docs/hermes-runtime-test-matrix.md`, and say dashboard chat is out of scope.
   Legacy dashboard docs should point chat UI contributors to
   `docs/chat-local-dev.md`.
2. Add `just doctor`, `just setup`, and `just check` to every repo, even if the
   first implementation is thin.
3. Add local toolchain pins for Rust, Node/npm, and Python where docs or CI
   already assume versions.
4. Add the `finite-skills` linter and CI. It is the largest gap in basic
   external contribution safety.
5. Add a no-SSH `finite-search` local smoke profile.
6. Build the workspace facade after the per-repo commands are stable enough to
   wrap cleanly.
7. Add the remote design sandbox once the exact local chat acceptance path is
   stable and documented.
