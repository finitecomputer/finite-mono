# NIP-01 and NIP-05 Auth Scaffold

## Primary Sources

- NIP-01: <https://github.com/nostr-protocol/nips/blob/master/01.md>
- NIP-05: <https://github.com/nostr-protocol/nips/blob/master/05.md>

## NIP-01 Rules Used By finite-auth

- The stable authenticated Nostr identity is the event public key.
- Signed event acceptance must validate the deterministic event ID and Schnorr
  signature before deriving a Finite principal.
- Event parsing and verification are delegated to `finite-nostr` so this repo
  does not create a second event serialization implementation.

## NIP-05 Rules Used By finite-auth

- Identifiers are split into `<local-part>@<domain>`.
- The local part is restricted to `a-z0-9-_.`.
- The lookup URL is
  `https://<domain>/.well-known/nostr.json?name=<local-part>`.
- The response document maps names to lowercase hex public keys under
  `"names"`.
- Optional `"relays"` data may be attached to the mapped public key.
- Public keys stay primary. A NIP-05 identifier is a current display/search
  binding and must not replace a stored public key.
- Redirects from the well-known endpoint are not allowed and must be ignored by
  fetchers.

## Initial finite-auth Flow

1. A Finite service issues an auth challenge with an explicit nonce, URL,
   method, issue time, and expiry.
2. A client signs a NIP-98-style HTTP auth event through a Nostr signer.
3. `finite-auth-core` validates request facts through `finite-nostr`, checks
   the expected challenge nonce, and returns a public-key principal.
4. `finite-auth-store` consumes the challenge in a transaction. Reuse is a
   replay.
5. The service creates a bounded session and persists only a session token
   hash.
6. Optional NIP-05 binding checks can attach an identifier to the principal,
   but authorization decisions still use the public key.

## Frostr Consideration

Frostr adds a threshold signing path for the same NIP-01 public-key identity
model. The first Finite policy is a fixed 2-of-3 keyset:

- server share;
- user-client share;
- native secure-storage share, used as a cold backup share.

The group public key is the user's primary Finite/Nostr identity. Per-agent
Nostr keypairs are not the default model. A FiniteBrain agent acts through the
same group public key when the user has granted an Agent Signing Session. NIP-01
observers see the user's public key; Finite audit records distinguish the human
or agent runtime that requested the signature.

Signup is client-orchestrated: the client creates the 2-of-3 setup, keeps the
active user-client share, stores the third share in native secure secret
storage for backup/recovery, and sends only the server share to the server.
Normal signing should use the server share plus active user-client share. The
native secure-storage share is reserved for recovery or rotation unless a later
ADR explicitly changes that policy.

`finite-auth` should keep the bounded policy and durable metadata for keysets,
share placements, signing sessions, and audit facts. Cryptographic ceremony,
package formats, relay messaging, signer readiness, and native/browser bridge
runtimes should be owned by bifrost-rs or host-specific adapters.
