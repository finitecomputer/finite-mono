# ADR 0002: Model Frostr Keysets As User Primary Signers

## Status

Accepted for Frostr keyset placement. Superseded in part by ADR 0003 for the
agent identity and action-attribution model.

## Context

Finite auth is Nostr-first, and FiniteBrain needs users and agents to operate
through Nostr public keys. Frostr introduces a threshold signing path where a
single public key can be controlled by multiple shares. The desired first setup
is 2-of-3: the server stores one share, the user's client stores one share, and
native keychain or secure storage stores one share.

bifrost-rs is a Rust implementation of the FROSTR stack. It owns FROST
threshold signing over secp256k1, collaborative ECDH, Nostr-native encrypted
peer messaging, native/browser bridge runtimes, and `frostr-utils` package,
onboarding, recovery, and backup helpers. Its README also states that
host-specific operator UX belongs in consuming projects.

## Decision

`finite-auth` treats a Frostr group public key as a possible User Primary Key.
The initial Finite policy is fixed to 2-of-3 with these placements:

- server share;
- user-client share;
- native secure-storage share.

`finite-auth` owns bounded policy and durable metadata:

- the group public key;
- fixed threshold and member count;
- share placement roles and member indexes;
- bounded share package references;
- signing-session and audit facts for actors that request signatures.

The first implementation scaffold used delegated agent Nostr key bindings as an
attribution mechanism. ADR 0003 replaces that target with shared user-agent
signing sessions.

`finite-auth` does not own Frostr ceremony cryptography, package codecs, relay
transport, signer runtime readiness, or native/browser bridge implementations.
Those should come from bifrost-rs or host adapters when integration moves past
scaffolding.

## Consequences

The user's primary identity remains public-key-first and works with NIP-01
verification. Losing any one share should not prevent signing once Frostr
runtime integration exists. Agent actions should resolve to the same NIP-01
public key as the user by default; Finite-owned audit data carries the agent
runtime attribution.

The server share is treated as highly sensitive material. The current scaffold
stores only package references, not decrypted share contents. A future slice
must choose the server share encryption, key management, and runtime signing
adapter before storing usable share material.
