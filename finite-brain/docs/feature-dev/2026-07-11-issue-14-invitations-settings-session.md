# Issue Session

## Issue

- Issue: #14 — Brain invitations in Settings
- Fixed point before session: `18d0b10`
- Worker session: `/root/ticket_14_invitations_settings`
- Commit: `9a80c17`
- Status: complete for the Invitations relocation slice

## Inputs

- Spec issue: #10
- Ticket: #14
- Relevant glossary terms: Brain, Member Identity, Session Lock, Ephemeral Client Plaintext
- Relevant ADRs: 0010, 0013, 0014, 0016

## Implementation

- Public interface used: real Rust-served Product Client `/client`; existing invitation, email-proof, and shared-Folder request functions
- Behaviors covered: Settings → Invitations tab, email invite/create controls, inspect/accept/revoke actions, pending invitations, shared-Folder relationships, signer/session gating, and invite-hash navigation
- Existing one-shot validation and Invite Secret handling remain on the original client state/request seams; the DOM nodes are reparented before bind to avoid duplicate IDs
- Commands run during implementation: JS syntax check, deterministic Product Client test, `cargo test -p finite-brain-server`, `cargo fmt --check`, `git diff --check`

## Review

- Review fixed point: `9a80c17`
- Standards findings: no hard repository, glossary, ADR, security, or secret-handling findings
- Spec findings: invitation navigation now opens Settings → Invitations; stale sidebar cleanup and responsive captures remain #15
- Worthy fixes applied: none beyond the committed slice

## Risks

- The historical source anchors remain in the large Access markup and are reparented at startup. Final integration must verify rendered DOM placement and no invitation controls remain in Files/Search.
