## Issue

- Issue: #23
- Slice type: Graph hidden-filter removal and current asset contract
- Acceptance criteria: no hidden filter affordance/state/binding; full local
  graph remains privacy-bounded; real controls remain; served/static assets agree
- Baseline: `b946ed6`
- Current diff: `b946ed6...72b5d82`

## Implementation Summary

The Graph View now always projects all decrypted accessible Pages. The invisible
filter control and all filter-specific behavior are removed; usable floating
graph controls remain in place. The Rust asset assertions and fixture verifier
now reject the old filter and require the current controls.

## Implementation Evidence

- `implement` session: `/root/ticket_23_graph_filter`
- `tdd` used: yes
- Red/green coverage: filter absence in HTML/CSS/JS, retained graph controls,
  simplified graph APIs, static asset contract, and served Product Client
- Commands run: deterministic tests, JS/verifier syntax, isolated fixture
  verifier, formatting, focused server asset test, full server test suite, and
  diff hygiene

## Reviewer Output

```text
STANDARDS_STATUS: pass after one P2 correction
STANDARDS_FINDING_FIXED:
- Legacy test calls still passed removed filter-era arguments. Tests now use the
  hard-cut one-argument Graph APIs.

SPEC_STATUS: pass
FINDINGS:
- No user-facing findings. The Graph remains a local client-decrypted projection
  and the verifier cleanup only removes baseline-stale markers.
```
