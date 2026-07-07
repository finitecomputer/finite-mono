# finite-auth Context

## Glossary

### Authentication Policy

Finite-owned rules that decide whether a Nostr signer becomes an authenticated
Finite principal. Examples include challenge replay handling, session creation,
NIP-05 binding state, and consumer-facing auth errors.

### Reusable Nostr Primitive

A generic protocol operation owned by `finite-nostr`, such as NIP-01 event ID
and signature verification, NIP-19 public key parsing, NIP-44 encryption,
NIP-59 wrapping, or NIP-98 HTTP authorization event validation.

### Finite Principal

The authenticated actor Finite services receive after a Nostr request is
validated. The stable identity is the Nostr public key. A NIP-05 identifier may
be attached only as a current identification binding.

### User Primary Key

The Nostr public key Finite treats as the user's durable identity. It may come
from a normal single-device signer or from a Frostr group public key. Product
authorization should follow this key, not a device, NIP-05 identifier, or
individual Frostr share.

### Frostr Keyset

A threshold signing setup for one User Primary Key. The initial Finite policy is
2-of-3: any two of the server share, user-client share, and native
secure-storage share can produce signatures for the same group public key.
Agents acting for the user request signatures through this same keyset by
default.

### Frostr Share Placement

The role that holds one share in a Frostr Keyset. The first supported
placements are Server Share, User Client Share, and Native Secure Storage
Share.

### Cold Backup Share

The Native Secure Storage Share's product purpose. During signup, the client
creates the 2-of-3 keyset, keeps the active User Client Share locally, sends
only the Server Share to the server, and writes the third share into native
secure secret storage as a user backup. This share is not part of routine
signing or default agent operation.

### Shared User-Agent Signer

The default signing model for FiniteBrain agents. The user and the user's
agents share one Nostr identity: the User Primary Key, usually backed by a
Frostr Keyset. Agent accountability is recorded as Finite authorization and
audit metadata, not as a distinct default Nostr public key.

### Agent Signing Session

A bounded authorization that lets an agent request signatures as the Shared
User-Agent Signer for a specific user, scope, and runtime. It is an application
principal for policy and audit. It is not a separate Nostr identity.

### Native Secure Storage Share

The Frostr share held by the host environment's secure storage facility, such
as Apple Keychain, Android Keystore, Windows Credential Locker, or a
Freedesktop Secret Service provider. In Rust host apps, the `keyring` ecosystem
is the preferred adapter surface for desktop-style native secret stores. This
share is the Cold Backup Share: it is used for recovery and rotation, not as a
silent background signer. `finite-auth` records policy and references for it;
host adapters own the platform storage mechanics.

### NIP-05 Binding

A server-observed statement that a NIP-05 well-known document currently maps an
identifier to a Nostr public key. It is not a durable account key and must not
replace the public key as the primary identity.

### Auth Challenge

A bounded, server-issued nonce that is consumed after a matching signed Nostr
HTTP authorization event is accepted. Reuse is a replay and must be rejected.

### Auth Session

A bounded Finite session derived from an accepted Nostr authentication event.
Stores persist only token hashes, not bearer tokens.

### FiniteBrain Consumer

The first expected consumer of this repo. FiniteBrain owns Vault, Folder,
Folder Key Grant, sharing, and OKF policy; `finite-auth` should return
principals and session facts without depending on those product concepts.
