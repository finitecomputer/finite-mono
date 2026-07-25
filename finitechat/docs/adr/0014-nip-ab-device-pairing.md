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
  public server URL, issue time, and expiry.
- The hosted source seals its state-machine checkpoint and exact randomized
  outbound events before publishing. A retry or process restart republishes the
  exact same bytes and event IDs.
- The rendezvous server durably appends the offer, source confirmation,
  encrypted payload, and target completion. Exact retries are idempotent;
  alternate bytes, keys, order, recipients, or closed-state mutations fail.
- A target does not publish completion until its platform credential store has
  promoted and read back the account secret.
- Existing MLS room fanout and complete-history transfer begin once the source
  response is durably published. They do not depend on receiving the target's
  courtesy completion event.

## Platform Boundaries

On iOS, Rust owns the ephemeral keys and transcript. Swift receives only the
decrypted account secret long enough to store and read it back from Keychain.

On Electron, the authenticated source descriptor travels from the main process
to the Rust child through a bounded dedicated file descriptor. The account
secret travels from Rust on a separate private descriptor. It is written
provisionally, promoted into Electron safe storage, read back, and only then
confirmed to Rust. Neither value is renderer IPC, argv, stdout, stderr, or log
data.

## Hard Cut

The old `/link-sessions` endpoints, payload/claim/ack records, crypto helper,
CLI commands, and compatibility decoding are removed. A stale pre-release
pending-session marker may be discarded, but active credentials and user chat
data are never deleted as part of that cleanup.

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
durability, and credential-store ordering. Shipping clients additionally
require an iOS Simulator build/test/launch and Electron supervisor tests.
