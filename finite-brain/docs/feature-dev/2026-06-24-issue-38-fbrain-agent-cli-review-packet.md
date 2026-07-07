# Issue #38 Review Packet: fbrain Agent CLI

## Review Mode

Direct review in current thread. Subagent review was skipped because the user did not explicitly request subagents for this implementation pass.

## Security/Correctness Review

- Checked Nostr HTTP auth event generation against `finite_nostr::validate_http_auth_event`.
- Added a nonce tag to CLI auth events after live smoke exposed same-second replay collisions on repeated signed GET requests.
- Verified signed server routes for Vault create, metadata, open sync, and Folder create against a live local app server.
- Verified local signer supports public key, signing, NIP-44 encrypt, and NIP-44 decrypt.
- Verified local signer state uses Unix `0600` file permissions.
- Fixed server bootstrap grant-id collision across multiple personal Vaults for the same owner.
- Fixed server personal-owner authorization for owner Folder creation.

## Domain/API Review

- Command name is `fbrain`.
- Terminology uses Vault Working Tree, not Volumes.
- Normal flow attempts automatic sync on `open` and `daemon start`; `sync now` is diagnostic/recovery.
- Agents keep using normal filesystem tools for wiki reads/writes while `fbrain` owns identity, sync, grants, permissions, invites, and sharing controls.
- JSON output exists for status-bearing and server-response commands through `--json`.

## Verification Result

Pass.

## Commands

- `cargo fmt --check`
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p finite-brain-cli`
- `cargo test -p finite-brain-server`
- `cargo test --workspace`
- `cargo build`
- `git diff --check`
- Live smoke on `127.0.0.1:4017` with temp SQLite.

## Follow-Ups

- Replace prototype secret-file storage with a platform secret backend.
- Add resident daemon process supervision and file-watch encrypted object writeback.
- Add HTTPS support to the CLI HTTP transport.
- Split the CLI crate into smaller modules once behavior stabilizes.
