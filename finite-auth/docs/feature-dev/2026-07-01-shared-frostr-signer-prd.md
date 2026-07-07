# PRD: Shared Frostr Signer For Users And Agents

## Status

Local PRD artifact. No remote issue tracker is configured for this repo.

## Problem Statement

FiniteBrain wants agents to act on a user's behalf with the same Nostr identity
the user controls. The current scaffold models a Frostr group public key as a
user primary key, but it also models agents as separate delegated Nostr keys.
That splits the identity model in the wrong place.

The target model is one shared Nostr signer by default: a user's Frostr group
public key. A user, an agent runtime, or a native client can request signatures
only when Finite policy and a two-share Frostr quorum allow it.

## Solution

Define the shared signer model and prepare the implementation migration:

- The user primary key is the Nostr public key used by the human and default
  agents.
- With Frostr enabled, the user primary key is the Frostr group public key.
- The standard signup flow creates a 2-of-3 keyset on the client side.
- The active client keeps one share, the cold backup share is stored in native
  secure secret storage, and only the server share is sent to the server.
- Agents are represented as signing sessions and audit actors, not separate
  default Nostr public keys.
- The native secure-storage share is held behind a host adapter such as Apple
  Keychain, Android Keystore, Windows Credential Locker, Freedesktop Secret
  Service, or an equivalent platform facility.
- Rust host adapters should prefer the `keyring` ecosystem: use `keyring` for
  simple desktop native secret read/write, and `keyring-core` plus explicit
  store crates when backend selection matters.
- Bifrost-rs owns FROST signing, package utilities, encrypted Nostr transport,
  runtime readiness, and native/browser bridges. Finite-auth owns policy,
  metadata, session authorization, and audit facts.

## User Stories

1. As a FiniteBrain user, I want my agent to sign as my Nostr identity when I
   authorize it, so that work performed on my behalf uses the same public key I
   control.
2. As a FiniteBrain user, I want the server to hold only one Frostr share, so
   that the service cannot sign alone.
3. As a FiniteBrain user, I want my active client to hold the normal signing
   share and native secure storage to hold the cold backup share, so that I can
   recover or rotate if the active client share is lost.
4. As a FiniteBrain operator, I want agent accountability to live in audit
   records, so that we can tell human and agent actions apart even when NIP-01
   sees one public key.
5. As a future client implementer, I want a narrow adapter boundary around
   bifrost-rs and native storage, so that finite-auth does not duplicate crypto
   or platform-specific secret handling.

## Implementation Decisions

- Replace delegated-agent-key vocabulary with Shared User-Agent Signer and
  Agent Signing Session vocabulary in durable context.
- Keep the first Frostr policy fixed at 2-of-3 until a later decision expands
  quorum options.
- Model share placement as server, user client, and native secure storage.
- Model routine signing as server plus active user-client share.
- Model native secure storage as a cold backup/recovery participant, not a
  routine signer.
- Treat native secure storage unlock and backup/sync policy as host-owned. The
  default product posture is recovery-only, not silent background signing.
- Store bounded references to share packages, not decrypted share contents.
- Keep NIP-01 and NIP-05 verification delegated to finite-nostr.
- Keep FiniteBrain product concepts outside finite-auth except where they
  appear as consumer-facing examples.

## Testing Decisions

- Domain tests should prove that the shared signer is one public key and that
  agent sessions cannot introduce a second default Nostr identity.
- Store tests should prove signing-session metadata survives restart and is
  queryable by user primary key and agent runtime identifier.
- Policy tests should prove server-only, client-only, and native-only signing
  cannot be represented as authorized quorums.
- Policy tests should prove normal signing does not consume the cold backup
  share.
- Adapter tests should use fakes for bifrost-rs and native storage until live
  ceremony integration is intentionally introduced.
- Existing challenge, session, NIP-05, and NIP-01 wrapper behavior remains the
  regression baseline.

## Out Of Scope

- Running a live FROSTR ceremony.
- Storing decrypted server share material.
- Choosing production server-side key management.
- Shipping Apple Keychain, Android Keystore, Windows Credential Locker,
  Freedesktop Secret Service, Rust `keyring`, or browser storage adapters.
- Implementing unattended background agent signing.
- Opening a staging PR, because this run is intentionally main-only and the
  repo has no remote issue tracker or PR target configured.

## Source Notes

- Bifrost-rs presents itself as the FROSTR stack for threshold signing,
  collaborative ECDH, encrypted Nostr peer messaging, hosted runtimes, and
  onboarding/recovery/backup helpers.
- The FROSTR overview describes M-of-N shareholder collaboration that produces
  a normal Schnorr signature for one public key.
- RFC 9591 describes FROST as a threshold Schnorr signing protocol requiring
  cooperation from a threshold number of participants and validation of network
  inputs.
- Apple platform documentation describes Keychain as storage for small secrets
  such as keys and tokens, with access classes and Secure Enclave-supported
  protections available to host adapters.
- Android Keystore, Windows Credential Locker, and Freedesktop Secret Service
  provide analogous platform-native secure storage surfaces.
- The Rust `keyring` crate provides a simple cross-platform desktop API for
  reading and writing native secrets; applications that need backend control
  should use `keyring-core` with specific store crates.
