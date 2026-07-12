## Issue

- Issue: #19
- Slice type: safe Product Client failure feedback
- Acceptance criteria: visible accessible feedback uses safe copy; Session Lock
  clears it; existing access errors are not duplicated; stale failures cannot
  recreate it after a lock
- Baseline: `7fc85c4`
- Current diff: `7fc85c4...cae93df`

## Implementation Summary

The Product Client now renders safe generic failures in one polite status
region, while existing inline access results retain their specific safe detail.
The session-owned state reset clears the region, and handled access errors are
marked before stale requests return.

## Implementation Evidence

- `implement` session: `/root/ticket_19_client_feedback`
- `tdd` used: yes
- Red/green coverage: deterministic closure seams prove access result
  suppression and post-lock stale failure suppression
- Commands run: Node Product Client contract test, JavaScript syntax check, and
  diff hygiene

## Reviewer Output

```text
STANDARDS_STATUS: pass after fixes
STANDARDS_FINDINGS:
- Compact breakpoint grid-row contract was corrected.
- Runtime regression now covers access-result/global-feedback suppression.

SPEC_STATUS: pass after fixes
SPEC_FINDINGS:
- Stale access failures are marked before the Session epoch guard, so a late
  rethrow cannot recreate feedback after Session Lock.
- The final isolated browser flow passed a generic visible failure followed by
  Session Lock purge of that feedback and session-owned plaintext.
```
