# ADR 0003: Share User-Agent Signer Through Frostr

## Status

Accepted.

## Date

2026-07-01.

## Context

FiniteBrain needs agents to act on a user's behalf in a Nostr-native way. The
earlier scaffold treated agents as separate delegated Nostr public keys. The
desired model is sharper: the user and the user's agents share the same Nostr
identity by default, and that identity is backed by a Frostr group public key.

FROSTR and bifrost-rs make this credible because threshold participants can
produce a standard Schnorr signature for one public key without any one
participant holding the whole secret. Bifrost-rs also keeps runtime, package,
relay, and host integration concerns outside the Finite policy layer.

## Decision

The User Primary Key is the Nostr public key that FiniteBrain users and agents
use by default. When Frostr is enabled, that key is the Frostr group public key.

Agents do not receive separate default Nostr keypairs for acting on behalf of
the user. Instead, Finite records an Agent Signing Session that authorizes a
specific agent runtime to request signatures as the user's primary key for a
bounded scope and time.

The first Frostr setup remains fixed at 2-of-3:

- server share;
- user-client share;
- native secure-storage share, used as the user's cold backup share.

Signup is client-orchestrated. The client creates the 2-of-3 setup, keeps the
active user-client share, writes the cold backup share into native secure
secret storage, sends only the server share to the server, and discards any
temporary access to shares it does not own after packaging succeeds.

Routine signing should use two shares without ever reconstructing the secret
key. The preferred paths are:

- user-present web/client signing: server share plus user-client share;
- default agent signing on an active user client: server share plus user-client
  share, with an Agent Signing Session carrying the Finite audit identity;
- recovery or device replacement: server share plus cold backup share, subject
  to product policy and additional verification.

The native secure-storage share is not a routine signer and not a silent
background signer by default. Host adapters may use Apple Keychain, Android
Keystore, Windows Credential Locker, Freedesktop Secret Service, or equivalent
native storage. Rust host adapters should prefer the `keyring` ecosystem:
`keyring` for simple desktop secret read/write, or `keyring-core` plus specific
store crates when the app must control the exact backend. Unlock,
user-presence, backup/sync, and backend policy belong to the host adapter, not
core cryptographic identity.

`finite-auth` owns:

- keyset policy;
- share placement metadata;
- package references, not raw decrypted share contents;
- signing-session policy;
- audit facts that distinguish user actions from agent actions.

`finite-auth` does not own:

- FROST ceremony cryptography;
- bifrost package codecs;
- relay publish/query mechanics;
- signer runtime readiness;
- native/browser bridge storage implementations.

## Consequences

NIP-01 observers see one public key for both the human and default agents. This
is intentional: the agent is acting as the user cryptographically. FiniteBrain
must keep first-class audit records for which agent runtime requested a
signature, what scope allowed it, and which quorum path signed it.

The current `AgentNostrKeyBinding` code and store table are now a migration
target. They can remain as historical scaffold until the implementation slice
replaces them with signing-session records.

The server share remains highly sensitive and insufficient alone. Server-side
storage must still choose encryption, key management, and operational controls
before usable share material is stored.

## References

- <https://github.com/FROSTR-ORG/bifrost-rs>
- <https://github.com/FROSTR-ORG/bifrost>
- <https://datatracker.ietf.org/doc/rfc9591/>
- <https://support.apple.com/guide/security/keychain-data-protection-secb0694df1a/web>
- <https://support.apple.com/guide/security/the-secure-enclave-sec59b0b31ff/web>
- <https://developer.android.com/privacy-and-security/keystore>
- <https://learn.microsoft.com/en-us/windows/apps/develop/security/credential-locker>
- <https://specifications.freedesktop.org/secret-service/>
- <https://docs.rs/keyring>
