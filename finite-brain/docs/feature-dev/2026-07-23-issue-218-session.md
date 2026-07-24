# Issue Session

## Issue

- Issue: https://github.com/finitecomputer/finite-mono/issues/218
- Fixed point before session: `cc9dfa4`
- Worker session: `/root/ticket_218_worker`
- Commit: pending
- Status: implementation complete; final independent review pending

## Inputs

- Spec issue: https://github.com/finitecomputer/finite-mono/issues/216
- Ticket: https://github.com/finitecomputer/finite-mono/issues/218
- Relevant glossary terms: Brain Role, Folder Access Readiness, Organization
  Brain Collaboration, Product Client, Session Folder Key
- Relevant ADRs:
  `finite-brain/docs/adr/0034-make-organization-brain-collaboration-a-desired-state-operation.md`
- Prototype answer and source branch, if any: none

## Implementation

- Public interface used: Organization Brain metadata plus
  `POST /_admin/brains/{brain_id}/collaborators/ensure-admin`
- Behaviors covered:
  - Organization admins receive an authoritative role/current-grant readiness
    projection while non-admin metadata omits collaborator grant relationships.
  - People rows show Brain Role independently from ready/total current Folder
    access and name every unready Folder.
  - Email-first administrator addition snapshots all current Folders, wraps
    every locally available current Session Folder Key, signs Folder-specific
    evidence, and submits the #217 desired-state command.
  - Partial rows offer a repair action that rebuilds and repeats the same
    desired-state intent.
  - Complete, partial, and indeterminate results use distinct, accessible,
    secret-free presentation.
- `tdd` used: yes; Product Client role/readiness and receipt assertions failed
  before implementation, then passed. The server projection test was extended
  through missing-grant, repaired, rotated-version, and non-admin cases.
- Commands run during implementation:
  - `node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - `scripts/with-dev-env cargo check -p finite-brain-server --locked`
  - `scripts/with-dev-env cargo test -p finite-brain-server --locked signed_organization_collaboration_is_complete_idempotent_and_partial_safe -- --nocapture --test-threads=1`
  - `scripts/with-dev-env cargo fmt --all --check`
- Full suite command:
  `scripts/with-dev-env cargo test -p finite-brain-server --locked`

## Review

- Review fixed point: `cc9dfa4`
- Standards findings: first pass requested a bounded summary projection,
  policy-entitled member scope, repair without an email alias, and a rendered
  repair regression test.
- Spec findings: first pass found Organization readiness leaking into the
  otherwise hidden Personal owner row.
- Worthy fixes applied: replaced collaborator-by-Folder response expansion
  with one capacity-bounded summary per collaborator; derived member totals
  from policy entitlement; allowed human-safe repair with the internally
  canonical identity; added rendered unresolved-admin and Personal regression
  coverage.
- Findings ignored with reasons: none

## Risks

- The readiness projection intentionally exposes per-member current-grant
  coverage only to Organization Brain admins. Product UI retains no raw key,
  grant plaintext, or public-key fallback.
