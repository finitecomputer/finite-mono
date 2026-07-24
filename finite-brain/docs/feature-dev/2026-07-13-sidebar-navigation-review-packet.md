# Sidebar Navigation Consolidation — Review Packet

## Issue

- Issue: PR #16 continuation — Sidebar Navigation Consolidation
- Slice type: tiny isolated Product Client UI continuation
- Acceptance criteria:
  - Remove the separate far-left activity rail.
  - Put Files, Graph View, Search, Quick switcher, and Brain access in the
    top File-sidebar section without changing their behavior.
  - Preserve semantic labels, active state, keyboard behavior, and responsive
    usability.
- Baseline: `af12b52`
- Current diff: `git diff af12b52 -- finite-brain/crates/finite-brain-server/src/lib.rs finite-brain/crates/finite-brain-server/src/product-client.css finite-brain/crates/finite-brain-server/src/product-client.html finite-brain/crates/finite-brain-server/src/product-client.test.js finite-brain/scripts/verify-obsidian-product-client.mjs`

## Implementation Summary

The Product Client now has one sidebar. The existing five navigation buttons
sit in a labeled navigation row beside the File-sidebar heading, and the shell
grid gives the reclaimed rail width back to the workspace. No handler or
product workflow was rewritten.

## Implementation Evidence

- `implement` session: current Codex thread, recorded in
  `2026-07-13-sidebar-navigation-session.md`.
- `tdd` used: served `/client` structural contract updated first; it failed
  against the old rail and passed after the implementation.
- Red test: `cargo test -p finite-brain-server --locked product_client_serves_spine_assets_and_config` failed because `sidebar-primary-nav` was absent.
- Green implementation: targeted served-client test, deterministic Product
  Client suite, formatting, diff check, full finite-brain-server suite,
  clippy, build, and browser smoke pass.
- Browser evidence: all five controls are 40x40px header targets; Search,
  Graph View, Files, Quick switcher Escape focus restoration, and Brain access
  routes work. Desktop, 1000px, 390px, and 320px layouts have no horizontal
  overflow.

## Review Instructions

Review only this issue's slice unless a severe cross-slice regression appears.
Keep standards and spec findings separate. Check the acceptance criteria,
public-interface behavior, old-rail removal, breakpoint geometry, and absence
of unrelated workflow changes.

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- No actionable finding. A redundant base Graph shell rule was identified and
  removed before final review.

SPEC_STATUS: pass
SPEC_FINDINGS:
- No missing behavior. The requested narrow-viewport browser verification was
  completed after review and passed.
```
