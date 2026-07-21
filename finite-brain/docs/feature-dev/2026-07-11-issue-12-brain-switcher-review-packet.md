# Review Packet

## Issue

- Issue: #12 — Brain switcher and Manage Brains modal
- Slice type: AFK
- Acceptance criteria: Obsidian-like Brain switcher from the bottom row; separate Manage Brains modal; explicit Load/Resume and signer/create-org actions; no changes to encrypted lifecycle semantics; responsive no-overflow behavior
- Baseline: `644e981`
- Current diff: `644e981...4442081`

## Implementation Summary

The account footer now opens a compact Brain menu populated from the existing
visible Brain state. A dedicated Manage Brains dialog exposes the same selection
and management operations without reintroducing the dense sidebar controls.

## Implementation Evidence

- Worker session: `/root/ticket_12_brain_switcher` (shared worktree recovery)
- `tdd` used: yes, deterministic Product Client markup/handler contract assertions
- Commands run: `node --check`; deterministic Product Client test; full Rust test/fmt/diff checks pending final integration

## Review Instructions

Review only the Brain navigation slice. Access/sharing and invitation relocation
are explicitly covered by #13/#14.

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- No hard repository, glossary, ADR, security, or secret-handling findings.

SPEC_STATUS: pass with staged follow-up
SPEC_FINDINGS:
- The new menu/modal preserve the existing Brain/session operation seams; the dense Access sidebar remains until the dependent migration tickets.
```
