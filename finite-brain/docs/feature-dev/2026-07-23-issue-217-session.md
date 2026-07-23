# Issue #217 Session: Convergent Organization Brain collaboration

## Issue

- Issue: `finitecomputer/finite-mono#217`
- Fixed point before session: `894d1ba`
- Worker session: `/root/ticket_217_worker`
- Commit: final feature commit reported in the handoff
- Status: review fixes implemented; focused suites and built-process acceptance verified locally

## Inputs

- Spec issue: #216
- Ticket: #217
- Relevant glossary terms: Organization Brain Collaboration, Brain Role,
  Folder Access Readiness, Folder Key Grant, Member Identity
- Relevant ADRs/specs: `docs/adr/0034-make-organization-brain-collaboration-a-desired-state-operation.md`,
  `docs/feature-dev/2026-07-23-organization-brain-collaboration-spec.md`
- Prototype answer and source branch: none

## Implementation

- Public interface: signed `/_admin/brains/{brain-id}/collaborators/ensure-admin`
  and
  `fbrain collaborators ensure-admin --brain ... --target ...`.
- Behaviors covered:
  - Native target resolution is performed before the batch and canonical npub
    is reused throughout the signed request.
  - The client snapshots every existing Folder, opens only local session keys,
    and sends opaque NIP-59 recipient grants for available keys.
  - One SQLite transaction converges membership, admin role, available grants,
    and control records; retries are no-ops for current relationships/grants.
  - Receipts expose complete/partial state and safe per-Folder outcomes without
    Folder Keys, grant plaintext, or signer secrets.
- `tdd` used: yes; public CLI and signed server seams were retained. Review
  fixes add typed transport/HTTP rejection handling, Folder-scoped audit
  evidence, bounded request fanout, and authoritative postcondition receipts.

## Verification

- `cargo fmt --all -- --check`
- `cargo check -p finite-brain-server`
- `cargo check -p finite-brain-cli`
- `cargo test -p finite-brain-store --lib --no-fail-fast` (59 passed)
- `cargo test -p finite-brain-server --lib --no-fail-fast` (67 passed)
- `cargo test -p finite-brain-cli --lib --no-fail-fast` (130 passed, 2 ignored)
- `cargo test -p finite-brain-cli --test fbrain_process_acceptance built_fbrain_process_two_independent_homes_open_restricted_collaboration --no-fail-fast` (passed)
- `cargo test -p finite-brain-server --lib --no-fail-fast` (66 passed)

## Risks

- Built-process acceptance now runs two independent Finite Homes against a real
  signed Brain server, grants an existing restricted Folder, and proves the
  recipient materializes and reads the restricted Page.
- A lost response after mutation is represented as `indeterminate` only for
  transport failure; authoritative HTTP status responses remain typed errors.
