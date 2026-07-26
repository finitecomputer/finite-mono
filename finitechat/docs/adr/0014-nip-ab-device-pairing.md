# ADR 0014: NIP-AB Is the Account Pairing Protocol

Date: 2026-07-25

Status: Accepted

## Context

Finite must transfer an account secret to a new native Device without exposing
that secret to WorkOS, the dashboard renderer, the rendezvous server, Swift, or
Electron JavaScript. The pre-release iOS and Electron implementations used a
Finite-specific encrypted payload protocol. There are no external users whose
old sessions need migration.

Buzz's draft NIP-AB supplies a small, signed Nostr pairing transcript with
ephemeral source and target keys, NIP-44 encryption, explicit confirmation, and
key derivation vectors. The draft is useful protocol input, not an adopted
standard or a substitute for our own review:
<https://github.com/block/buzz/blob/ab3af828714ab699dfc87644d234014987a4fe6b/crates/buzz-core/src/pairing/NIP-AB.md>

## Decision

- `finitechat-core` owns the only NIP-AB state machines and derivation code.
  iOS and Electron use that Rust implementation.
- Pairing uses signed Nostr kind `24134` events. The server accepts only
  canonical events with one exact `p` recipient tag, a valid signature, a
  bounded timestamp, and bounded content.
- Before WorkOS approval begins, the target generates its ephemeral key and
  creates a rendezvous session that immutably binds the pairing-session ID,
  Device ID, and target public key.
- WorkOS authenticates the human and acts as the out-of-band trust and consent
  channel. The hosted source returns its NIP-AB descriptor only to the native
  supervisor boundary. It never enters the dashboard renderer.
- The custom encrypted payload binds its schema version, purpose,
  pairing-session ID, account secret and derived account ID, target Device,
  public server URL, issue time, expiry, enrollment account, and a random
  enrollment resume capability. Outside the sealed encrypted payload
  checkpoint, the hosted source stores only the capability's SHA-256 digest.
- The hosted source seals its state-machine checkpoint and exact randomized
  outbound events before publishing. A retry or process restart republishes the
  exact same bytes and event IDs.
- The rendezvous server durably appends the offer, source confirmation,
  encrypted payload, and target completion. Exact retries are idempotent;
  alternate bytes, keys, order, recipients, or closed-state mutations fail.
- A target does not publish completion until its platform credential store has
  atomically stored and read back both the account secret and enrollment
  capability.
- NIP-AB is a bounded 120-second credential grant. Existing MLS room fanout and
  complete-history transfer begin only after the target acknowledges durable
  credential storage, but they are a separate durable enrollment operation.
  Once the grant is acknowledged, either client can resume enrollment using
  the encrypted capability without another WorkOS session and without
  extending or replaying the NIP-AB grant.
- Enrollment is advanced synchronously by authenticated/capability polling
  against one durable pending record. There is no detached in-memory history
  worker, process-local retry map, or second background finisher.
- The durable MLS link-fanout record is the sole source history job. Each Room
  freezes its accepted membership sequence, audits and plans one page per tick,
  and emits deterministic chunks. There is no target bootstrap request/replay
  protocol.
- The target keeps chunks invisible in encrypted SQLite. Its commit atomically
  imports exact history, Room metadata, profiles, and an immutable manifest
  receipt; conflicting duplicates durably poison the transfer.
- `ready` is an exact manifest receipt list, not a Room-count heuristic. The
  source returns the frozen manifests and the target independently proves the
  matching durable receipts plus its canonical paired agent. Intermediate
  source success is never presented as completed pairing.
- The resume capability lives for seven days independently of the 120-second
  NIP-AB transcript. `ready` consumes its mutation authority but retains an
  exact replayable tombstone until expiry. Expired valid records are removed,
  and each new approval opportunistically garbage-collects at most 32 expired
  records.

## Platform Boundaries

On iOS, Rust owns the ephemeral keys and transcript. Swift receives only the
decrypted account secret and opaque enrollment grant long enough to store and
read them back together from Keychain. Relaunch resumes pending enrollment
through Rust and clears the capability only after strict target readiness.

On Electron, the authenticated source descriptor travels from the main process
to the Rust child through a bounded dedicated file descriptor. The account
secret travels from Rust on a separate private descriptor. It is written
provisionally, promoted into Electron safe storage, read back, and only then
confirmed to Rust. Neither value is renderer IPC, argv, stdout, stderr, or log
data. The main process stores the enrollment grant before promoting the
credential, so every active credential has a crash-resumable enrollment path.

## Hard Cut

The old `/link-sessions` endpoints, payload/claim/ack records, crypto helper,
CLI commands, compatibility decoding, bootstrap request event, mutable replay
ledger, and pre-release client recovery flow are removed. A stale pre-release
pending-session marker may be discarded, but active hosted credentials and
user chat data are never deleted as part of that cleanup.

## Deferred

- QR scanning and public-relay rendezvous.
- A visible short-authentication-string comparison UI.
- Pairing without WorkOS.
- Claiming that the draft has received independent cryptographic audit.

Those features must reuse the same core state machines and exact payload
binding. They may replace the out-of-band descriptor channel, not fork the
credential-transfer protocol.

## Verification

Tests must cover the published derivation vector, attacker target keys, signed
events bound to the wrong transcript, route/account/expiry substitution,
explicit confirmation, restart with exact event IDs, server idempotency and
durability, and credential-store ordering. A required integration scenario
kills the target after credential storage but before NIP-AB completion,
restarts after the 120-second grant expires, rejects an incorrect resume
capability, and completes enrollment without WorkOS. Shipping clients
additionally require an iOS Simulator build/test/launch, Electron supervisor
tests, and complete-history stress above one transfer chunk/window.
