# Issue #38 Session: fbrain Agent CLI

## Scope

- Added `crates/finite-brain-cli` with binary name `fbrain`.
- Implemented agent-native command families:
  - `auth status|login|logout`
  - `signer status|public-key|sign|encrypt|decrypt`
  - `open`, `status`, `doctor`
  - `daemon status|start|stop|logs|tick`
  - `sync status|now`
  - `unlock`, `conflicts`, `resolve`, `activity`, `access explain`
  - `brain create|metadata|export`
  - `folder create`
  - `permissions add-member|remove-member|add-admin|remove-admin|grant-folder`
  - `invites create|show|accept|revoke`
  - `share link|accept|revoke|source|folder-invite|folder-accept`
- Added Brain Working Tree state files and local `FBRAIN_CONFIG_DIR` signer state.
- Added signed HTTP client for local `http://` FiniteBrain servers using NIP-98-style Nostr auth.
- Added NIP-44 signer encrypt/decrypt and NIP-59 wrapped Folder Key Grant creation for CLI admin/share flows.
- Added automatic sync attempts on `open` and `daemon start`, with strict diagnostic sync through `sync now`.
- Fixed server issues surfaced by live CLI smoke:
  - Bootstrap Folder Key Grant ids now include Brain id, avoiding collisions across multiple Personal Brains owned by the same npub.
  - Personal Brain owners can pass the admin mutation gate for owner-scoped Folder creation.

## Files

- `Cargo.toml`
- `Cargo.lock`
- `CONTEXT.md`
- `README.md`
- `crates/finite-brain-cli/Cargo.toml`
- `crates/finite-brain-cli/src/lib.rs`
- `crates/finite-brain-cli/src/main.rs`
- `crates/finite-brain-server/src/lib.rs`
- `crates/finite-brain-server/src/routes/brains.rs`

## Verification

- `cargo fmt --check`
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p finite-brain-cli`
- `cargo test -p finite-brain-server`
- `cargo test --workspace`
- `cargo build`
- `git diff --check`
- Live smoke on `127.0.0.1:4017` with temp SQLite:
  - `fbrain auth login`
  - `fbrain brain create` twice for the same owner
  - `fbrain open` with automatic sync status `caught-up`
  - `fbrain status --json`
  - `fbrain folder create notes`
  - repeated `fbrain brain metadata` without replay failure

## Residual Hardening

- The prototype HTTP client supports `http://` only.
- `daemon start` records state and sync attempts, but does not yet spawn a resident background process.
- Automatic file-watch encrypted object writeback is not implemented in this slice.
- Secret storage is a prototype local file with `0600` permissions on Unix, not platform keychain storage.
