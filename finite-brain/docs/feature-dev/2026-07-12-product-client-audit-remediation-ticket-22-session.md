## Issue

- Issue: #22 — create true Child Folders from the context menu
- Fixed point before session: `ca86dbe`
- Worker session: `/root/ticket_22_child_folders`
- Commit: `f4d66f8`
- Status: complete; final shared browser verification passed

## Inputs

- Spec issue: #17
- Ticket: #22
- Relevant glossary terms: Brain, Folder, Child Folder, Folder Key Grant,
  Session Folder Key, independent access boundary
- Relevant ADRs: 0004, 0007, 0010, 0013, 0014, 0016
- Product truth: a Child Folder's decorated path and immediate parent are
  hierarchy metadata; access and cryptographic scope remain independent

## Implementation

- Public interface used: `New Folder Inside` context-menu action and root
  toolbar action
- Behaviors covered: context menu passes its selected Folder identifier;
  creation resolves the live parent metadata and rejects a stale parent;
  root creation remains root; child creation sends `parentFolderId` and full
  `parent.path/name` path; deeper hierarchy composes recursively
- Scope preserved: every new Folder still gets a fresh key/version-one grants,
  normal personal/organization defaults, empty explicit recipients, and does
  not copy a restricted parent's recipients, key, grants, or access mode
- `tdd` used: yes; deterministic helper and source-contract regressions cover
  root, restricted-parent child, deep child, stale parent, context wiring, and
  non-inherited defaults
- Commands run during implementation:
  - `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
  - `git diff --check`

## Review

- Review fixed point: `ca86dbe`
- Standards review: pass; no actionable findings
- Spec review: pass; hierarchy and independent scope match the portability
  contract and the remediation spec
- Final browser proof: the isolated organization-Brain flow created a Child
  Folder through the context menu and confirmed the parent id, nested path,
  independent grants, and `all_members` default access in the real POST.

## Risks

- The backend validates parent existence but does not derive the decorated path
  independently. That is not needed for this client repair and remains a
  separate server-hardening opportunity.
