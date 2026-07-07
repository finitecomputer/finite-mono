# PRD: Smoke Alpha Hardening For Product Client And Agent Workflows

## Problem Statement

The Rust FiniteBrain stack is now on `main`, but the first internal smoke alpha
still needs the remaining product and deployment-readiness gaps closed before it
matches the desired state: smoke members can use the Product Client, every smoke
machine can use `fbrain`, and a User or their Agent can harden and maintain a
Vault Working Tree with a Nostr keypair.

The live smoke route still points at the old SilverBullet prototype, Product
Client Folder Key Grant handling is still partly development-shaped, `fbrain`
sync is command-driven rather than resident, organization Vault invitation
controls are not productized in the browser, and backup/restore/cutover
expectations need an operator runbook.

## Solution

Ship the remaining smoke-alpha hardening in one feature branch:

- Replace the old SilverBullet route as an explicit deployment handoff and
  runbook, with the Rust app as the target service.
- Harden Product Client Folder Key Grant wrapping/opening around NIP-07 NIP-44
  encryption/decryption rather than plaintext development grant parsing.
- Add a real foreground daemon watch loop for `fbrain` so an Agent Runtime can
  keep a Vault Working Tree synced without manually invoking `sync now` after
  every edit.
- Add Product Client organization Vault invitation controls for creating,
  accepting, and revoking organization member invitations.
- Add a SQLite backup/restore and smoke cutover runbook plus an executable local
  verifier where practical.

## User Stories

1. As a smoke member, I want the smoke FiniteBrain URL to serve the Rust Product Client, so that I am not using the old SilverBullet prototype.
2. As a smoke operator, I want a concrete cutover and rollback checklist, so that replacing the old route is deliberate and recoverable.
3. As a smoke operator, I want to know exactly which process, port, database path, and route should be live, so that the smoke box is not split across old and new services.
4. As a Product Client User, I want Folder Key Grants opened through my NIP-07 signer, so that plaintext grants are not treated as the production trust path.
5. As a Product Client User, I want bad or wrong-recipient grant envelopes rejected clearly, so that I can trust what keys are opened locally.
6. As a Product Client User, I want grant creation for access/share flows to use browser NIP-44 encryption when available, so that grants created in the browser are hardened for smoke.
7. As an Agent, I want `fbrain daemon watch` to keep syncing my Vault Working Tree, so that normal file edits are encrypted, signed, pushed, and refreshed without manual polling.
8. As an Agent, I want daemon watch failures recorded in local state, so that blocked sync conditions are visible through `status`, `daemon logs`, and `conflicts`.
9. As an Agent, I want bounded daemon options, so that tests and smoke commands can run the watcher safely.
10. As an organization Vault admin, I want browser controls to invite a smoke member by npub, so that the internal org Vault can be bootstrapped without dropping to the Smoke UI.
11. As an invited smoke member, I want browser controls to inspect and accept an organization Vault invitation, so that joining the org Vault is possible from the Product Client.
12. As an organization Vault admin, I want browser controls to revoke pending invitations, so that stale invites can be cleaned up.
13. As a smoke operator, I want a SQLite backup command that uses a database-consistent snapshot, so that smoke state can be preserved before and after cutover.
14. As a smoke operator, I want a restore verification checklist, so that a restored server proves health, metadata, grants, sync, invitations, and readable client behavior.
15. As a future deployment agent, I want the deployment handoff to distinguish Feature Dev work from live smoke promotion, so that live route changes happen in the right loop.

## Implementation Decisions

- The feature branch remains based on `main` because the repo has already
  hard-cut `main` to the Rust Product Client plus `fbrain` stack.
- Live smoke route changes are recorded as a deployment handoff in this Feature
  Dev run. The codebase must contain enough runbook and verification detail for
  a Deployment loop to replace the old SilverBullet service with the Rust app.
- Product Client grant opening will prefer NIP-07 `nip44.decrypt` and validate
  the gift-wrap shell, seal event, rumor event, recipient, and Folder Key Grant
  plaintext before opening a key.
- Product Client grant creation will prefer NIP-07 `nip44.encrypt` and will keep
  development plaintext grant handling as an explicit test/dev fallback only.
- `fbrain daemon watch` will be a foreground resident process suitable for tmux,
  systemd, or agent supervisor use. It will run the same sync engine used by
  `sync now` and `daemon tick`, with bounded options for tests and smoke.
- Product Client organization invitation controls will call the existing
  protected invitation routes and reuse the existing signed auth boundary.
- Backup/restore will use the existing SQLite authoritative-store decision and
  document database-consistent backup, restore, integrity, and route-cutover
  checks.

## Testing Decisions

- Product Client grant tests should use deterministic fake NIP-44
  encrypt/decrypt adapters to verify browser grant behavior through exported
  Product Client helper functions.
- Product Client organization invitation tests should verify request builders
  and route invocation behavior without depending on a real browser extension.
- CLI daemon tests should run through `run_with_env` and use a bounded watch
  mode such as `--once` or `--max-ticks` to avoid background-process flakiness.
- Runbook/verifier coverage should be executable locally where possible, using a
  temporary SQLite database and local Rust server instead of live smoke data.
- Full verification should include Rust workspace checks, Product Client JS
  tests, the existing Product Client smoke verifier, and a local fbrain
  create/open/watch/sync flow.

## Out of Scope

- Live smoke route mutation, production config changes, live data deletion, and
  customer-impacting switches are out of scope for Feature Dev and belong to a
  Deployment loop.
- Legacy SilverBullet compatibility is out of scope. This is a hard cut.
- Postgres, relay federation, and durable key backup are out of scope.
- A background daemon supervisor installer is out of scope; this PR provides the
  foreground resident watch loop and runbook hooks for a supervisor.

## Further Notes

- The current live smoke host evidence shows the old route serving a
  SilverBullet process on port `3025`, no Rust `finite-brain-app` process, and no
  `fbrain` binary on the smoke host PATH.
- The earlier fbrain transport and working-tree sync PRD is implemented on
  `main`, but those older issues may still need issue-tracker cleanup after this
  hardening branch lands.
