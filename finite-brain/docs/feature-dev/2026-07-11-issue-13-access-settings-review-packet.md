# Review Packet

## Issue

- Issue: #13 — Access & sharing in Settings
- Slice type: AFK
- Acceptance criteria: Access & sharing in Settings; existing Folder/Brain/share workflows preserved; Access ribbon deep link; fail-closed signer/session behavior; modal-scoped results/busy state
- Baseline: `93ddfcb`
- Current diff: `93ddfcb...18d0b10` plus the refresh follow-up in `93ddfcb`

## Implementation Summary

The Access ribbon and context actions now open Settings directly to an Access &
sharing tab. The existing dense panel is moved into the tab before the client
binds its handlers, so all request and authorization behavior remains on the
same DOM nodes and state model.

## Implementation Evidence

- Worker session: `/root/ticket_13_access_settings`
- `tdd` used: yes, deterministic Product Client markup/handler assertions
- Commands run: `node --check`; deterministic Product Client test; `cargo test -p finite-brain-server`; `cargo fmt --check`; `git diff --check`

## Review Instructions

Review the rendered DOM and routing behavior. Invitations receive a dedicated
section in #14; final stale-sidebar cleanup and responsive captures are #15.

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- No hard repository, glossary, ADR, security, or secret-handling findings.

SPEC_STATUS: pass with staged follow-up
SPEC_FINDINGS:
- Access controls are Settings-owned at runtime; the historical source anchor and top Brain strip are explicitly deferred to final integration cleanup.
```
