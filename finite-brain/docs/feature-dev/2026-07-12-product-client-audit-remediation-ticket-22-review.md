## Issue

- Issue: #22
- Slice type: context-menu Child Folder hierarchy metadata
- Acceptance criteria: the context parent is preserved; path is decorated and
  recursive; stale parent is rejected; child keys/access stay independent
- Baseline: `ca86dbe`
- Current diff: `ca86dbe...f4d66f8`

## Implementation Summary

The Product Client resolves an optional parent from current Vault metadata and
uses a pure hierarchy helper for the Folder-create request. The existing key
generation, grants, role, and access defaults remain unchanged.

## Implementation Evidence

- `implement` session: `/root/ticket_22_child_folders`
- `tdd` used: yes
- Red/green coverage: root, child, deep child, stale parent, context parent,
  hierarchy request fields, and independent access defaults
- Commands run: Node Product Client contract test, JavaScript syntax check, and
  diff hygiene

## Reviewer Output

```text
STANDARDS_STATUS: pass
SPEC_STATUS: pass
FINDINGS:
- None. The implementation preserves the independent Folder key/access boundary
  while transmitting only hierarchy metadata for the context parent.
- The final isolated browser flow passed the context-menu Child Folder POST and
  confirmed parent metadata, nested path, independent grants, and default
  `all_members` access.
```
