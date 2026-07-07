# finite-auth

Nostr-first authentication policy for Finite Rust services.

`finite-auth` is mainly intended for FiniteBrain. It depends on
`finite-nostr` for reusable Nostr protocol primitives and keeps Finite auth
state in a separate, transaction-backed store.

## Status

This repo is scaffolded and model-driven. It currently contains:

- NIP-01/NIP-05/NIP-98-oriented auth challenge and session policy.
- SQLite storage for challenges, sessions, NIP-05 bindings, and Frostr keyset
  metadata.
- Frostr 2-of-3 share-placement scaffolding.
- Documentation for the target shared user-agent signer model.

Important audit note: the older `AgentNostrKeyBinding` code path still exists
as implementation scaffold. ADR 0003 supersedes that model. The target model is
Agent Signing Sessions under one shared user primary key, not separate default
agent Nostr identities.

## Workspace

| Crate | Purpose |
| --- | --- |
| `finite-auth-core` | Domain types and validation policy for challenges, sessions, NIP-05 bindings, Nostr HTTP auth, Frostr keysets, and the current agent-key scaffold targeted for shared signer migration. |
| `finite-auth-store` | SQLite transaction boundary for challenge/session/NIP-05 storage and Frostr keyset metadata. |

The workspace uses Rust 2024 and forbids unsafe code through workspace lints.

## Auth Model

- The stable Finite principal is a Nostr public key.
- NIP-05 is an identification binding, not the account key.
- NIP-05 well-known fetches must use
  `https://<domain>/.well-known/nostr.json?name=<local-part>`, cap response
  size, and reject redirects.
- NIP-01 event ID and signature verification belongs to `finite-nostr`.
- NIP-98-style HTTP authorization events are accepted through the
  `finite-nostr` wrapper and can be bound to a finite-auth challenge nonce.
- Session stores persist bearer token hashes, not bearer tokens.

## Frostr Model

The target Frostr setup is a client-orchestrated 2-of-3 keyset:

1. The client creates the Frostr keyset and group public key during signup.
2. The active client keeps the User Client Share for normal signing.
3. Native secure secret storage receives the third share as the user's Cold
   Backup Share.
4. The client sends only the Server Share package to the server.
5. Temporary access to shares the client does not own is discarded after
   packaging succeeds.

The Frostr group public key is the User Primary Key. FiniteBrain agents use
that same public key by default. Agent accountability lives in Finite
authorization/session/audit records, not separate default Nostr public keys.

Normal signing uses the server share plus the active user-client share. The
Cold Backup Share is for recovery or rotation. It is not routine signing
material and is not an unattended-agent-signing mechanism.

## Native Secret Storage

There is no one universal native secure secret storage API. The Finite standard
is an adapter over each platform's native store:

| Platform | Native store |
| --- | --- |
| Apple | Keychain Services, with access classes and optional user-presence ACLs |
| Android | Android Keystore / KeyChain |
| Windows | Credential Locker / Credential Manager, or DPAPI for protected blobs |
| Linux desktop | Freedesktop Secret Service providers such as GNOME Keyring or KWallet |
| Rust desktop hosts | `keyring` ecosystem |

Rust host adapters should prefer `keyring` for simple platform-independent
desktop secret read/write. Use `keyring-core` plus explicit store crates when
the app must control the exact backend, access behavior, or platform coverage.
Keep this adapter outside `finite-auth-core`.

## Boundaries

`finite-auth` owns:

- Finite authentication policy.
- Challenge replay handling and session facts.
- NIP-05 binding state.
- Frostr keyset policy, share-placement metadata, and bounded package
  references.
- Shared user-agent signer policy and audit/session facts.

`finite-auth` does not own:

- NIP-01 event serialization or signature verification.
- FROST ceremony cryptography.
- bifrost package codecs, relay messaging, or signer runtime readiness.
- Native/browser bridge implementations.
- Platform secret-store integrations.
- FiniteBrain product concepts such as Vault, Folder, sharing, or OKF policy.

## Audit Map

Start here:

- `CONTEXT.md`: glossary and domain language.
- `docs/adr/0001-split-auth-from-nostr-primitives.md`: boundary between
  `finite-auth` and `finite-nostr`.
- `docs/adr/0002-model-frostr-keysets-as-user-primary-signers.md`: original
  Frostr keyset placement decision.
- `docs/adr/0003-share-user-agent-signer-through-frostr.md`: current shared
  user-agent signer decision.
- `docs/specs/nip01-nip05-auth-scaffold.md`: auth protocol scaffold.
- `docs/specs/native-secure-secret-storage.md`: native cold-share storage
  standard.
- `docs/feature-dev/2026-07-01-shared-frostr-signer-prd.md`: current PRD.
- `docs/feature-dev/2026-07-01-shared-frostr-signer-issues.md`: pending
  implementation slices.

The highest-priority follow-up is replacing the old delegated agent key binding
scaffold with Agent Signing Session domain/store records.

## Development

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```
