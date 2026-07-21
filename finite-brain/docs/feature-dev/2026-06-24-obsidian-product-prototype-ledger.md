# Obsidian Product Prototype Ledger

## Run

- Target repo: `finitecomputer/finite-brain`
- Base branch: `staging`
- Feature branch: `feature/guided-smoke-brain-reader`
- Staging PR: `finitecomputer/finite-brain#30`
- Human goal: evolve the Product Client from a smoke-test dashboard into a
  lightweight but complete Obsidian-like prototype that preserves FiniteBrain
  Brain, Folder, Page, access, sharing, sync, graph, and crypto behavior.

## Skill Setup

- `AGENTS.md`: present.
- `docs/agents/issue-tracker.md`: GitHub Issues via `gh`.
- `docs/agents/triage-labels.md`: Matt Pocock default role labels.
- `docs/agents/domain.md`: single-context repo; read `CONTEXT.md` and `docs/adr/`.

## Alignment

- Product direction: copy Obsidian's basic layout and interaction model rather
  than inventing a new shell.
- FiniteBrain boundary: keep the Product Client first-party and local-trusted;
  do not introduce server plaintext search/import/graph behavior.
- UI scope: left ribbon, file explorer sidebar, top workspace tabs, Page view,
  Graph view, right-click menus, top buttons, status/access affordances, and
  FiniteBrain-specific Brain/Folder/share/access controls.
- Prototype boundary: static Rust-served HTML/CSS/JS remains acceptable for the
  internal prototype. A full rich Markdown editor and mobile parity are out of
  scope for this feature-dev run.

## Issues

- PRD: `#31` PRD: Obsidian-like Product Client prototype
- Slice 1: `#32` Add Obsidian workspace shell to Product Client
- Slice 2: `#33` Add Obsidian-style folder and Page context menus
- Slice 3: `#34` Promote Graph View into an Obsidian-like workspace pane
- Slice 4: `#35` Surface FiniteBrain access and sharing in the Obsidian shell
- Slice 5: `#36` Harden Obsidian Product Client prototype verification

## Verification Commands

- `node --check crates/finite-brain-server/src/product-client.js`
- `node crates/finite-brain-server/src/product-client.test.js`
- `cargo fmt --check`
- `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `git diff --check`
- local browser/screenshot verification against `http://127.0.0.1:4015/client`

## Slice Sessions

### `#32` Obsidian Workspace Shell

- Baseline: `216e3f1`
- Implementation checkpoint: `ef28643`
- Status: implementation in progress; first local pass verified
- Owner: current orchestrator thread using direct implementation because the
  first slice is the shell foundation for later slices.
- Current implementation:
  - Replaced the dashboard body with an Obsidian-like shell: left ribbon,
    file sidebar, workspace tab strip, Page workspace, Graph View workspace,
    right property/activity rail, and status bar.
  - Added real sidebar modes for Files, Search, and Access.
  - Added expandable/collapsible folder rows and Page rows in the file tree.
  - Added prototype-safe folder/Page context menus with open, new Page, copy
    id, access/share, graph, and disabled delete affordances.
  - Kept advanced crypto/OKF/Page-loop controls available in a collapsed
    developer drawer rather than making them the primary product workflow.
  - Expanded `scripts/seed-smoke-doc-pages.mjs` so local smoke brains can be
    deterministically filled with 53 real FiniteBrain docs-themed encrypted
    Pages across every seeded Folder.
- Verification:
  - `node --check scripts/seed-smoke-doc-pages.mjs`
  - `node --check crates/finite-brain-server/src/product-client.js`
  - `node crates/finite-brain-server/src/product-client.test.js`
  - `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
  - `cargo test --workspace`
  - `cargo fmt --check`
  - `git diff --check`
  - Runtime endpoint check against `http://127.0.0.1:4015/client`
  - In-app browser screenshot/layout verification: shell rendered, Files panel
    visible, Page workspace visible, Graph workspace hidden until selected.
  - Smoke DB has 53 encrypted current objects and all 53 opened through the
    Product Client keyring/decrypt path.
- Visual note: standalone headless Chromium screenshot capture hung on this
  machine after loading browser background services; in-app browser
  verification succeeded.

### `#33` Folder And Page Context Menus

- Baseline: `ef28643`
- Implementation checkpoint: implemented inside the shell foundation and
  verified before local branch review
- Status: implementation complete; behavior is covered by Product Client seam
  tests and browser-visible shell smoke
- Owner: current orchestrator thread; this slice was absorbed into the initial
  Obsidian shell pass because context menu wiring shares the same sidebar tree,
  selection, and workspace state.
- Current implementation:
  - Folder context menus include open folder, new Page, new Folder inside,
    copy Folder id, manage access, share folder, and disabled delete
    affordances.
  - Page context menus include open Page, copy Page id, copy Folder id, graph
    view routing, and disabled delete affordance.
  - Context menus clamp to the viewport so tooltips/menus do not render off
    screen in normal desktop viewports.
  - Context actions either update active Folder/Page/workspace/sidebar state or
    produce explicit activity-log output for intentionally stubbed operations.
  - Menu routing for Manage Access and Share Folder was later tightened in
    `#35` so FiniteBrain-specific actions open the Access inspector directly.
- Review:
  - Standards axis: self-review against ADR 0004 and the Obsidian product
    direction found the simple DOM menu appropriate for the static
    Rust-served prototype.
  - Spec axis: #33 acceptance criteria are covered by `contextMenuItemsForTarget`
    tests, viewport positioning helper behavior, menu routing, and the browser
    shell smoke.
- Verification:
  - `node --check crates/finite-brain-server/src/product-client.js`
  - `node crates/finite-brain-server/src/product-client.test.js`
  - `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo fmt --check`
  - `git diff --check`
  - Browser smoke through the in-app browser: `/client` renders the context menu
    host and Obsidian shell, and later Access/Graph actions route into their
    workspace surfaces.

### `#34` Graph View Workspace Pane

- Baseline: `fa0532d`
- Implementation checkpoint: this commit
- Status: implementation complete; local checks and browser verification passed
- Owner: current orchestrator thread using direct implementation for the
  Obsidian graph workspace slice.
- Current implementation:
  - Promoted Graph View into a fuller workspace pane with a compact local graph
    topbar, graph stats pill, full-canvas graph stage, floating Fit/Reset
    controls, and replay overlay.
  - Replaced the fixed mini graph drawing with deterministic viewport-aware
    graph layout and stats helpers.
  - Kept Graph View and Graph Replay derived only from the Product Client's
    decrypted accessible Page index, preserving the ADR 0005 privacy boundary.
  - Added Enter-to-render behavior for the graph filter and reset behavior that
    clears filters without losing the active Page selection.
  - Added deterministic tests for graph visibility filtering, graph stats,
    graph layout bounds/determinism, hub placement, and workspace view-state
    switching.
- Review:
  - Standards axis: pass against `AGENTS.md`, `CONTEXT.md`, ADR 0004, and ADR
    0005.
  - Spec axis: initial gap found for view-state switching test coverage; fixed
    with `workspaceChromeState` and Product Client tests before commit.
- Verification:
  - `node --check crates/finite-brain-server/src/product-client.js`
  - `node crates/finite-brain-server/src/product-client.test.js`
  - `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo fmt --check`
  - `git diff --check`
  - ASCII scan over edited Product Client files
  - Local server rebuilt and restarted on `http://127.0.0.1:4015/client`
  - In-app browser verification: Graph tab active, ribbon active, Page pane
    hidden, Graph pane visible, shell state `graph`, graph stats present, and
    Fit/Reset controls present.

### `#35` Access And Sharing Shell Surface

- Baseline: `44e134f`
- Implementation checkpoint: this commit
- Status: implementation complete; local checks and visual smoke passed
- Owner: current orchestrator thread using direct implementation for the
  Obsidian access/share shell slice.
- Current implementation:
  - Added a compact Access inspector inside the left sidebar with selected
    Folder title, status pill, badges, detail text, and Manage/Share actions.
  - Added deterministic access badge projection for admin-only, restricted,
    shared, setup, locked, open-key, and key-version states.
  - Added Folder menu routing so Manage Access and Share Folder select the
    Folder, switch to the Access sidebar, and set the visible intent instead of
    only logging.
  - Added compact sidebar badges for restricted/admin/shared/locked states
    while keeping full badge detail in the Access inspector.
  - Kept OKF import and Page write controls in the existing Advanced client
    tools drawer.
  - Expanded the smoke docs seed content so every seeded Folder has richer
    FiniteBrain-themed pages for local UX testing.
- Review:
  - Standards axis: self-review against `AGENTS.md`, `CONTEXT.md`, ADR 0004,
    and ADR 0005 found no worthy follow-up before commit.
  - Spec axis: #35 acceptance criteria are covered by helper tests, panel
    rendering, menu routing, and the retained advanced/dev drawer.
- Verification:
  - `node --check scripts/seed-smoke-doc-pages.mjs`
  - `node --check crates/finite-brain-server/src/product-client.js`
  - `node crates/finite-brain-server/src/product-client.test.js`
  - `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo fmt --check`
  - `git diff --check`
  - Smoke DB reseeded with 53 encrypted current objects; all 53 opened through
    the Product Client decrypt path.
  - Local server rebuilt and restarted on `http://127.0.0.1:4015/client`.
  - In-app browser verification: Access ribbon switches the sidebar to the new
    inspector, no-signer state is disabled and explicit, and the page remains
    in the Obsidian shell.

### `#36` Prototype Verification Hardening

- Baseline: `18c31ac`
- Implementation checkpoint: this commit
- Status: implementation complete; local checks and browser smoke passed
- Owner: current orchestrator thread using direct implementation for the
  verification hardening slice.
- Current implementation:
  - Added `scripts/verify-obsidian-product-client.mjs` as a no-new-dependency
    smoke verifier for the Obsidian Product Client prototype.
  - The verifier checks static HTML/CSS/JS shell markers, opens the seeded
    Folder Key Grants, decrypts the docs-rich fixture, validates Page
    navigation rows, validates Graph View projection, and validates
    access/share panel helpers.
  - Hardened the verifier so seeded fixture folders/pages are enforced while
    extra human-created smoke-test folders do not make the fixture check
    brittle.
  - Expanded the Rust static asset test and parity runbook so the Obsidian
    shell, graph pane, access inspector, context menu, and repeatable smoke
    verifier stay covered.
  - Normalized Product Client folder metadata handling across camelCase,
    snake_case, and enum-style access values so UI rows do not leak
    `undefined` when metadata shape drifts.
- Review:
  - Standards axis: self-review against `AGENTS.md`, ADR 0004, ADR 0005, and
    the parity runbook found the verification split appropriate for the current
    NIP-07 browser boundary.
  - Spec axis: #36 acceptance criteria are covered by the repeatable fixture
    verifier, browser shell/Graph smoke, full workspace checks, and PR update
    follow-up before final staging review.
- Verification:
  - `node --check scripts/verify-obsidian-product-client.mjs`
  - `node --check scripts/seed-smoke-doc-pages.mjs`
  - `node --check crates/finite-brain-server/src/product-client.js`
  - `node crates/finite-brain-server/src/product-client.test.js`
  - `node scripts/seed-smoke-doc-pages.mjs`
  - `node scripts/verify-obsidian-product-client.mjs`
  - `cargo fmt --check`
  - `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `git diff --check`
  - `curl -fsS http://127.0.0.1:4015/health`
  - `curl -fsS http://127.0.0.1:4015/client/app.js | rg 'normalizeAccessValue|access_mode|AdminOnly'`
  - Browser smoke through the in-app browser: `/client` renders
    `.obsidian-shell`, Files sidebar, Page workspace, Access inspector, and
    Graph workspace; Graph ribbon switches shell state to `graph`.
- Browser boundary:
  - The automation browser reports `window.nostr` missing, so it cannot load
    protected fixture metadata directly. Fixture Page navigation and Graph
    projection are verified through the repeatable Node verifier using the same
    Product Client helpers and seeded Folder Key Grants.

### Final Obsidian Parity Polish

- Baseline: `47e043b`
- Implementation checkpoint: pending commit
- Status: implementation complete; local checks, review, and CodeRabbit passed;
  branch push pending
- Owner: current orchestrator thread using direct implementation as a small
  follow-up polish slice across `#32`, `#34`, and `#36`.
- Current implementation:
  - Replaced placeholder-letter primary ribbon controls with icon-only Files,
    Graph, Search, and Access controls with accessible labels and
    `aria-pressed` state.
  - Kept the development-only Smoke UI link out of the primary product ribbon
    and behind the Advanced client tools drawer.
  - Replaced text/pseudo-content sidebar toolbar controls with real icon
    buttons for New Page, New Folder, and Refresh.
  - Tightened the Obsidian shell geometry with wider ribbon hit areas, active
    rail indicators, tab/tree active markers, tabular numeric counters, root
    font smoothing, explicit transition properties, and tactile press states.
  - Added intentional Page and Graph empty states so unselected, unreadable, or
    filtered-empty views do not look like raw placeholder text.
  - Extended the repeatable verifier so the new sidebar icon controls and empty
    states remain pinned by local smoke checks.
- Verification:
  - `node --check crates/finite-brain-server/src/product-client.js`
  - `node --check scripts/seed-smoke-doc-pages.mjs`
  - `node --check scripts/verify-obsidian-product-client.mjs`
  - `node crates/finite-brain-server/src/product-client.test.js`
  - `node scripts/seed-smoke-doc-pages.mjs`
  - `node scripts/verify-obsidian-product-client.mjs`
  - `git diff --check`
  - `cargo fmt --check`
  - `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo build`
  - Local server rebuilt and restarted on `http://127.0.0.1:4015/client`.
  - Local curl smoke verified `/health`, `/client`, `/client/app.css`, and
    `/client/app.js` serve the polished shell markers.
- Visual verification boundary:
  - Computer Use browser-state verification captured the desktop Product
    Client shell from `http://127.0.0.1:4015/client` in Zen, including the
    icon ribbon, sidebar toolbar, Page empty state, Graph tab switch, centered
    Graph empty state, and right properties/activity rail.
- Review:
  - Standards/spec review packet:
    `docs/feature-dev/2026-06-24-final-obsidian-polish-review-packet.md`
  - Findings fixed: development Smoke UI demoted from the primary product
    ribbon; graph empty state no longer renders raw SVG placeholder text;
    graph empty copy is filter-aware; verification evidence was expanded.

### Further Obsidian Parity Polish

- Baseline: uncommitted final polish delta
- Implementation checkpoint: current uncommitted delta
- Status: implementation complete; verification, review, and CodeRabbit refresh
  passed
- Owner: current orchestrator thread using direct implementation as a focused
  UI/UX parity pass.
- Current implementation:
  - Added a safe, tiny Markdown reading renderer for decrypted Pages so the
    main workspace shows headings, lists, quotes, code blocks, and internal
    links instead of a raw source dump by default.
  - Added a Reading/Source toggle in the Page header for smoke-testers who
    still need to inspect plaintext source.
  - Added right-rail Outgoing links and Backlinks panels derived from the same
    decrypted Page link extraction used by Graph View.
  - Added status-bar Page/Brain detail for selected path, word count, link
    count, readable Page count, and open key count.
  - Fixed folder tree row layout so Folder labels, page-count details, and
    access badges do not run together.
  - Extended deterministic tests and the seeded smoke verifier for Markdown
    preview blocks, inline link parsing, page link context, stats, and the new
    shell markers.
- Verification:
  - `node --check crates/finite-brain-server/src/product-client.js`
  - `node --check scripts/verify-obsidian-product-client.mjs`
  - `node crates/finite-brain-server/src/product-client.test.js`
  - `node scripts/verify-obsidian-product-client.mjs`
  - `git diff --check`
  - `cargo fmt --check`
  - `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo build`
  - Local server rebuilt and running on `http://127.0.0.1:4015/client` with
    `FINITE_BRAIN_DB=/tmp/finite-brain-smoke-test.sqlite3`.
  - Chromium Computer Use verification confirmed the refreshed shell markers.
  - Playwright fixture smoke against the live client opened the seeded Brain
    with 11 folders, rendered Markdown headings in the Page surface, and
    confirmed the page content host is no longer a raw `<pre>`.
- Review:
  - Review packet:
    `docs/feature-dev/2026-06-24-further-obsidian-parity-review-packet.md`
  - Local CodeRabbit report:
    `docs/feature-dev/2026-06-24-local-coderabbit-further-obsidian-parity.md`
  - Findings fixed: wiki alias display text, zero-readable graph empty-state
    copy, and path/basename-aware link context resolution.

## Local CodeRabbit

### Round 1

- Command: `coderabbit review --agent --type all --base staging`
- Result: completed through the free CLI allowance.
- Report: `docs/feature-dev/2026-06-24-local-coderabbit-obsidian-product-prototype.md`
- Findings: 11 valid findings addressed.
- Fix areas:
  - Empty decrypted Pages now count as readable.
  - Folder collapse state is no longer undone by default selection.
  - Draft object ids now satisfy the local object-id contract.
  - Reader Page rows now get fallback titles consistently.
  - Replay stats use the readable graph source count.
  - Page/Graph tabs, search/filter inputs, and narrow-width right rail behavior
    received accessibility/responsive polish.
  - Smoke verifier and seeder were hardened for repeatable local fixture runs.
- Post-fix verification:
  - `node --check crates/finite-brain-server/src/product-client.js`
  - `node --check scripts/seed-smoke-doc-pages.mjs`
  - `node --check scripts/verify-obsidian-product-client.mjs`
  - `node crates/finite-brain-server/src/product-client.test.js`
  - `node scripts/seed-smoke-doc-pages.mjs`
  - `node scripts/verify-obsidian-product-client.mjs`
  - `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
  - `cargo fmt --check`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `git diff --check`

### Final Polish Uncommitted Delta

- Command: `coderabbit review --agent --type uncommitted`
- Result: completed through the free CLI allowance.
- Report:
  `docs/feature-dev/2026-06-24-local-coderabbit-final-obsidian-polish.md`
- Findings: zero.
- Post-fix verification:
  - `node --check crates/finite-brain-server/src/product-client.js`
  - `node --check scripts/verify-obsidian-product-client.mjs`
  - `node crates/finite-brain-server/src/product-client.test.js`
  - `node scripts/verify-obsidian-product-client.mjs`
  - `cargo fmt --check`
  - `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
  - `cargo build`
  - `git diff --check`
  - live curl smoke for `/health`, `/client`, `/client/app.css`, and
    `/client/app.js`
  - Computer Use browser-state verification in Zen for Page and Graph views.

## PR Review And CI

- PR: `finitecomputer/finite-brain#30`
- PR CodeRabbit trigger: `@coderabbit full review`
- PR CodeRabbit result: unavailable; no bot response appeared and the local CLI
  reported the repo is not connected to an accessible CodeRabbit organization.
- PR fallback report:
  `docs/feature-dev/2026-06-24-pr-coderabbit-obsidian-product-prototype.md`
- CI result: GitHub Actions jobs failed before runner steps.
- CI blocker annotation: recent account payments failed or the spending limit
  needs to be increased.
- Local CI-equivalent evidence remains green:
  - `node --check crates/finite-brain-server/src/product-client.js`
  - `node --check scripts/seed-smoke-doc-pages.mjs`
  - `node --check scripts/verify-obsidian-product-client.mjs`
  - `node crates/finite-brain-server/src/product-client.test.js`
  - `node scripts/seed-smoke-doc-pages.mjs`
  - `node scripts/verify-obsidian-product-client.mjs`
  - `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
  - `cargo fmt --check`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `git diff --check`
