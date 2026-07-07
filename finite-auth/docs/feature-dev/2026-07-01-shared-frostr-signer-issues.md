# Shared Frostr Signer Local Issue Breakdown

Parent PRD: `2026-07-01-shared-frostr-signer-prd.md`.

No remote issue tracker is configured, so these are local tracer-bullet issues.
They are ordered for implementation after this mapping slice.

## Local-SFS-1: Hard-Cut Shared Signer Domain Vocabulary

Type: AFK.

Blocked by: None.

Status: Complete for active domain docs; historical code scaffold migration is
tracked by Local-SFS-2.

### What To Build

Replace the remaining target-state language that treats default agents as
separate delegated Nostr identities. The docs should consistently say agents
request signatures through the user's primary key and are distinguished by
Finite signing-session and audit data.

### Acceptance Criteria

- [x] Durable context defines Shared User-Agent Signer and Agent Signing
  Session.
- [x] Agent-facing domain notes describe session/audit attribution instead of
  delegated default keys.
- [x] ADR 0003 is the accepted model for agent identity.
- [x] Historical PRD/issue/ledger artifacts that mention delegated keys are
  clearly marked as superseded or implementation debt.

## Local-SFS-2: Replace Agent Key Binding With Signing Session Model

Type: AFK.

Blocked by: Local-SFS-1.

### What To Build

Replace the delegated agent key binding domain/store scaffold with a signing
session model. A signing session belongs to one user primary key, records the
agent runtime requesting signatures, and stores enough scope and audit metadata
to explain why a signature request was authorized.

### Acceptance Criteria

- [ ] Core domain types model an agent signing session without a separate
  default agent Nostr public key.
- [ ] Store schema persists signing-session records and can query them by user
  primary key and agent runtime identifier.
- [ ] Tests cover session creation, restart-safe persistence, and rejection of
  malformed scope or expiry metadata.
- [ ] Obsolete agent-key APIs are removed or explicitly quarantined behind a
  migration boundary.

## Local-SFS-3: Model Frostr Quorum Paths For User And Agent Signing

Type: AFK.

Blocked by: Local-SFS-2.

### What To Build

Represent the allowed two-share signing paths for the fixed server,
user-client, and native secure-storage placement model. The model should make
routine signing use the server share plus active user-client share, while the
native secure-storage share is reserved as a cold backup for recovery or
rotation.

### Acceptance Criteria

- [ ] Policy names the allowed two-share quorum paths.
- [ ] Server-only, client-only, and native-only signing are impossible states.
- [ ] Agent signing requires an active signing session and an allowed quorum
  path.
- [ ] Normal user or agent signing cannot consume the cold backup share.
- [ ] Tests cover accepted and rejected quorum-path representations.

## Local-SFS-4: Define Bifrost And Native Storage Adapter Interfaces

Type: AFK.

Blocked by: Local-SFS-3.

### What To Build

Define narrow interfaces for future bifrost-rs runtime calls and native secure
storage access. The interfaces should keep finite-auth responsible for policy
and metadata while letting host adapters own ceremony transport, package
format, signer readiness, platform secret handling, and the Rust `keyring`
backend choice.

### Acceptance Criteria

- [ ] A bifrost adapter boundary can prepare or request a threshold signature
  without exposing raw share material to finite-auth policy code.
- [ ] A native secure-storage boundary can report package references and unlock
  capability without choosing a specific platform implementation.
- [ ] Rust host guidance documents when to use `keyring` directly and when to
  use `keyring-core` with explicit store crates.
- [ ] Fake adapters can drive policy tests without live relays or real platform
  storage.
- [ ] Documentation states that live ceremony/runtime integration is still out
  of scope for this slice.

## Local-SFS-5: Product Gate For Unattended Agent Signing

Type: HITL.

Blocked by: Local-SFS-4 for any background-agent implementation.

### What To Decide

Decide whether FiniteBrain ever allows an agent to sign while the user is not
present. The native secure-storage share is not the mechanism for this; it is
the user's Cold Backup Share. The recommended default is no unattended signing
until a separate product/security review accepts a different mechanism.

### Acceptance Criteria

- [ ] A product/security decision records whether unattended signing is allowed.
- [ ] If allowed, the decision names required scopes, expiry, revocation,
  notification, audit behavior, and the non-backup signing mechanism.
- [ ] If not allowed, policy tests reject background signing paths.
- [ ] In either case, normal or unattended agent signing cannot consume the Cold
  Backup Share.

## Parked HITL Slices

Local-SFS-5 is parked. It does not block the domain/session/quorum scaffolding,
but it blocks any production behavior that would let agents sign unattended.
