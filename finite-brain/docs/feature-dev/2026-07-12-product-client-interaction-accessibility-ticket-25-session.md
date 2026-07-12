## Issue

- Issue: #25 — make Page and Folder affordances truthful
- Fixed point before session: `c7ca3ef`
- Worker session: `/root/ticket_25_truthful_page_affordances`
- Commit: pending
- Status: implementation complete; final review in progress

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
    draft, with an action-specific accessible name;
  - added an explicit `Save Page` action that uses the existing signed save
    path, while retaining Cmd/Ctrl+S and avoiding background writes.
- `tdd` used: yes — added failing deterministic contracts for Save Page,
  removal of the duplicate reader mode and Folder delete affordance, Markdown
  task toggling, and task checkbox accessible names before implementation.
- Commands run during implementation:
  - `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
  - `scripts/with-dev-env node --check finite-brain/scripts/verify-obsidian-product-client.mjs`
  - `scripts/with-dev-env cargo test -p finite-brain-server product_client_serves_spine_assets_and_config --locked -- --nocapture`
  - `scripts/with-dev-env cargo fmt --check`
  - `scripts/with-dev-env cargo build -p finite-brain-app --locked`
  - `git diff --check`
- Browser verification: passed in a disposable Rust-served client on a
  Chromium-safe local port. It confirmed a task toggle stayed `draft` and
  made no protected write, then `Save Page` made one signed protected write
  and reached `rev 1`. Chromium blocks port 4045 as unsafe, so the isolated
  fixture used port 8787 instead.
- Full suite command: pending

## Review

- Review fixed point: `c7ca3ef`
- Standards findings: none
- Spec findings: in progress
- Worthy fixes applied: added action-specific checkbox names after focused
  review: `Mark task complete/incomplete: <task text>`.
- Findings ignored with reasons: pending

## Risks

- Folder deletion remains intentionally absent because Portable v1 has no
  deletion contract for live Folder Objects.
