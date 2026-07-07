# CodeRabbit Round: Obsidian Product Prototype

## Round

- Scope: local
- Round number: 1
- Command or trigger: `coderabbit review --agent --type all --base staging`
- Started: 2026-06-24T20:53:00Z
- Completed: 2026-06-24T21:02:28Z
- Availability: completed
- Fallback review thread: none

## Findings To Address

| Finding | Severity | Decision | Notes |
| --- | --- | --- | --- |
| Runbook verifier wording omitted Search while expected behavior included Search. | minor | fixed | Added Search to verifier coverage wording and executable marker check. |
| Empty decrypted Pages were treated as unreadable because `page.text` was checked truthily. | major | fixed | Added status-plus-nullish readable predicate and regression coverage. |
| Selected Folder collapse was immediately undone by default target selection. | minor | fixed | Default selection only expands when choosing a replacement Folder. |
| New Page draft object ids could be shorter than the object-id contract. | major | fixed | Padded generated draft ids and added a seam test. |
| Fallback Page titles could be overwritten by spread order and direct helper callers could receive undefined titles. | major | fixed | Normalized title fallback after spreads and inside `readerPageRows`. |
| Replay graph stats used the latest frame node count as the source count. | minor | fixed | Replay stats now use the readable graph source count. |
| Page and Graph buttons lacked tab semantics. | minor | fixed | Added tablist/tab/tabpanel roles and dynamic `aria-selected`. |
| Search/filter inputs relied on placeholders only. | minor | fixed | Added explicit accessible names to Search, Graph filter, and Folder Key inputs. |
| Narrow-width CSS hid the right sidebar completely. | major | fixed | Narrow-width layout now keeps the right rail reachable as a lower pane. |
| Verifier assumed `seededAdminNpub` was always present. | minor | fixed | Added the same fallback used by the seeding script. |
| Smoke seeder sequence high-water mark drifted across reruns. | major | fixed | Excluded replaced seeded object ids from the sequence high-water query. |

## Findings Not Addressed

| Finding | Reason |
| --- | --- |
| None | All local CodeRabbit findings were valid and addressed. |

## Result

- Continue: yes
- Escalate: no
- Notes: Re-ran JS syntax, Product Client seam tests, smoke seeding/verifier, targeted Rust asset test, full workspace tests, clippy, formatting, and diff hygiene after fixes.
