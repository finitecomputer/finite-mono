# Issue Session

## Issue

- Issue: #12 — Brain switcher and Manage Brains modal
- Fixed point before session: `644e981`
- Worker session: `/root/ticket_12_brain_switcher` (recovered shared worktree after worker interruption)
- Commit: `4442081`
- Status: complete for the Brain navigation slice

## Inputs

- Spec issue: #10
- Ticket: #12
- Relevant glossary terms: Brain, Session Lock, Session Folder Key, Ephemeral Client Plaintext
- Relevant ADRs: 0010, 0013, 0014, 0016

## Implementation

- Public interface used: real Rust-served Product Client `/client`; existing Brain/session state and request functions
- Behaviors covered: bottom-row Brain switcher menu, visible Brain selection, Manage Brains dialog, explicit Load/Resume, signer connection, Organization Brain creation, focus return, Escape/backdrop close, and responsive layout
- Existing crypto/auth/sync semantics remain delegated to `setActiveBrainId`, `connectSigner`, `loadBrainReader`, `resumeSession`, and `createOrganizationBrainFromInput`
- Commands run during implementation: JS syntax check and deterministic Product Client test
- Full suite command: pending final integration ticket

## Review

- Review fixed point: `644e981`
- Standards findings: no hard repository, glossary, ADR, security, or secret-handling findings
- Spec findings: the switcher and Manage Brains controls now replace the interim Brain-settings trigger; Access/Invitations migration remains owned by #13/#14
- Worthy fixes applied: none beyond the committed slice

## Risks

- The dense Access sidebar remains temporarily until #13 moves it into Settings; final integration must verify no stale primary entry point remains.
