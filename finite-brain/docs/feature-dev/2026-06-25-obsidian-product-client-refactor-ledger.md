# Obsidian Product Client Refactor Ledger

## Run

- Run ID: 2026-06-25-obsidian-product-client-refactor
- Loop: Feature Dev
- Target repo: sibling `finite-brain` worktree
- Base branch: `main`
- Feature branch: local `main` worktree
- Human owner: Austin
- Started: 2026-06-25
- Current status: simplification pass implemented, rebuilt, locally verified, and checkpointed to main
- Skill setup status: existing `AGENTS.md` and `docs/agents/` setup present

## Goal

Make the prototype Product Client frontend feel much closer to Obsidian, using the supplied Obsidian graph-view reference as the visual target.

## Durable Artifacts

- CONTEXT updates: none
- ADRs: none
- PRD issue: not created for this direct polish continuation
- Slice issues: not created for this direct polish continuation
- Issue sessions: direct in-thread implementation
- Agent briefs: none
- Review packets: this ledger
- Local CodeRabbit report: not run; local deterministic, Rust, and browser visual checks were used
- PR URL: none

## Commands

- Syntax: `node --check crates/finite-brain-server/src/product-client.js`
- Static smoke: `node --check scripts/verify-obsidian-product-client.mjs`
- Product Client seams: `node --test crates/finite-brain-server/src/product-client.test.js`
- Seeded UI smoke: `FINITE_BRAIN_DB=/tmp/finite-brain-polish.sqlite3 FINITE_BRAIN_SMOKE_KEYS=/tmp/finite-brain-polish-keys.json node scripts/verify-obsidian-product-client.mjs`
- Rust/test gate: `cargo fmt --check && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && git diff --check`
- Local run: `FINITE_BRAIN_ADDR=127.0.0.1:4015 FINITE_BRAIN_PUBLIC_BASE_URL=http://127.0.0.1:4015 FINITE_BRAIN_DB=/tmp/finite-brain-polish.sqlite3 cargo run -p finite-brain-app`
- Visual verification: Playwright with a local NIP-07 shim against `http://127.0.0.1:4015/client`

## Evidence

- Page screenshot: `/tmp/finite-brain-obsidian-page-live-2.png`
- Graph screenshot: `/tmp/finite-brain-obsidian-graph-live-2.png`
- Second-pass page screenshot: `/tmp/finite-brain-obsidian-page-live-4.png`
- Second-pass graph screenshot: `/tmp/finite-brain-obsidian-graph-live-4.png`
- Final rebuilt page screenshot: `/tmp/finite-brain-obsidian-page-live-6.png`
- Final rebuilt graph screenshot: `/tmp/finite-brain-obsidian-graph-live-6.png`
- Simplified page screenshot: `/tmp/finite-brain-simplified-page-live-1.png`
- Simplified graph screenshot: `/tmp/finite-brain-simplified-graph-live-1.png`
- Stripped chrome page screenshot: `/tmp/finite-brain-stripped-page-live-1.png`
- Stripped chrome graph screenshot: `/tmp/finite-brain-stripped-graph-live-1.png`
- Pure page screenshot: `/tmp/finite-brain-pure-page-live-2.png`
- Pure graph screenshot: `/tmp/finite-brain-pure-graph-live-2.png`
- Final simplified page screenshot: `/tmp/finite-brain-simplified-page.png`
- Final simplified graph screenshot: `/tmp/finite-brain-simplified-graph.png`
- Browser fixture layout: graph visible, right sidebar hidden in graph mode, brain controls collapsed after load, 47 graph nodes, 37 graph links
- Second-pass browser fixture: target npub `npub17v7g49shev2lwp0uwrx5v88ad6hj970zfse74wkes9jguhkx7aqsgjwsvj`, graph visible, right sidebar hidden in graph mode, 47 graph nodes, 37 graph links, node radius `2.1..4.26`, graph spread `743x472`, ordinary folder details hidden, muted graph stats text
- Final rebuilt browser fixture: target npub `npub17v7g49shev2lwp0uwrx5v88ad6hj970zfse74wkes9jguhkx7aqsgjwsvj`, 47 readable Pages, 9 opened Folder Keys, graph right sidebar `display: none`, graph stats border `0px`, graph controls opacity `0.34`, graph filter collapsed to `2px`, node radius `2.1..4.26`, graph spread `1476x859`
- Simplified browser fixture: target npub `npub17v7g49shev2lwp0uwrx5v88ad6hj970zfse74wkes9jguhkx7aqsgjwsvj`, page grid `44px 318px 1238px 0px`, right sidebar `display: none`, statusbar `display: none`, workspace status dots `display: none`, page header `display: none`, brain controls hidden after load, sidebar footer `11 folders / 47 pages`, graph `47 nodes / 37 links`
- Stripped chrome browser fixture: fake titlebar absent, traffic lights absent, duplicate workspace tab strip absent, app grid starts at the ribbon/files/workspace row, graph remains reachable through the left ribbon, selecting a Page returns to the reader, graph `47 nodes / 37 links`
- Pure browser fixture: no hidden legacy `folderList`/`readerPageList` DOM, no duplicate Files section title, no folder count pill, no file-tree folder detail subtitles, right sidebar/status bar/page header/key footer hidden, brain controls collapsed after load, footer only `11 folders | 53 pages`, page view loads 53 readable Pages, graph view renders `53 nodes / 37 links`
- Final simplified browser fixture: seeded local signer `79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798`, page grid `44px 318px 1238px`, 11 folder buttons, 53 decrypted Pages, no file-tree access badges, no right sidebar/statusbar/sidebar footer, no forbidden prototype copy, graph view renders `53 nodes / 37 links`, and browser console has no errors.
- Served asset smoke: `/client`, `/client/app.css`, and `/client/app.js` include the new Obsidian chrome markers

## Changes

- Added Obsidian-like application titlebar, traffic lights, tab chrome, left ribbon, file explorer density, status dots, and right properties panel treatment.
- Reworked graph view into a full-bleed workspace with muted nodes/edges, compact icon controls, hidden labels until hover, and no right sidebar in graph mode.
- Collapsed brain/auth controls into a subtle disclosure after load so the file tree becomes the primary sidebar content.
- Replaced graph ring placement with a deterministic force-style layout using folder clusters, link springs, repulsion, and a centered hub when present.
- Added static and server asset markers for the new chrome.
- Second polish pass narrowed the left ribbon/sidebar widths, muted diagnostic chips, simplified file-tree detail copy, hid non-active folder subtitles, compacted the right properties panel, and changed graph layout/rendering toward a sparse Obsidian-style particle field.
- Final polish pass hides the graph filter field until hover/focus and scatters unlinked graph nodes across the canvas so the graph reads less like a custom dashboard and more like Obsidian's ambient graph view.
- Repaired the local `/tmp/finite-brain-polish.sqlite3` smoke fixture grants for the target npub so the browser smoke fixture can decrypt all current all-member folders with the same identity used for manual testing.
- Simplification pass removed the visible properties/activity sidebar, bottom statusbar, workspace status pill cluster, page metadata header, key-count footer copy, and prototype-heavy visible labels while preserving the real page reader, file tree, search, access panel, edit drawer, and graph view.
- Stripped top-chrome pass removed the decorative macOS traffic lights, fake top tool icons, fake document tab, plus button, duplicate workspace tab row, and related CSS/JS/test markers. The left ribbon now carries the functional page/graph switching.
- Pure file-tree pass removed dead hidden legacy `folderList` and `readerPageList` surfaces, deleted the duplicate render paths and obsolete reader-list CSS, and extended the static verifier so those prototype-era hooks cannot return unnoticed.
- Final simplification pass default-opens the brain control disclosure before load, removes the hidden OKF plan list from the DOM, turns the stale OKF renderer into a no-op, and updates server asset tests to reject removed prototype copy (`Page Loop`, `OKF Import`, `Plan OKF import`).

## Open Questions

- None for this local prototype polish pass.

## Escalations

- None.
