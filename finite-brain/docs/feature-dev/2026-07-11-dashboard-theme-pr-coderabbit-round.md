# CodeRabbit Round: Dashboard Theme PR #9

## Round

- Scope: PR
- Round number: 1
- Command or trigger: `@coderabbit full review`
- Started: 2026-07-11
- Completed: 2026-07-11
- Availability: timed out
- Fallback review thread: `/root/pr9_fallback_review`

CodeRabbit did not acknowledge three full-review triggers, exhausting the
Feature Dev loop's retry budget. A fresh Codex review thread reviewed the full
PR diff against `main` with parallel standards and spec axes.

## Findings To Address

| Finding | Severity | Decision | Notes |
| --- | --- | --- | --- |
| Reduced-motion compatibility was claimed but not implemented | major | fixed | Added a `prefers-reduced-motion: reduce` override and verified computed timing plus working Files/Graph interaction in real Chromium. |
| Graph SVG text bypassed Funnel Sans | minor | fixed | Graph canvas text now uses the shared `--font-sans` token; real Chromium reports Funnel Sans. |
| Search/edit/replay/menu/empty-state screenshots were ephemeral | minor | fixed | Regenerated the full curated matrix and committed seven additional representative states, for 22 durable PNGs. |
| FiniteBrain agent guide named the archived component tracker | minor | fixed | Updated tracker authority to `finitecomputer/finite-mono`, matching root monorepo doctrine. |
| Ticket #5/#6 session records cited nonexistent ADR filenames | minor | fixed | Replaced them with authoritative ADRs 0010, 0014, and 0015. |
| Two historical evidence statements still said 15 images | minor | fixed | Updated traceability to the expanded 22-image matrix. |

## Findings Not Addressed

| Finding | Reason |
| --- | --- |
| Deduplicate Brain font assets with the dashboard and replace explicit route registration with a shared manifest | FiniteBrain must serve the same typography independently when opened outside the dashboard. Owning fixed local bytes and explicit allowlisted routes is intentional, already protected by hash/content-type/cache tests, and avoids runtime or deployment coupling to the dashboard. |
| Deduplicate the 22-image inventory between the issue session and review packet | Both artifacts are intentionally self-contained for different review audiences, and the local CodeRabbit gate explicitly required an unambiguous review-packet inventory. Exact-set checks guard against drift. |

## Result

- Continue: yes, after commit `dbb6c61` and updated CI
- Escalate: no
- Notes: Fallback review found zero security, privacy, data-integrity, session-lifecycle, secret, or live-data issues. The unrelated untracked research file remained untouched.
