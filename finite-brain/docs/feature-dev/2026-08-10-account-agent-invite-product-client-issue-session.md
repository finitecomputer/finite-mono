# Account-Agent Invite Product Client Issue Session

## Issue

- Issue: frontend tracer across #442, #443, #446, #449, and #450
- Fixed point before session: `0903b4267efbd53c8eafad65ea42958f860ebc0c`
- Worker session: current delegated Codex task
- Commit: pending
- Status: implemented, verified, and reviewed; publication in progress

## Inputs

- Spec issue: #441
- Ticket: #442 read-only preflight, #443 immutable plan commit, #446 explicit
  reduced-set approval, and #449/#450 atomic Folder cohort access/removal
- Relevant glossary terms: Finite VIP Mailbox Address, Account Access Cohort,
  Account Agent Set, Invitation Preflight, Invitation Participant Set, Folder
  Access Readiness, Member Identity, Product Client
- Relevant ADRs: ADR 0045, plus preserved client-owned key and server-blindness
  rules referenced by it
- Prototype answer and source branch, if any: none

## Implementation

- Public interface used: deterministic Product Client contract suite over the
  real browser client closure and protected HTTP request seam; Rust-served
  `/client` for visual and interaction verification
- Behaviors covered: success, visible friendly roster, not-ready/excluded
  agents, capacity failure, stale/expired plan, backend-not-yet-upgraded refusal,
  mutation-free preview, exact exclusion approval, explicit commit, additive
  Personal Brain Agent Set rendering/grant fanout, and explicit acting-agent /
  authorizing-human authority presentation. Both visible Folder grant entry
  points converge on cohort preflight for Finite VIP mailboxes; removal previews
  friendly identities, preserves independently retained access, and rotates once
  through the atomic cohort route. A human row removes the mailbox cohort; an
  account-agent row uses the existing targeted-principal Folder DELETE, records
  a durable Folder exclusion, and preserves the human and siblings. Routing is
  driven by `accountAccessCohorts` provenance rather than interpreting a Managed
  Agent NIP-05 as a deliverable mailbox. Ready Personal Brain agents cannot use
  this browser client to restrict a ready sibling: the control is withheld and
  the request seam fails closed until Chat/CLI supplies authenticated human
  intent; owner-human removals remain unchanged.
- `tdd` used: yes; one failing user-observable client seam per vertical slice
- Commands run during implementation: Product Client contract suite; JS syntax
  checks; static verifier shell; Rust formatting; full `finite-brain-server`
  tests; focused served-client asset test; server clippy with warnings denied;
  `finite-brain-app` build; desktop/mobile browser verification
- Full suite command: `scripts/with-dev-env cargo test -p finite-brain-server
  --locked` (81 passed) plus the deterministic Product Client suite

## Review

- Review fixed point: `0903b4267efbd53c8eafad65ea42958f860ebc0c`
- Standards findings: none remaining
- Spec findings: none remaining
- Worthy fixes applied: authoritative empty-roster handling, converged Finite
  VIP Folder entry points, bounded plan fanout, relational removal-plan
  validation, targeted agent removal, canceled-preview routing isolation, and
  malformed/duplicate exclusion rejection. Authenticated Human Intent coverage
  proves the peer-agent UI guard, zero-request mutation guard, and unchanged
  owner-human path at the real client seam.
- Findings ignored with reasons: CodeRabbit's suggestion that exclusions carry
  `npub` and `relationship` was not applied because the authoritative backend
  exclusion contract is `{name,nip05,reason}`; its valid validation concern was
  addressed against that exact shape

## Risks

- This frontend PR depends on the concurrent account-agent cohort backend change
  landing first or with it. Older backends remain readable but the new mailbox
  mutation controls fail closed rather than writing legacy permissions.
