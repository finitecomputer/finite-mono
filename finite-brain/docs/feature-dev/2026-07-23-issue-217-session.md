# Issue #217 Session: Convergent Organization Brain collaboration

## Issue

- Issue: `finitecomputer/finite-mono#217`
- Fixed point before session: `894d1ba`
- Worker sessions: `/root/ticket_217_worker`,
  `/root/ticket_217_fixes`, `/root/ticket_217_fixes_round3`, and
  `/root/ticket_217_fixes_round4`
- Commits: `ef48206`, `bb500b7`, `49c8b66`, plus the final round-4 correction
  commit reported in the handoff
- Status: four implementation/review passes completed; component suites,
  clippy, and built-process acceptance verified locally

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
  evidence, bounded request fanout, authoritative postcondition receipts,
  recipient-derived current-key-holder guidance, and a collaboration-specific
  request-body bound sized for the documented 1,000-grant maximum.

## Verification

- `cargo fmt --all -- --check`
- `cargo check -p finite-brain-server`
- `cargo check -p finite-brain-cli`
- `cargo clippy -p finite-brain-server -p finite-brain-cli --all-targets -- -D warnings`
- `cargo test -p finite-brain-store --lib --no-fail-fast` (59 passed)
- `cargo test -p finite-brain-server --lib --no-fail-fast` (72 passed)
- `cargo test -p finite-brain-cli --lib --no-fail-fast` (135 passed, 2 ignored)
- `cargo test -p finite-brain-cli --test fbrain_process_acceptance built_fbrain_process_two_independent_homes_open_restricted_collaboration --no-fail-fast` (passed)

## Risks

- Built-process acceptance now runs two independent Finite Homes against a real
  signed Brain server, grants an existing restricted Folder, and proves the
  recipient materializes and reads the restricted Page.
- A lost or malformed successful response after mutation is represented as
  `indeterminate`; authoritative HTTP status responses remain typed errors.
- Signed-route coverage proves existing-member and incomplete-admin repair,
  already-complete and mutation-free exact retry, stale/add/remove drift,
  malformed evidence and wrapped grants, authorization, grant-limit rejection,
  transactional rollback, secret-free receipts, distinct grant
  issuer/recipient guidance, and the largest accepted 1,000-folder/1,000-grant
  request.
- Human CLI output uses only recorded email identity for key holders and gives
  a safe unavailable message rather than falling back to raw `npub`; JSON
  retains public-key diagnostics.
