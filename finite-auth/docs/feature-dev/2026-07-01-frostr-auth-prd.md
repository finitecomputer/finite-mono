# PRD: Frostr-Aware Auth Model

Status: Historical artifact. The Frostr keyset placement remains useful, but
ADR 0003 and `2026-07-01-shared-frostr-signer-prd.md` supersede the delegated
agent-key model described here.

## Problem Statement

Finite auth needs to support Nostr-first identities while avoiding a
single-device, single-secret account model. Users should be able to sign in and
recover signing capability through a simple 2-of-3 Frostr setup, and agents
should have their own Nostr keypairs when acting on a user's behalf.

## Solution

Add Frostr-aware domain and store scaffolding to finite-auth. The first pass
captures the product model without depending directly on bifrost-rs:

- A user's primary Finite identity can be a FROSTR group public key.
- The standard setup is fixed to 2-of-3 shares.
- Share placements are server, user client, and native secure storage.
- The server records metadata for its share and stores only a bounded package
  reference at this layer.
- Agents have independent Nostr public keys delegated to a user public key.
- Future bifrost-rs integration owns cryptographic package contents, signer
  runtime, relay transport, and native/browser bridge behavior.

## User Stories

1. As a FiniteBrain user, I want my account signer to survive loss of one
   device or service, so that I can keep access without a single secret.
2. As a FiniteBrain user, I want a 2-of-3 setup with server, browser/client,
   and native secure storage shares, so that the recovery story is predictable.
3. As a FiniteBrain user, I want agents to have their own Nostr keys, so that
   agent actions can be attributed separately from my primary key.
4. As a FiniteBrain server, I want bounded Frostr keyset metadata, so that auth
   state is queryable and constrained.
5. As a future client implementer, I want finite-auth to name the bifrost-rs
   boundary clearly, so that crypto/session runtime is not reinvented here.

## Implementation Decisions

- `finite-auth-core` owns Finite policy types for Frostr keyset placement and
  agent key delegation.
- `finite-auth-store` owns durable metadata for Frostr keysets and delegated
  agent keys.
- The first scaffold does not store raw FROSTR share material. It stores a
  bounded package reference/handle so the secret package format can later come
  from bifrost-rs or platform storage without schema churn.
- The hard-coded first policy is 2-of-3, with exactly one server share, one
  user-client share, and one native-secure-storage share.
- The group public key is treated as the user's primary public-key identity.
  Agent keys are distinct delegated actors.
- No direct bifrost-rs crate dependency is added yet; bifrost-rs is beta and
  already separates host-owned UX/storage from crypto/runtime concerns.

## Testing Decisions

- Core tests validate the fixed 2-of-3 share placement model, role uniqueness,
  member index bounds, and agent key separation.
- Store tests validate restart-safe persistence for Frostr keyset metadata and
  delegated agent key records.
- Existing auth challenge/session/NIP-05 tests remain the regression baseline.

## Out of Scope

- Running a live Frostr ceremony.
- Storing decrypted server share material.
- Relay publishing/querying.
- Native keychain integration.
- Browser WASM bridge integration.
- Recovery UX.
- PR publication, because this repo has no remote and this run is main-only.

## Further Notes

Sources reviewed:

- <https://frostr.org/>
- <https://github.com/FROSTR-ORG/bifrost-rs>
- <https://github.com/FROSTR-ORG/bifrost>
