# Review Packet: Bootstrap Scaffold

## Issue

- Issue: #2 through #6 bootstrap slices
- Slice type: AFK scaffold
- Acceptance criteria: `docs/prd/0001-self-hosted-web-search-extract.md`
- Baseline: empty new repository
- Current diff: root scaffold plus tracker-link backfill through `d7ee319`

## Implementation Summary

The repo now has a Finite-owned service boundary for self-hosted `web_search`
and `web_extract`, including domain context, ADRs, agent setup docs, Latitude
runbooks, SearXNG config, Firecrawl wrapper notes, smoke scripts, GitHub
labels, and GitHub issues.

## Implementation Evidence

- `implement` session: current Codex thread
- `tdd` used: not applicable; docs/scripts scaffold
- Red test, if applicable: not applicable
- Green implementation, if applicable: `scripts/check-static.sh`
- Refactor, if applicable: not applicable
- Commands run:
  - `scripts/check-static.sh`
  - `scripts/doctor.sh lat2`
  - `gh issue list --limit 20 --json number,title,labels,state`
  - `gh repo view finitecomputer/finite-search --json name,url,visibility,defaultBranchRef`

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- No standards findings. The scaffold includes AGENTS.md, docs/agents setup,
  root CONTEXT.md, ADRs for durable decisions, and a run ledger.

SPEC_STATUS: pass
SPEC_FINDINGS:
- No spec findings. The scaffold satisfies the PRD acceptance criteria for a
  main-branch bootstrap repo with context, ADRs, SearXNG/Firecrawl runbooks,
  smoke scripts, and GitHub issue tracking.
```

