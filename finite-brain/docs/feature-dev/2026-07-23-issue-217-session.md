# Issue #217 Session: Convergent Organization Brain collaboration

## Issue

- Issue: `finitecomputer/finite-mono#217`
- Fixed point before session: `894d1ba`
- Worker session: `/root/ticket_217_worker`
- Commit: final feature commit reported in the handoff
- Status: implemented, verified locally

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
  (with `collaboration/ensure-admin` compatibility alias) and
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
- `tdd` used: yes; existing public CLI and signed server seams were retained,
  with compilation and focused component suites as the tracer checks.

## Verification

- `cargo fmt --all -- --check`
- `cargo check -p finite-brain-server`
- `cargo check -p finite-brain-cli`
- `cargo test -p finite-brain-server --lib --no-fail-fast` (66 passed)
- `cargo test -p finite-brain-cli --lib --no-fail-fast` (128 passed, 2 ignored)

## Risks

- The full two-Finite-Home process acceptance remains a follow-up runtime gate;
  this slice is covered at the signed HTTP and public CLI seams.
- A lost response after mutation is not observable by the server route itself;
  the CLI transport layer still reports its existing HTTP error form rather
  than persisting an indeterminate retry receipt.
