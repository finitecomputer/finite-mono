# Review Packet

## Issue

- Issue: #14 — Brain invitations in Settings
- Slice type: AFK
- Acceptance criteria: dedicated Invitations section with create/inspect/accept/revoke/email-invite controls; pending invitation/shared-Folder status; existing validation and fail-closed behavior; invite hash deep link
- Baseline: `18d0b10`
- Current diff: `18d0b10...9a80c17`

## Implementation Summary

Settings now has a dedicated Invitations tab. Existing invitation forms and
status lists are moved into its mount before binding, preserving the established
event handlers, state, and request/crypto behavior.

## Implementation Evidence

- Worker session: `/root/ticket_14_invitations_settings`
- `tdd` used: yes, deterministic Product Client markup/handler assertions
- Commands run: `node --check`; deterministic Product Client test; `cargo test -p finite-brain-server`; `cargo fmt --check`; `git diff --check`

## Review Instructions

Review the rendered Settings tab and invitation hash route. Final stale-sidebar
cleanup and browser captures are owned by #15.

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- No hard repository, glossary, ADR, security, or secret-handling findings.

SPEC_STATUS: pass with staged follow-up
SPEC_FINDINGS:
- Invitation workflows remain on the existing fail-closed request seams; runtime source-anchor cleanup and final responsive verification are #15-owned.
```
