## Issue

- Issue: #25 — make Page and Folder affordances truthful
- Fixed point before session: `c7ca3ef`
- Worker session: `/root/ticket_25_truthful_page_affordances`
- Commit: `7bf2581` plus focused review follow-up
- Status: verified; follow-up ready to commit

## Inputs

- Spec issue: #24
- Ticket: #25
- Relevant glossary terms: Product Client, Page, Folder, Graph View, Session
  Lock, Session Folder Key, Ephemeral Client Plaintext, Source Note
- Relevant ADRs: 0004, 0005, 0008, 0010, 0014
- Prototype answer and source branch, if any: none

## Implementation

- Public interface used: visible Product Client Page reader/editor, Folder
  context menu, Graph View
- Behaviors covered:
  - preserved the existing non-pointer Graph node contract;
  - removed the unavailable Folder delete menu row;
  - removed reader Reading/Source state and control; the single raw editor is
    labeled `Edit Markdown`;
  - made visual Markdown task checkboxes update only the active local Page
    draft, with an action-specific accessible name, using the same Markdown
    block semantics as the visual renderer so fenced code is never toggled;
  - added an explicit `Save Page` action that uses the existing signed save
    path, while retaining Cmd/Ctrl+S and avoiding background writes; the
    normal reader layout gives the Page header its own visible grid row.
- `tdd` used: yes — added failing deterministic contracts for Save Page,
  removal of the duplicate reader mode and Folder delete affordance, Markdown
  task toggling (including fenced-code regression), task checkbox accessible
  names, and the visible Page-header grid before implementation.
- Commands run during implementation:
  - `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
  - `scripts/with-dev-env node --check finite-brain/scripts/verify-obsidian-product-client.mjs`
  - `scripts/with-dev-env cargo test -p finite-brain-server product_client_serves_spine_assets_and_config --locked -- --nocapture`
  - `scripts/with-dev-env cargo fmt --check`
  - `scripts/with-dev-env cargo build -p finite-brain-app --locked`
  - `git diff --check`
- Browser verification: passed in a disposable Rust-served client on a
  Chromium-safe local port. It confirmed the Save Page button had a nonzero
  rendered box and could receive keyboard focus after a draft change; a task
  toggle stayed `draft` and made no protected write, then `Save Page` made
  one signed protected write and reached `rev 1`. Chromium blocks port 4045
  as unsafe, so the isolated fixture used port 8787 instead.
- Full suite command: pending

## Review

- Review fixed point: `c7ca3ef`
- Standards findings: none
- Spec findings: two, fixed before follow-up commit:
  - a fenced code block could contain task-looking text that the old source
    toggle counted before the visual task; toggling now derives the target
    source line from the same Markdown block parse as rendering;
  - the Page header was `display: none`, hiding Save Page; it is now visible
    in an explicit three-row Page layout and browser-proven.
- Worthy fixes applied: action-specific checkbox names (`Mark task
  complete/incomplete: <task text>`), semantic source-line task mapping,
  visible Page header, and a graph no-click test alongside the existing
  no-pointer rule.
- Findings ignored with reasons: none.

## Risks

- Folder deletion remains intentionally absent because Portable v1 has no
  deletion contract for live Folder Objects.
