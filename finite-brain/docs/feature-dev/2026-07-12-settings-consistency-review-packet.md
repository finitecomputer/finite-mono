# Review Packet

## Issue

- Issue: #10 — FiniteBrain Settings, Brain, and Access UI
- Slice type: AFK continuation
- Acceptance criteria: Settings remains truthful to the session, signer,
  invitation, and Folder Key boundaries; locked state offers only Session;
  controls use one legible hierarchy; the responsive modal remains usable; no
  backend or cryptographic behavior changes.
- Baseline: `d88bffa`
- Current diff: working tree continuation from `d88bffa`

## Implementation Summary

Settings now presents Session and signer, Brain context, Folder access, and
Invitations as a coherent four-section surface. A locked or resuming client
has only the Session surface. The copy distinguishes a Member Identity invite
from the email bootstrap path, keeps the explicit Folder Key boundary, and
places invite feedback with the active Settings flow. The pass also removes
duplicate Access hierarchy, exposes a clear join path, and uses a consistent
action/card treatment across Settings.

## Implementation Evidence

- `implement` session: current Codex thread
- `tdd` used: existing deterministic Product Client contract suite updated
  through real Product Client exports; no test-only source-rewrite seam.
- Red test, if applicable: not retained; the pass corrected existing UI
  behavior and added deterministic externally visible contract coverage.
- Green implementation: `scripts/with-dev-env node
  finite-brain/crates/finite-brain-server/src/product-client.test.js`
- Refactor: locked return focus now targets a visible Session control rather
  than a hidden Brain tab; visual CSS details are not asserted as contracts.
- Commands run: `node --check`; deterministic Product Client test; `cargo
  build -p finite-brain-app --locked`; previously scoped `cargo test -p
  finite-brain-server --locked`, Clippy, formatting, workspace check, and
  diff hygiene; final scoped checks are recorded with the commit.
- Browser: the rebuilt Rust-served client was smoke-tested locally at
  `http://127.0.0.1:4039/client`, plus a disposable opt-in signer/database
  fixture for unlock flows. Desktop light/dark and a 390px viewport covered
  locked, unlocked, Access, and Invitations states.

## Review Instructions

Review only this continuation of #10 unless a severe cross-slice regression is
found. Verify product-truth language against the session-only key lifecycle,
NIP-07 signer boundary, and direct-versus-email invitation semantics. Ensure
the tests exercise public Product Client behavior rather than source-rewrite or
decorative-CSS contracts.

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- Initial test-only source-rewrite seam was replaced with real Product Client
  exports; no remaining P1-or-higher standards concern.
- No secret, backend, crypto, or security-boundary change was introduced.

SPEC_STATUS: pass
SPEC_FINDINGS:
- Initial locked nested-return focus issue was corrected so a hidden Brain tab
  cannot receive focus.
- Decorative CSS regex tests were removed; browser evidence now records the
  actual rendered states.
- Recheck found no remaining P1-or-higher spec concern.
```
