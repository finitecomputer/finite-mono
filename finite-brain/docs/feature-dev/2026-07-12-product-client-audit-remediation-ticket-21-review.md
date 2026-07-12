## Issue

- Issue: #21
- Slice type: invitation controls, Session Lock, and admin revoke resolution
- Acceptance criteria: locked actions fail closed; eligible actions update on
  input; direct list revoke is guarded; revocation does not inspect recipient
  links; client-only secret material stays ephemeral
- Baseline: `fe577fc`
- Current diff: `fe577fc...4909693`

## Implementation Summary

The invitation panel now derives one explicit control state from signer,
Session, input, and Vault state. Protected actions guard before epoch capture.
The top-level administrator revoke resolves only known admin-side IDs, avoiding
the recipient-bound inspection route.

## Implementation Evidence

- `implement` session: `/root/ticket_21_invitations`
- `tdd` used: yes
- Red/green coverage: locked/unlocked controls, handler guards, input reactivity
  and invalidation, manual-secret non-retention, direct pending-row revoke, and
  known-admin revoke resolution
- Commands run: Node Product Client contract test, JavaScript syntax check, and
  diff hygiene

## Reviewer Output

```text
INDEPENDENT_FOCUSED_REVIEW_STATUS: pass
FINDINGS:
- No P0–P3 findings.
- Guards precede epoch capture for create, inspect, email scope, accept, panel
  revoke, and direct pending-list revoke.
- The revoke resolver uses explicit, remembered, or loaded administrator-side
  IDs and never calls the recipient-only link lookup.

FINAL_INTEGRATION_CORRECTION:
- A real recipient acceptance flow found that the state reset correctly locked
  the Session but did not immediately re-render the new lock/notice. Normal
  and email invitation acceptance now render after setting that notice; a
  deterministic regression guard and the disposable browser flow pass.
```
