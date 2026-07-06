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

- Kept `just default` as `just --list`.
- Added `just metadata` for root Cargo workspace metadata validation.
- Added `just check` for `cargo check --workspace --locked`.
- Added `just test` for `cargo test --workspace --locked`.
- Left dashboard, chat, sites, CI, release, deploy, and formatter commands out
  of the root command surface for now.

Validation:

- `just --list`
- `just metadata`
- `just check`
- `just test`
