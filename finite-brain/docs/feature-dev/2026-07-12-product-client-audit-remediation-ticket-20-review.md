## Issue

- Issue: #20
- Slice type: Session Lock after confirmed active-Vault authorization loss
- Acceptance criteria: active Vault membership loss clears session-only
  material and locks; unrelated failures do not; safe notice remains visible
- Baseline: `65b98a7`
- Current diff: `65b98a7...390c801`

## Implementation Summary

`protectedRequest` preserves response status, reason, and path. A narrow
predicate recognizes only the server's exact active-Vault membership-loss
contract on metadata, export, and sync bootstrap, then clears the session and
leaves safe guidance. It also suppresses the generic failure strip because the
Session Lock notice is the relevant user-visible result.

## Implementation Evidence

- `implement` session: `/root/ticket_20_access_loss`
- `tdd` used: yes
- Red/green coverage: exact positive paths, generic auth failures,
  administrator and Folder boundaries, stale epochs, state purge, retained
  notice, and feedback suppression
- Commands run: Node Product Client contract test, JavaScript syntax check, and
  diff hygiene

## Reviewer Output

```text
FOCUSED_SELF_REVIEW_STATUS: pass
FINDINGS:
- None. The membership-loss predicate is constrained to status 403, reason
  "vault access required", matching active Vault, and one of the three
  vault-wide state-read paths.
- Browser member-removal proof remains in the final isolated-Vault pass.
```
