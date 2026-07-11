## Parent

#4

## What to build

Complete the Dashboard-Aligned Product Theme as an integrated Product Client by
refining responsive behavior, auditing every state in light and dark modes,
capturing final visual evidence, and fixing any presentation or preservation
regressions found by the full verification story.

## Acceptance criteria

- [ ] The integrated Files, Search, Page/Edit, Graph, Access, menus, dialogs, fields, status states, and empty/busy states are visually consistent across the whole client.
- [ ] Existing narrow-screen ribbon/sidebar/workspace behavior is preserved and polished at representative mobile and tablet widths.
- [ ] Light and dark mode screenshots cover locked and resumed states at desktop and mobile widths, including representative knowledge and access workflows.
- [ ] Automated preservation checks prove required DOM identifiers, workspace structure, Session Lock hooks, storage prohibitions, critical JavaScript behavior, and local theme/font contracts remain intact.
- [ ] JavaScript syntax and Product Client tests, the seeded browser verifier, focused Rust server tests, the full FiniteBrain suite, formatting, Clippy, build, and diff checks all pass.
- [ ] Any visual or functional defects found during integration review are fixed and the affected evidence is regenerated.
- [ ] No production deployment, production configuration, or live-data operation is performed.

## Blocked by

- #6
- #7
