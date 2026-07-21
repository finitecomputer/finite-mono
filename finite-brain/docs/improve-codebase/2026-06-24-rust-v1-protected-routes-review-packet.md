# Review Packet: Protected Route Handling

## Scope

- Loop: Improve Codebase
- Selected candidate: Deepen Protected Route Handling
- Fixed point: `2454979`
- Branch: `feature/rust-portable-v1-core`

## Intent

Concentrate protected-route mechanics behind one server module without changing
the FiniteBrain product behavior.

The route catalog in `crates/finite-brain-server/src/lib.rs` should describe
brain, folder, sharing, object, and sync operations. Nostr HTTP auth parsing,
expected URL/body validation, replay checks, route rate limits, and CORS
allowlist response shaping should live together in
`crates/finite-brain-server/src/protected_routes.rs`.

## Review Notes

- Behavior should stay equivalent for all existing protected endpoints.
- `validate_request_auth` now returns the normalized actor npub so handlers do
  not repeat `NostrPublicKey` conversion and auth-error mapping.
- CORS allowlist behavior still derives allowed origins from `PUBLIC_BASE_URL`
  and rejects disallowed preflight origins.
- Replay-cache and rate-limit state still use the existing `ServerState`
  fields.
- Sub-agent review was intentionally skipped because the current tool policy
  requires explicit user delegation before spawning sub-agents.

## Verification

```sh
cargo fmt --check
cargo test -p finite-brain-server protected_create -- --nocapture
cargo test -p finite-brain-server cors_preflight_is_allowlist_driven -- --nocapture
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
git diff --check
node --check crates/finite-brain-server/src/product-client.js
node crates/finite-brain-server/src/product-client.test.js
```
