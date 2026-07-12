# Review Packet

## Issue

- Issue: #11 — Settings shell and Session controls
- Slice type: AFK
- Acceptance criteria: compact footer identity/settings row; accessible Session/Vault Settings modal; existing Session Lock/Resume/signer behavior; no regression to Files/Search; responsive no-overflow foundation
- Baseline: `9408651`
- Current diff: `9408651...644e981`

## Implementation Summary

The Product Client now has a compact bottom account/Vault row and a Settings
modal with Session and Vault sections. Session Lock, Resume, signer connection,
keyboard navigation, Escape, backdrop close, focus return, and responsive modal
styles reuse the existing client state and security lifecycle.

## Implementation Evidence

- `implement` session: `/root/ticket_11_settings_shell`
- `tdd` used: yes, at the existing deterministic Product Client contract seam
- Red test, if applicable: markup contract initially expected the switcher role
- Green implementation, if applicable: corrected dialog semantics and explicit interim Vault Settings action
- Refactor, if applicable: footer details interaction replaced by explicit controls
- Commands run: `node --check`; deterministic Product Client test; `cargo test -p finite-brain-server`; `cargo fmt --check`; `git diff --check`

## Review Instructions

Review only this issue's foundation slice unless a severe cross-slice regression appears. Access/Invitations migration is explicitly covered by #13/#14.

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- No hard repository, glossary, ADR, security, or secret-handling findings.

SPEC_STATUS: pass with staged follow-up
SPEC_FINDINGS:
- The final Access-sidebar migration is intentionally deferred to #13; the interim Vault footer action is functional and explicitly opens Vault Settings until #12 supplies the switcher.
```
