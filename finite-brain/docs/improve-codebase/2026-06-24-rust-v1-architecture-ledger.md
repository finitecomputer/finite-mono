# Improve Codebase Ledger: Rust v1 Architecture Candidates

## Run

- Run ID: `2026-06-24-rust-v1-architecture`
- Loop: Improve Codebase
- Target repo: `finitecomputer/finite-brain`
- Base branch: `staging`
- Improvement branch: `feature/rust-portable-v1-core`
- Status: selected structural slice implemented and ready for review

## Discovery Input

- Domain glossary: `CONTEXT.md`
- ADRs: `docs/adr/0001-adopt-rust-workspace-and-finite-nostr.md`,
  `docs/adr/0004-build-a-first-party-product-client.md`,
  `docs/adr/0005-derive-graph-and-replay-from-client-decrypted-indexes.md`,
  `docs/adr/0006-keep-okf-import-execution-client-owned.md`
- Hot modules:
  - `crates/finite-brain-server/src/lib.rs`
  - `crates/finite-brain-store/src/lib.rs`
  - `crates/finite-brain-core/src/portability.rs`
  - `crates/finite-brain-server/src/product-client.js`

## Candidate Report

- Local HTML report:
  `/tmp/architecture-review-finite-brain-20260624.html`
- The report stays outside the repo as required by the Improve Codebase loop.

## Candidates

| Candidate | Recommendation | Why |
| --- | --- | --- |
| Deepen Protected Route Handling | Strong | Concentrates Nostr auth, replay resistance, rate limiting, CORS, and request/error handling behind one server module without changing product behavior. |
| Deepen Store Sharing And Mounts | Worth exploring | Concentrates Share Link, Shared Folder Invitation, connection member, key rotation, and mount projection lifecycle rules. Higher risk than the server guard slice. |
| Deepen Sync Projection | Worth exploring | Concentrates append-log, duplicate event, baseRevision conflict, pagination, retention, and current-state projection behavior. Medium behavior risk. |
| Split Portable Readable Surfaces | Speculative | Separates OKF and Vault Working Tree readable portability surfaces if future changes keep increasing churn in `portability.rs`. |

## Top Recommendation

Start with **Deepen Protected Route Handling**.

Reason: it has the best locality/leverage ratio for a first structural slice.
It should reduce route-test friction while preserving the Product Client and
server behavior already proven by the v1 parity gates.

## Human Gate

The human delegated the first implementation slice to Codex and approved the
top recommendation:

- Selected candidate: **Deepen Protected Route Handling**
- Selection note: "you lead the way here the human gives you authority"

## Implemented Slice

The protected-route auth and CORS behavior now lives in
`crates/finite-brain-server/src/protected_routes.rs`.

This slice preserves behavior while moving the Nostr HTTP auth decoding,
expected URL/body validation, replay cache, protected-route rate limit, and CORS
allowlist response handling out of the route catalog in
`crates/finite-brain-server/src/lib.rs`.

Handlers now receive the normalized actor npub from `validate_request_auth`
instead of handling `NostrPublicKey` conversion and auth-error mapping inline.
That keeps route code focused on product operations like vault membership,
folder grants, sharing, mounted folders, object writes, and sync records.

## Verification So Far

```sh
git diff --check
grep -E 'id="(protected-route-module|store-sharing-module|sync-projection-module|portability-module|top-recommendation)"' /tmp/architecture-review-finite-brain-20260624.html
cargo fmt --check
cargo test -p finite-brain-server protected_create -- --nocapture
cargo test -p finite-brain-server cors_preflight_is_allowlist_driven -- --nocapture
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
node --check crates/finite-brain-server/src/product-client.js
node crates/finite-brain-server/src/product-client.test.js
```

## Parked Notes

- Runtime wording in `crates/finite-brain-app/src/main.rs` still says
  "smoke server"; change only if selected as an implementation cleanup or if a
  future context pass approves runtime wording changes.
- Original local checkout content reads blocked during this run. The branch work
  was prepared and pushed from `/tmp/finite-brain-context-work`.
