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
