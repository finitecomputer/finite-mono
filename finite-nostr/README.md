# finite-nostr

Reusable Nostr primitives for Finite Rust projects.

This crate owns generic Nostr helpers and wrappers that can be reused by
FiniteBrain, Finite Chat, and other Finite repos. It should not contain
FiniteBrain vault, folder, sync, or sharing policy.

Initial target primitives:

- NIP-19 public key encoding/decoding.
- NIP-07-compatible event authorization helpers.
- NIP-44 encryption/decryption adapters.
- NIP-59 gift-wrap/seal/rumor helpers.
- Deterministic event validation and typed errors.

## Current API

The crate currently wraps the lower-level `nostr` protocol crate with:

- `NostrPublicKey` for hex/npub parsing and formatting.
- `EventIdHex` and `compute_event_id` for deterministic NIP-01 event IDs.
- `verify_event_integrity` for event ID and signature validation.
- `HttpAuthEventRequest`, `sign_http_auth_event`, header encode/decode helpers,
  `HttpAuthValidation`, and `validate_http_auth_event` for NIP-98-style request
  authorization flows.
- `encrypt_nip44` and `decrypt_nip44` for caller-provided NIP-44 payload
  encryption.
- `build_rumor`, `seal_rumor`, `wrap_rumor`, and `open_gift_wrap` for
  product-neutral NIP-59 rumor/seal/gift-wrap flows.
- `NostrPrimitiveError` for stable typed failures.

FiniteBrain-specific vault, folder, grant, invitation, and share policies stay
out of this crate.

## Development

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```
