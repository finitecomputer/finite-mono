# Issue #128 Review Packet

## Issue

- Issue: #128
- Slice type: AFK tracer bullet
- Acceptance criteria: report both explicit and effective Folder Access through
  the public CLI while preserving the existing contract and authorization rules
- Baseline: `fc6dad8`
- Current diff: `fc6dad8..b8dcb7ac`

## Implementation Summary

`fbrain access list` now distinguishes identities explicitly listed on a Folder
from identities that can actually access it. Effective access is computed by the
same recipient policy already used to distribute Folder Keys.

## Implementation Evidence

- `implement` session: `/root/ticket_128_effective_access`
- `tdd` used: yes
- Red test, if applicable: Organization and Personal Brain public CLI cases
- Green implementation, if applicable: additive JSON fields and labeled text
  fields backed by the existing recipient policy
- Refactor, if applicable: shared Folder row rendering retained compatibility
- Commands run: focused public CLI tests (2 passed), full
  `finite-brain-cli` suite (95 passed), fmt, clippy with warnings denied, and
  diff check

## Review Instructions

Review only this issue's slice unless you find a severe cross-slice regression.
Keep standards and spec findings separate.

Check:

- Acceptance criteria are met.
- Tests verify behavior through public interfaces.
- No implementation-only tests are masquerading as behavior tests.
- No obvious incomplete work, TODO placeholders, or unrelated changes.
- Relevant test, typecheck, build, or visual verification commands pass.

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- None.

SPEC_STATUS: pass
SPEC_FINDINGS:
- None.
```
