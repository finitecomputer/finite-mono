# ADR 0013: iOS Is a Single-Agent Client

Date: 2026-07-23

Status: Accepted

## Context

The first iOS implementation mirrored a general chat application: rooms,
people, agents, profiles, scanning, group creation, and several independent
navigation surfaces. That breadth kept the native client behind the working
web and Electron products.

The product we need to ship is narrower. One human links their existing Finite
account, chooses one agent, and talks to that agent across the same Topic and
Chat history projected by the Rust runtime.

There is no deployed iOS user data to migrate. Compatibility with the
pre-release iOS store or Keychain shape is explicitly not a requirement.

## Decision

- iOS targets iOS 26 and uses native SwiftUI controls and Liquid Glass.
- WorkOS authenticates the human through native-app PKCE. iOS then calls the
  existing authenticated dashboard device-link APIs automatically, matching
  Electron's current sequence without adding an approval page. A one-use Rust
  receiver transfers the encrypted account secret to iOS, which stores it in a
  new Keychain namespace before acknowledging delivery.
- The browser never receives account or pairing secrets. Swift never implements
  device-link cryptography, and secret material never enters `AppState`,
  `UserDefaults`, diagnostics, URLs, or logs.
- The user pairs exactly one connected direct agent Room. Rust persists both
  the agent account identity and canonical Room identity; selection and sort
  order are never authority.
- Home is the root. Submitting its composer dispatches one idempotent
  `StartHomeChat` intent. Rust chooses the paired Room and `home` Topic, creates
  or reuses the intent-specific Chat, persists the first message in the normal
  outbox, and returns the exact selected route.
- Home presents the three most recently updated Chats. Chat navigation presents
  an overlay drawer grouped by the existing Topics.
- The existing transcript, composer, RMP update stream, encryption, replies,
  reactions, attachments, polls, and voice UI remain.
- The tab shell, room directory, people browser, agent directory, QR scanner,
  manual Nostr create/import login, profile editor, group creation, membership
  editing, and room administration are outside the iOS product.
- Settings contains only paired-agent switching, linked-account identity, and
  destructive local sign-out.

## Hard-Cut Rules

- Missing new paired-agent metadata fails closed; it is not compatibility
  decoded or inferred.
- The linked-account Keychain service is new, so pre-release manual identities
  are not silently adopted.
- No migration screen, legacy importer, old navigation fallback, or hidden
  compatibility path is added.

## Quality Contract

- SwiftUI screens are composed from small value-driven components with semantic
  design tokens.
- Home, first-run Home, account link, agent picker, and chat drawer have
  deterministic network-free Xcode previews.
- Rust tests prove direct-agent validation, hard-cut metadata decoding, pairing
  persistence, and idempotent Home submission.
- Dashboard tests prove exact bounded device-link coordinates, authenticated
  approval, and status polling.
- Shipping requires an iOS Simulator build/test, a real app launch, and visual
  inspection through the Codex simulator browser.

## Non-Goals

- Full key self-custody.
- Multiple simultaneously paired agents.
- Apple sign-in configuration in WorkOS.
- Creating Topics, agents, rooms, or memberships from iOS.
- Reworking the retained chat transcript UI.
