# ADR 0003: Keep Folder Object Crypto In finite-brain-core

Status: accepted

Date: 2026-06-23

## Context

FiniteBrain Portable v1 uses two different crypto categories:

- Generic Nostr/NIP primitives: NIP-19 identity encoding, Nostr event
  serialization and verification, NIP-44 encryption/decryption adapters,
  NIP-59 wrapping helpers, and NIP-98-style HTTP authorization helpers.
- FiniteBrain Folder Object crypto: AES-256-GCM envelopes, Folder Keys,
  FiniteBrain AAD construction, ciphertext hashing, and Folder Key Grant
  plaintext validation.

Folder Object encryption is bound to FiniteBrain policy because its AAD
includes `brainId`, `folderId`, `objectId`, and `keyVersion`.

## Decision

`finite-nostr` owns only generic Nostr/NIP primitives.

`finite-brain-core` owns Folder Key types, AES-256-GCM Folder Object envelope
construction, AAD construction, ciphertext hashing, and FiniteBrain-specific
Folder Key Grant plaintext validation.

`finite-brain-store` stores ciphertext, envelopes, signed events, and grant
metadata, but does not decrypt Folder Objects.

## Consequences

- Other Finite repos can reuse Nostr helpers without inheriting FiniteBrain
  Brain or Folder concepts.
- FiniteBrain's product crypto can validate its own domain-specific context.
- Store and server code remain policy callers rather than crypto owners.
