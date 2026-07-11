# Review Packet: Issue #7 Dashboard-Themed Access Workflows

## Issue

- Issue: [finitecomputer/finite-mono#7](https://github.com/finitecomputer/finite-mono/issues/7)
- Slice type: AFK tracer bullet
- Acceptance criteria: dashboard-aligned Vault and Folder access surfaces;
  coherent permission, people, invitation, email-bootstrap, share-link,
  shared-Folder, and cross-Vault workflows; distinct status and destructive
  actions; unchanged DOM hooks and authorization/request behavior;
  representative resumed desktop evidence in light and dark; checks green
- Baseline: `ac6564a`
- Current diff: `git diff ac6564a...HEAD`

## Implementation Summary

The access and collaboration UI now consumes one semantic presentation layer
mapped to the Product Client's dashboard-derived light and dark tokens. Tabs,
selectors, inspectors, cards, people lists, forms, disclosures, invitations,
sharing, administration, cross-Vault relationships, lifecycle badges, and
operational states use warm neutral surfaces, blue product emphasis, readable
status colors, local Funnel typography, and JetBrains Mono identities. HTML,
JavaScript, authorization, request construction, and responsive geometry are
unchanged.

## Implementation Evidence

- `implement` session: `/root/ticket_7_access_workflows`
- `tdd` used: yes, at the approved Product Client contract seam
- Red test: the existing suite rejected removal of an exact purple RGBA value
  from the primary invitation option
- Green implementation: the obsolete decorative assertion was removed while
  the retained DOM, layout, access behavior, and seeded verifier checks passed
- Refactor: access-specific presentation literals were replaced in place by a
  localized semantic token layer; no behavior refactor
- Full checks: 40 Rust server tests, Product Client deterministic suite, seeded
  verifier (11 Folders, 54 Pages, 54 Graph nodes, 41 edges), JavaScript syntax,
  Rustfmt, Clippy with warnings denied, app build, and diff check all pass

### Visual evidence

For both `light` and `dark`, screenshots are recorded under
`/tmp/finite-brain-ticket7-{theme}-{access,share,vaults}.png` for:

- resumed Folder access overview and people/share-link lists;
- expanded share-link fields, disabled state, accept action, and destructive
  revoke action;
- Vault selector, loaded/available Vault states, Load action, and expanded
  organization-Vault creation form.

Browser assertions: Session Lock resumed to `Session unlocked`; the active
Folder remained `getting-started`; both themes had zero horizontal overflow and
zero page errors. The light-mode access content that was nearly invisible
before the change is readable after the token migration.

## Review Instructions

Review only issue #7's slice unless a severe cross-slice regression is present.
Keep standards and spec findings separate. Confirm that no decorative CSS
declaration test is being treated as a public behavior contract.

## Reviewer Output

```text
STANDARDS_STATUS: pending
STANDARDS_FINDINGS:
- pending

SPEC_STATUS: pending
SPEC_FINDINGS:
- pending
```
