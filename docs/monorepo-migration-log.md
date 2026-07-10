# Monorepo Migration Log

This log records the source snapshots and migration decisions used to construct
`finite-mono`.

## Phase 0: Preparation

Date: 2026-07-06

Destination repository:

- `/Users/alex/Projects/finite/finite-mono`

Import method:

- Direct file copy from source repos.
- No git history preservation.
- No `git subtree`, `git filter-repo`, or equivalent history-preserving import
  mechanism.
- Existing source repos remain untouched.
- No rollback plans are being maintained for the copy operation.

Source snapshots:

| Repo | Source path | Commit SHA | Working tree at record time |
| --- | --- | --- | --- |
| `finitecomputer-v2` | `/Users/alex/Projects/finite/finitecomputer-v2` | `862e6bf11ec2c8e8c0e6b3d85471a39257bb7e21` | Clean |
| `finitechat` | `/Users/alex/Projects/finite/finitechat` | `f13c973d493831065994767b0f93783a49873071` | Clean |
| `finite-sites` | `/Users/alex/Projects/finite/finite-sites` | `768a0b84898e7acfd865dd1e13645ab6ce19ea09` | Clean |

Notes:

- These SHAs identify the source commits used for the first planned copy.
- The copied tree will be a snapshot, not a history-preserving import.

## Phase 1: Monorepo Skeleton

Date: 2026-07-06

Created root skeleton files:

- `README.md`
- `AGENTS.md`
- `justfile`
- `scripts/.gitkeep`
- `Cargo.toml`
- `flake.nix`
- `flake.lock`

Created or retained root docs:

- `docs/monorepo-plan.md`
- `docs/fedimint-monorepo-structure-analysis.md`
- `docs/monorepo-migration-log.md`

Rust workspace setup:

- Root `Cargo.toml` is an empty virtual workspace for now.
- Workspace members will be added after source repos are copied.
- Workspace resolver is `2`.
- Workspace package defaults set edition `2024`, license `MIT`, repository
  `https://github.com/finitecomputer/finite-mono`, and `rust-version` `1.88`.

Nix setup:

- Root `flake.nix` uses a pinned `nixpkgs` input and `flake-utils`.
- Root `flake.lock` pins:
  - `nixpkgs` to `b6018f87da91d19d0ab4cf979885689b469cdd41`.
  - `flake-utils` to `11707dc2f618dd54ca8739b309ec4fc024de578b`.
- The default dev shell is intentionally minimal. It includes
  Rust/Cargo/rustfmt/Clippy/rust-analyzer, `just`, `pkg-config`, and OpenSSL.
  Add native dependencies such as protobuf or SQLite only when copied crates
  require them.
- `nix develop -c rustc --version` reported
  `rustc 1.91.1 (ed61e7d7e 2025-11-07)`.
- `nix develop -c cargo --version` reported
  `cargo 1.91.0 (ea2d97820 2025-10-10)`.

Validation run:

- `cargo metadata --format-version 1 --no-deps`
- `just --list`
- `nix flake show --all-systems`
- `nix develop -c rustc --version`
- `nix develop -c cargo --version`

## Phase 2: Copy `finitecomputer-v2`

Date: 2026-07-06

Copied source:

- From `/Users/alex/Projects/finite/finitecomputer-v2`
- To `/Users/alex/Projects/finite/finite-mono/finitecomputer-v2`
- Source commit SHA: `862e6bf11ec2c8e8c0e6b3d85471a39257bb7e21`

Copy method:

- Direct `rsync` file copy.
- Excluded `.git/`.
- Excluded generated or machine-local directories:
  - `target/`
  - `apps/dashboard/node_modules/`
  - `apps/dashboard/.next/`
  - `.local-state/`

Validation run from copied tree:

- `cargo check --workspace`
  - Result: passed.
- `npm ci` from `finitecomputer-v2/apps/dashboard`
  - Result: passed under system Node `v18.16.0`, but emitted engine warnings
    because dashboard dependencies require newer Node versions.
- `npm test` from `finitecomputer-v2/apps/dashboard` under system Node
  `v18.16.0`
  - Result: failed with `node: bad option: --import`.
  - Cause: system Node is too old for the dashboard test command.
- `nix shell nixpkgs#nodejs_24 -c npm ci` from
  `finitecomputer-v2/apps/dashboard`
  - Result: passed.
- `nix shell nixpkgs#nodejs_24 -c npm test` from
  `finitecomputer-v2/apps/dashboard`
  - Result: passed, 100 tests.

Notes:

- The copied repo's internal folder structure was left intact.
- The root flake was not expanded to include Node. Node 24 was used through a
  transient Nix shell for validation only.
- Generated validation artifacts were removed after checks:
  `finitecomputer-v2/target` and
  `finitecomputer-v2/apps/dashboard/node_modules`.

## Phase 3: Copy `finitechat`

Date: 2026-07-06

Source:

- From `/Users/alex/Projects/finite/finitechat`
- To `/Users/alex/Projects/finite/finite-mono/finitechat`
- Source commit: `f13c973d493831065994767b0f93783a49873071`
- Source worktree state before copy: clean

Copy method:

- Direct `rsync` snapshot copy.
- No git history was preserved.
- The existing source repo was not modified.

Excluded generated or local paths:

- `.git/`
- `target/`
- `.finitechat/`
- `.state/`
- `.direnv/`
- `.env`
- `.env.*`
- `ios/.build/`
- `ios/Frameworks/`
- `ios/Bindings/*.swift`
- `ios/Bindings/*FFI.h`
- `ios/Bindings/*FFI.modulemap`
- `ios/*.xcodeproj/`
- Python `__pycache__/` and `*.pyc` files
- `.DS_Store`

Root workspace integration:

- Removed copied `finitechat/Cargo.toml` and `finitechat/Cargo.lock` so the
  copied Rust crates resolve through the monorepo root workspace and root
  `Cargo.lock`.
- Added `finitechat/crates/*` and `finitechat/uniffi-bindgen` as explicit root
  workspace members.
- Moved `finitechat`'s workspace dependency table into the root
  `[workspace.dependencies]`, with local paths adjusted to `finitechat/...`.
- Aligned `rusqlite` to `0.37` in the root workspace dependency table because
  the combined workspace can only link one `libsqlite3-sys` crate.

Validation:

- `cargo metadata --format-version 1 --no-deps`
- `cargo check --workspace`
- `just check`
- `just test`
- `python3 -m unittest discover -s tests -p '*test*.py'` from `finitechat/`
  - Initial result: failed because the test expected binaries under
    `finitechat/target/debug` after the workspace was flattened.
- `CARGO_TARGET_DIR=/Users/alex/Projects/finite/finite-mono/target python3 -m unittest discover -s tests -p '*test*.py'`
  from `finitechat/`
  - Result: passed, 119 tests, 4 skipped.
- `cargo run -p finitechat-rmp -- doctor`
  - From monorepo root: failed because `finitechat-rmp` searches for
    `rmp.toml` in the current directory and parents.
  - From `finitechat/`: passed.

Notes:

- `just test` passed for the expanded root Rust workspace, including the copied
  `finitechat` Rust tests.
- `finitecomputer-v2` still depends on git-pinned `finitechat-*` crates. The
  copied local `finitechat-*` crates coexist in the workspace for now; replacing
  those git dependencies with local path dependencies is a later compatibility
  step.
- Python tests created `.finitechat/` and `__pycache__/` artifacts in the
  copied tree; those generated artifacts were removed after validation.

## Phase 4: Copy `finite-sites`

Date: 2026-07-06

Source:

- From `/Users/alex/Projects/finite/finite-sites`
- To `/Users/alex/Projects/finite/finite-mono/finite-sites`
- Source commit: `768a0b84898e7acfd865dd1e13645ab6ce19ea09`
- Source worktree state before copy: clean

Copy method:

- Direct `rsync` snapshot copy.
- No git history was preserved.
- The existing source repo was not modified.

Excluded generated or local paths:

- `.git/`
- `target/`
- `.dev-data/`
- `.finite/`
- `.env`
- `*.pem`
- `*.key`
- `.DS_Store`

Root workspace integration:

- Removed copied `finite-sites/Cargo.toml` and `finite-sites/Cargo.lock` so the
  copied Rust crates resolve through the monorepo root workspace and root
  `Cargo.lock`.
- Added `finite-sites/crates/*` as explicit root workspace members.
- Moved `finite-sites` workspace dependencies that were not already present into
  the root `[workspace.dependencies]` table.
- Extended root `tokio` features with `signal` and `time` for `finitesitesd`.

Validation:

- `cargo metadata --format-version 1 --no-deps`
- `cargo check --workspace`
- `just test` from `finite-sites/`
  - Result: passed. After workspace flattening, this command resolves through
    the monorepo root workspace.
- `cargo run -p fsite-cli --bin fsite -- describe workflow project-config --output json`
  from `finite-sites/`
  - Result: passed and emitted valid JSON.
- `just check` from the monorepo root
  - Result: passed with `--locked`.
- `just test` from the monorepo root
  - Result: passed with `--locked`.

Notes:

- `just dev` was not run because it starts a long-running local server against
  `.dev-data`.
- No generated `finite-sites` artifacts were left in the copied tree after
  validation.

## Phase 5: Root Cargo Workspace for `finitecomputer-v2`

Date: 2026-07-06

Fedimint reference checked:

- Fedimint uses one explicit root Cargo workspace and one root `Cargo.lock`.
- The Finite root workspace follows that shape for the imported
  `finitecomputer-v2` Rust crates.

Changes:

- Added the five `finitecomputer-v2/crates/*` packages as explicit root
  workspace members.
- Moved `finitecomputer-v2/Cargo.lock` to the monorepo root as `Cargo.lock`.
- Removed `finitecomputer-v2/Cargo.toml` so Cargo commands from inside
  `finitecomputer-v2` resolve upward to the monorepo workspace instead of
  recreating a nested lockfile.
- Added a root `.gitignore` for generated files including root `target/`.
- Kept dependency declarations in the member crate manifests for now instead of
  introducing a root `[workspace.dependencies]` table immediately.

Validation:

- `cargo metadata --format-version 1 --no-deps`
- `cargo check --workspace --locked`
- `cargo test --workspace --locked`
- `find . -name Cargo.lock -o -name Cargo.toml | sort`
- `cargo metadata --format-version 1 --no-deps` from
  `finitecomputer-v2/`

Result:

- The root workspace owns the imported Rust crates.
- Only one Cargo lockfile exists: `Cargo.lock` at the monorepo root.
- Root Cargo check and test both passed.

## Phase 6: Minimal Root Commands

Date: 2026-07-06

Fedimint reference checked:

- Fedimint's root `justfile` is generated from flakebox and includes a broad
  command surface for build, check, format, lint, test, watch, Clippy, Semgrep,
  and typos.
- Finite is intentionally using a small handwritten root `justfile` for now.

Changes:

- Kept `just default` as a command list. After adding the first module, it runs
  `just --list-submodules --list` so nested commands are visible.
- Added `just metadata` for root Cargo workspace metadata validation.
- Added `just check` for `cargo check --workspace --locked`.
- Added `just fmt` for `cargo fmt --all`.
- Added `just test` for `cargo test --workspace --locked`.
- Added `mod sites 'finite-sites/justfile'`, making commands such as
  `just sites build` available from the monorepo root.
- Updated `finite-sites/justfile` so `build`, `test`, `lint`, and `fmt` are
  package-scoped to the Finite Sites crates instead of using the full root
  workspace.
- Left dashboard, chat, CI, release, deploy, and root formatter commands out of
  the root command surface for now.

Validation:

- `just`
- `just --list-submodules --list`
- `just metadata`
- `just check`
- `cargo fmt --all -- --check`
- `just fmt`
- `just test`
- `just sites build`
- `just sites test`
- `just sites lint`

## Phase 7: Docs

Date: 2026-07-06

Fedimint reference checked:

- Fedimint keeps root documentation under `docs/`, mixes durable Markdown
  orientation with generated API/reference docs, and includes a docs Cargo
  workspace package.
- Finite is not copying the docs Cargo package or generated-doc publishing
  setup yet. The useful pattern for now is one root docs entry point plus clear
  ownership boundaries.

Source checked:

- `finite-eng-docs` source checkout:
  `4044b9b2aae698796ae1238a9d8a8bf959377a7f`.
- The source checkout had a dirty `README.md`; the imported files came from
  `finite-eng-docs/docs/`, while the new monorepo `docs/README.md` was written
  fresh for `finite-mono`.

Docs added or updated:

- Added `docs/README.md` as the current root docs index.
- Imported `architecture-overview.md`, `system-flow-and-trust-boundaries.md`,
  `navigation-plan.md`, `local-dev-matrix.md`, `slop-audit.md`, and
  `system-flow-and-trust-boundaries.excalidraw` from `finite-eng-docs`.
- Added unreviewed/stale status banners to the imported Markdown docs.
- Updated imported references from "cross-repo" to monorepo/component language
  where it was mechanical.
- Updated root `README.md` to point at the docs index.

Decision:

- Repo-local docs remain inside `finitecomputer-v2/`, `finitechat/`, and
  `finite-sites/` for now.
- Imported orientation docs are useful background, but not canonical runbooks
  until the Phase 13 stale-doc audit promotes, rewrites, or deletes them.

## Phase 9: Initial Local Integration Harness

Date: 2026-07-06

Fedimint reference checked:

- Fedimint's `devimint` is a top-level Rust crate that owns local integration
  environment setup, generated env files, ready state, logs, and process
  orchestration.
- Fedimint uses thin scripts around the harness for interactive developer
  flows, including an `mprocs` view over running service logs.
- Finite copied the durable shape but uses `process-compose` as the process
  runtime/TUI instead of implementing supervision directly in Rust.

Changes:

- Added top-level workspace crate `devfinity`.
- Added `process-compose` to the Nix development shell.
- Added `devfinity up`, `up --headless`, and `cleanup`.
- `devfinity` writes deterministic generated state under
  `.local-state/devfinity/runs/default/`.
- `devfinity` generates `process-compose.yaml`, `env`, `urls.txt`, logs
  directories, and a Unix socket path for process-compose control.
- The initial generated process-compose stack includes:
  - Rust build preflight for `finite-saas-core`, `finitechat-server`, and
    `finitesitesd`.
  - Local Postgres for `finite-saas-core`.
  - `finite-saas-core`.
  - Local `finitechat-server`.
  - Local `finitesitesd`.
  - Dashboard dev server.
- Added `just dev` module wrappers: `just dev up`, `just dev status`, and
  `just dev cleanup`.
- Added `docs/local-integration-harness.md`.

Notes:

- `process-compose` is the supervision and visualization layer. `devfinity` is
  the Finite-aware config generator.
- Normal shutdown is foreground lifecycle shutdown: quit the process-compose TUI
  or press Ctrl-C. `devfinity cleanup` is a recovery command for orphaned
  state/processes, including devfinity-managed process trees recorded in pid
  files.
- The richer local create-agent canary remains in
  `finitecomputer-v2/scripts/local_create_agent_canary.sh`; moving that into
  `devfinity` should be a later profile after the base stack is stable.

Validation:

- `nix develop -c process-compose version`
  - Result: passed with process-compose v1.78.0 from the pinned Nix shell.
- `nix develop -c cargo run -p devfinity --locked -- up --dry-run --headless`
  - Result: passed and process-compose validated the generated config.
- `just metadata`
- `just check`
- `just test`
- `cargo test -p devfinity --locked`
- `just dev status`
- `just dev cleanup`
- `cargo fmt --all -- --check`
- `just --unstable --fmt --check`

Not run:

- `just dev up` without `--dry-run`; it starts the long-running local stack
  and depends on dashboard Node/npm readiness.

## Later Repo Import: `finite-identity`

Date: 2026-07-07

Fedimint reference checked:

- Fedimint keeps Rust packages in one root Cargo workspace and uses one root
  lockfile.
- Finite follows that pattern for this import: `finite-identity` is a top-level
  workspace member, and downstream crates consume it through a local workspace
  path instead of a pinned git dependency.

Source snapshot:

| Repo | Source path | Commit SHA | Working tree at record time |
| --- | --- | --- | --- |
| `finite-identity` | `/Users/alex/Projects/finite/finite-identity` | `54a6936b5d7a0e8dc79018a30d9c794b10d25307` | Clean |

Changes:

- Copied `finite-identity` into `finite-mono/finite-identity`.
- Excluded source `.git/` and `target/`.
- Added `finite-identity` as a root Cargo workspace member.
- Replaced the root workspace dependency
  `finite-identity = { git = "...", rev = "54a6936..." }` with
  `finite-identity = { path = "finite-identity" }`.
- Removed the copied `finite-identity/Cargo.lock` after root Cargo resolution
  succeeded, preserving one root `Cargo.lock`.
- Updated root docs navigation to include the `finite-identity` README, spec,
  and CLI conventions.

Validation:

- `cargo metadata --format-version 1 --no-deps`
- `cargo fmt --all -- --check`
- `cargo test -p finite-identity --locked`
- `cargo clippy -p finite-identity --all-targets --locked -- -D warnings`
- `find . -name Cargo.lock -print | sort`
- `cargo metadata --format-version 1 --no-deps --locked`
- `cargo check --workspace --locked`
- `cargo test -p finitechat-core --locked shared_identity -- --nocapture`
- `cargo test -p fsite-cli --locked`

Result:

- `finite-identity` builds and tests as a local workspace package.
- Existing `finitechat-core` shared identity tests pass against the local
  package.
- Existing `fsite-cli` identity tests pass against the local package.
- Only one Cargo lockfile exists: `Cargo.lock` at the monorepo root.

## Later Repo Imports: `finite-nostr`, `finite-auth`, `finite-brain`,
`finite-search`, and `finite-skills`

Date: 2026-07-07

Fedimint reference checked:

- Fedimint keeps Rust packages in one root Cargo workspace and one root
  lockfile, with `just` as the developer command surface and repo scripts for
  larger command implementations.
- Finite follows that pattern here for Rust repos and uses root `just` modules
  for non-Rust repos that already have or need useful local checks.

Source snapshots:

| Repo | Source path | Commit SHA | Working tree at record time |
| --- | --- | --- | --- |
| `finite-nostr` | `/Users/alex/Projects/finite/finite-nostr` | `fefd22b3f3c39481225a28000bba0b2b9354d1ce` | Clean |
| `finite-auth` | `/Users/alex/Projects/finite/finite-auth` | `13347c93650b55be819d37ec77fbc3b50664a432` | Clean |
| `finite-brain` | `/Users/alex/Projects/finite/finite-brain` | `8e1033ce1af54402e6d8feea0f002cbe020b4a35` | Clean |
| `finite-search` | `/Users/alex/Projects/finite/finite-search` | `02d7628922e418405c059753ceaf3449e40a24e7` | Clean |
| `finite-skills` | `/Users/alex/Projects/finite/finite-skills` | `80ada39d477d645eaaacb624e89e0010d3e4aedc` | Clean |

Changes:

- Copied each repo into a top-level monorepo folder with generated/build state
  excluded.
- Added `finite-nostr` as a root Cargo workspace member and workspace
  dependency.
- Added `finite-auth-core` and `finite-auth-store` as root Cargo workspace
  members, removed the copied nested `finite-auth` workspace manifest and
  copied lockfile, and kept `finite-nostr` as a top-level local dependency.
- Added `finite-brain-app`, `finite-brain-cli`, `finite-brain-core`,
  `finite-brain-server`, and `finite-brain-store` as root Cargo workspace
  members. Removed the copied nested `finite-brain` workspace manifest,
  copied lockfile, and duplicate `finite-brain/crates/finite-nostr` package.
- Retargeted `finite-brain` to use the imported top-level `finite-nostr` and
  `finite-identity` workspace packages.
- Added `just search ...` as a root module backed by `finite-search/justfile`.
- Added `finite-skills/justfile` and `finite-skills/scripts/check-static.sh`
  for content validation, then exposed it as `just skills ...`.
- Added missing YAML frontmatter to
  `finite-skills/skills/software-development/publish-web-apps-finite/SKILL.md`
  after the new checker found it.
- Hardened the existing Finite Sites git test helper so synthetic test commits
  disable inherited global GPG signing config.
- Updated root README and docs navigation to include the imported repos and
  relevant local commands.

Validation:

- `cargo metadata --format-version 1 --no-deps`
- `cargo fmt --all -- --check`
- `cargo test -p finite-nostr --locked`
- `cargo clippy -p finite-nostr --all-targets --locked -- -D warnings`
- `cargo test -p finite-auth-core -p finite-auth-store --locked`
- `cargo clippy -p finite-auth-core -p finite-auth-store --all-targets --locked -- -D warnings`
- `cargo test -p finite-brain-core -p finite-brain-store -p finite-brain-server -p finite-brain-cli -p finite-brain-app --locked`
- `cargo clippy -p finite-brain-core -p finite-brain-store -p finite-brain-server -p finite-brain-cli -p finite-brain-app --all-targets --locked -- -D warnings`
- `cargo build -p finite-brain-core -p finite-brain-store -p finite-brain-server -p finite-brain-cli -p finite-brain-app --locked`
- `node --check finite-brain/crates/finite-brain-server/src/product-client.js`
- `node --check finite-brain/crates/finite-brain-server/src/smoke-ui.js`
- `node finite-brain/crates/finite-brain-server/src/product-client.test.js`
- `just search check`
- `just skills check`
- `just --list-submodules --list`
- `just check`
- `just test`
- `git diff --check`

Result:

- Rust repos build and test as local root workspace packages.
- `finite-search` static checks pass through the root `just search check`
  module. Ruby emitted local gem extension warnings before the success line,
  but the check exited successfully.
- `finite-skills` static checks pass across 46 skill files.
- The root command surface now exposes `search` and `skills` modules.
- Full root `just check` and `just test` pass after disabling inherited Git
  commit signing for Finite Sites synthetic test commits.

## Devfinity Native Postgres

Date: 2026-07-07

Changes:

- Added Postgres to the pinned Nix development shell.
- Updated `devfinity` to generate `run-postgres.sh` under the active run
  directory.
- The generated script initializes a local Postgres data directory, starts
  Postgres on the configured devfinity port, and creates `finite_saas_core`
  when needed.
- Updated `devfinity status` and `devfinity cleanup` to report and clean only
  devfinity-managed processes, service probes, and control files.
- Updated the local integration harness docs and phase 9 plan text.

## Full Re-Sync Before Cutover

Date: 2026-07-08

All drifted source repos were re-synced into the monorepo on branch
`migration-integration`, using a three-way merge per repo
(base = recorded import SHA, ours = mono copy, theirs = source `origin/main`).
Mono-side deliberate edits (deleted sub-workspace `Cargo.toml`/`Cargo.lock`,
path-dep retargets to workspace-local `finite-identity`/`finite-nostr`) were
preserved; all source drift was imported. The procedure is now scripted as
`scripts/import-sync`, with per-repo provenance in `scripts/import-sync.toml`.

Synced (one commit per repo on the branch):

| Repo | Range | Notes |
| --- | --- | --- |
| `finitecomputer-v2` | `862e6bf..0150dd9` | merged clean |
| `finite-identity` | `54a6936..0750e8c` | nip98 + authority work; supersedes the git-rev pins that `finite-sites`/`finite-brain` bumped in their source repos (both use the workspace path dep in mono) |
| `finite-sites` | `768a0b8..a6d03e9` | source's only root-manifest change was the finite-identity rev bump (moot in mono) |
| `finite-brain` | `8e1033c..7c66177` | carried source's workspace version bump as explicit `0.1.2` on all five crates; kept mono's workspace-dep retargets in `finite-brain-cli` |
| `finitechat` | `f13c973..6bde2cc` | 13 commits (~18k insertions incl. Electron app, `finitechat-daemon`); new crate added to root workspace members |

No drift (source `origin/main` == recorded import SHA):
`finite-nostr`, `finite-search`, `finite-skills`, `finite-auth`.
Note: earlier drift worries about these repos were an artifact of stale local
checkouts — the "mono-only" nip05/searxng-proxy/skills changes were in fact
pushed to the source repos by the import work.

Validation: `cargo check --workspace` passes; root `Cargo.lock` regenerated
(new crate `finitechat-daemon`, updated deps from finitechat drift).

## Infra Root, Root CI, and Doctrine Flip

Date: 2026-07-08 (branch `migration-integration`, same day as the full re-sync)

- Added `infra/` as the single deploy root, authored from read-only SSH
  captures of the production hosts taken this day. Key corrections vs prior
  belief: no Traefik on lat1 (host Caddy edge); the live finitechat server
  runs on clawland-ovh (15.204.108.57), not lat1; the real ovh-vps-smoke is
  15.204.56.61; finite-brain is NixOS-managed by the legacy fleet flow.
- Moved deploy assets into `infra/`: v2's k8s manifests + runner units +
  finitechat-server deploy script (→ `infra/hosts/lat1/`), finite-sites'
  lat-2 units/Caddyfile (→ `infra/hosts/lat2/`). All in-repo references
  updated; capture-vs-repo divergences recorded in per-host README appendices.
- Moved service image definitions to `infra/images/` (core, dashboard,
  private-limiter), adapted for the mono build context. The limiter image
  building from THIS repo closes the legacy-repo split-brain.
- Added root `.github/workflows/`: ci.yml (fmt/clippy/test vs Postgres,
  dashboard, Hermes bridge, nix checks, devfinity smoke), component-scoped
  release workflows with legacy mirror jobs, service-images, runtime-image
  (single-checkout adaptation), hermes-runtime-smoke. Superseded component
  `.github/workflows` files deleted (they never executed in mono).
- Doctrine flip: `docs/monorepo-doctrine.md` added; root AGENTS.md/README
  rewritten; CONTRIBUTING.md and `compat/matrix.toml` added; the old
  workspace-level "not a monorepo" orientation files updated outside this
  repo.
- Validation: `cargo fmt --check`, `cargo clippy --workspace --all-targets
  --locked -D warnings`, `cargo test --workspace --locked` (276 tests),
  dashboard `npm test` (101 tests) all green locally. Secrets scan (gitleaks
  over full history + targeted greps): 6 findings, all benign fixtures or
  placeholders — cleared for the public visibility flip.

Manual follow-ups this log deliberately leaves open: re-register a lat-2
GitHub Actions runner against finite-mono; set `RELEASE_MIRROR_TOKEN`; first
mono-cut releases proven in the field before archiving source repos; the
clawland→lat1 finitechat-server cutover; smoke-host backups + bind fix.

## Hard Cut, CI Green, lat1 NixOS Config, Runner

Date: 2026-07-08 (later the same day, still on `migration-integration`)

- **Hard-cut release model** (Paul's decision — no live users): mirror jobs
  removed; per-component rolling alias releases (`finitechat-latest`,
  `fsite-latest`, `fbrain-latest`) refreshed by the release workflows;
  install blocks/READMEs/compat matrix point only at finite-mono. Legacy
  repos get archived once their first mono-built release verifies.
- **CI is green** (all five jobs, including the devfinity full-stack smoke).
  Getting there surfaced real debt the drift carried: devfinity had never
  been clippy'd; finitechat's upstream CI had been red since the MLS-welcome
  hard cut (smoke script filtered on a renamed test — passing vacuously;
  unformatted python; RUF015), and a REAL product bug: the agent health
  server trusted a stale pre-cut cached `paired` flag (now downgraded to
  consumed_pending_admission before any probe). Also: devfinity postgres
  pinned its unix socket into the run dir (CI runners can't write
  /run/postgresql); clippy pinned to 1.93.
- **infra/nixos**: flake packages for all server binaries + CLIs and
  `nixosConfigurations.finite-lat-1` — the single-server target from
  finite-fable/single-server-plan.md. Deploy = pinning the flake rev.
  Cutover remains supervised (plan Phase 2).
- **finite-lat-2 runner** `finite-lat-2-mono` registered against finite-mono
  and online (labels self-hosted,Linux,X64,finite-lat-2,docker,nix); repo
  requires approval for outside collaborators (public-repo mitigation).

## lat1 Consolidation Cutover — COMPLETE

Date: 2026-07-09

finite-lat-1 (64.34.82.77) was reinstalled as NixOS from finite-mono
(`infra/nixos/`, host `finite-lat-1`) and now runs the entire coupled
cluster: finite-saas-core, dashboard, native Postgres 16 (finite_core, 87
Finite Private keys restored + verified), the SaaS runner (dormant), the
finitechat server (:8788, migrated from clawland under the single-writer
doctrine), finitesitesd (:8787, migrated from lat2), finite-search, and one
Caddy edge for finite.computer + chat.finite.computer + *.finite.chat
(Cloudflare origin cert). Verified green over real public DNS; Finite Private
validated end-to-end through the Tinfoil limiter.

Topology now:
- **lat1** = the single NixOS app server (deploy = `nixos-rebuild --flake
  ...#finite-lat-1`). k3s/Traefik/on-host-podman-builds are gone.
- **lat2** = the CI runner box (sites/search/tunnel disabled; hosts the
  `finite-lat-2-mono` Actions runner).
- **clawland** = legacy fleet; its finitechat-server is disabled (migrated).
- **smoke** = finite-brain only (deferred from the cutover).

Config facts learned the hard way (all in the flake now): single-disk, no
mdadm (RAID1 superblocks were unassemblable on the pinned nixpkgs kernel);
disks by `/dev/disk/by-id`; WAN bound by MAC (NIC is `enp1s0f1`, not `eno1`).
Procedure + gotchas: `infra/runbooks/lat1-nixos-reinstall.md`. Outcome +
follow-ups: `finite-fable/notes/lat1-cutover-complete-2026-07-09.md`.

Open follow-ups: offsite borg backups (redundancy gap — single-disk root),
disk mirror, brain+oauth2-proxy migration, runner (phala/enclavia), firecrawl
API. Docs across `infra/` were reconciled to this topology the same day.

## Later Repo Import: `finite-specialization`

Date: 2026-07-09

Source:

| Repo | Source path | Commit SHA | Working tree | Remote state |
| --- | --- | --- | --- | --- |
| `finite-specialization` | `/Users/plebdev/Desktop/Projects/finite/finite-specialization` | `54a87aedc2f43dfb794a9ca9b654e42f45c97ecd` | Clean | Matches `origin/main` |

Validation:

- Compared the local source commit with GitHub `refs/heads/main`; both resolve
  to the recorded SHA.
- Ran the source `scripts/check.sh`; result: passed with
  `finite-specialization scaffold looks ok`.
- Ran `finite-specialization/scripts/check.sh` from the copied monorepo tree;
  result: passed with `finite-specialization scaffold looks ok`.
- `cargo fmt --all -- --check`: passed through `scripts/with-dev-env`.
- `just check`: passed for the root Cargo workspace with the lockfile enforced.
- `just test`: passed for the full root Cargo workspace with the lockfile
  enforced.
- `git diff --check`: passed.

Import:

- Imported the 11 tracked files intact under a top-level
  `finite-specialization/` folder using `git archive` at the recorded source
  SHA. Source history, `.git/`, and machine-local files were not copied.
- Removed one redundant trailing blank line from the copied `.gitignore` so
  the root `git diff --check` gate passes. This is the only deliberate
  mono-side content change from the recorded source snapshot.
- Added the imported snapshot to `scripts/import-sync.toml` so future stray
  source commits can use the existing three-way sync process.
- Added the component to the root README and docs navigation.
- The source is currently a docs-and-example-config scaffold. It has no
  `Cargo.toml`, `Cargo.lock`, `flake.nix`, or `justfile`, so the initial import
  required no root Cargo, Nix, lockfile, or `just` changes.
- Preserved `finite-specialization/scripts/check.sh` as the component's initial
  validation loop.
- The deployed `finite-specialization-worker` is not in the scaffold repo. Its
  Rust implementation remains under legacy `finitecomputer`, which the
  monorepo doctrine deliberately keeps outside while the legacy fleet is
  active. Importing the scaffold does not migrate that worker or its deploy
  ownership.

The worker's final code and deployment boundary remains a later legacy-fleet
migration decision rather than an implicit part of this scaffold import.

## Removed Retired `finite-auth` Experiment

Date: 2026-07-09

- Confirmed that no product crate consumed `finite-auth-core` or
  `finite-auth-store`; only root workspace registration and historical docs
  referenced them.
- Deleted the `finite-auth/` source and documentation tree and the superseded
  v2 auth/key-custody brief.
- Removed both crates from the root Cargo workspace and lockfile, and removed
  the `finite-auth` import-sync mapping so the archived source repo cannot be
  accidentally synchronized back into the monorepo.
- Retained the earlier import entries in this migration log and monorepo plan
  as historical facts. `finite-identity` is the active shared identity owner;
  human and agent Nostr identities remain separate.
