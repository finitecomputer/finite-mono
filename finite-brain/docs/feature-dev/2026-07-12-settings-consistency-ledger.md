# FiniteBrain Settings Consistency Pass Ledger

## Run

- Run ID: 2026-07-12-settings-consistency
- Loop: Feature Dev continuation
- Target repo: finitecomputer/finite-mono
- Base branch: `main` (the existing Product Client PR explicitly targets `main`)
- Feature branch: `feature/finitebrain-settings-vault-ui`
- Human owner: Austin
- Started: 2026-07-12
- Current status: implementation complete; final review and PR update in progress
- Skill setup status: present (`finite-brain/AGENTS.md` and
  `finite-brain/docs/agents/{issue-tracker,triage-labels,domain}.md`)

## Goal

Make every Settings section accurate to FiniteBrain's Product Client truths and
make the whole modal simpler, more consistently segmented, and easier to
approach without changing Vault, Folder, Member Identity, Session Lock,
invitation, or cryptographic behavior.

## Scope And Decisions

- This is a continuation of Settings spec issue #10 and its open PR #16, not a
  new product capability. It is one AFK consistency slice; no new GitHub issue
  is needed while the existing scope remains sufficient.
- Existing Product Client behavior and `finite-brain/CONTEXT.md` are the source
  of truth. Presentation must not imply that a selected Vault is loaded, that a
  locked Session holds readable data, or that an invitation/link changes access
  before its explicit completion path succeeds.
- No backend routes, durable browser state, data migrations, key handling, or
  authorization policy changes are in scope.

## Durable Artifacts

- CONTEXT updates: none expected; existing glossary terms remain authoritative
- ADRs: none expected; this pass does not introduce a hard-to-reverse policy
  decision
- Spec issue: #10 — existing Settings, Vault, and Access shell spec
- Tickets: #10 continuation, one AFK consistency slice
- Ticket sessions: this implementation session
- Review packets: pending final patch and CodeRabbit review
- Local CodeRabbit report: pending implementation review
- PR URL: https://github.com/finitecomputer/finite-mono/pull/16

## Commands

- Syntax: `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
- Targeted test: `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
- Server test: `scripts/with-dev-env cargo test -p finite-brain-server --locked`
- Build: `scripts/with-dev-env cargo build -p finite-brain-app --locked`
- Visual verification: Rust-served Product Client at
  `http://127.0.0.1:4039/client`; inspect Settings sections at desktop and
  narrow viewport when a local browser is available

## Implementation Notes

- Locked and resuming Settings now expose Session only; navigation, Vault,
  Access & sharing, and Invitations remain hidden until the Session is
  unlocked.
- Session owns signer state, the one contextual signer action, Session Lock,
  and a factual recovery note. Vault is now read-only context plus the explicit
  Manage Vaults navigation action.
- Settings feedback is shared above the active section, so invitation results
  and busy state no longer disappear inside Access & sharing.
- Invitation copy distinguishes direct Member Identity membership/access plans
  from email-bootstrap encrypted Folder Key Grant claims. The Join panel is
  available for a Personal Vault without a pre-existing fragment or code.
- Member Identity input labels and Personal Vault shortcut language now match
  the actual input and mount behavior. Email proof time is placed behind an
  explicit advanced disclosure.

## Verification Seams

The existing Product Client specification and Settings issue #10 establish the
following user-visible seams for this continuation:

- **Settings availability:** the deterministic Product Client seam verifies
  that a locked or resuming Session exposes only Session, while an unlocked
  Session exposes the complete Settings navigation.
- **Served Settings structure:** the Product Client HTML/CSS contract verifies
  that feedback, signer state, recovery disclosure, and invitation language
  appear in the section where a person performs the related action.
- **Invitation outcome language:** deterministic source contracts verify that
  normal Member Identity invites do not promise Folder Keys, while the email
  bootstrap path names its explicit encrypted-grant claim.

These seams deliberately avoid testing key material, network internals, or
server implementation details; the no-backend-change boundary remains intact.

## Browser Evidence

- Rebuilt and served the Rust Product Client locally on port 4039. The original
  local app remains available at `http://127.0.0.1:4039/client` for manual
  smoke testing.
- Used the repository's already-installed Playwright runtime after
  `agent-browser` was unavailable in this environment. An opt-in local smoke
  signer and disposable local SQLite database exercised the Rust-served client
  on port 4040; no production service or user Vault was contacted.
- Desktop, system light and dark: locked Settings showed only Session; Session
  showed signer state and recovery disclosure; unlock exposed all four
  sections; relocking returned to Session only.
- Desktop dark: Vault showed context and the explicit Manage Vaults action;
  Access & sharing had no duplicate Access heading; Invitations showed Join a
  Vault for a Personal Vault and its opened form scrolled within the modal.
- Narrow 390px viewport: Settings fit within the viewport, its four nav labels
  remained visible without truncated subtitles, and Invitations stayed
  operable.
- No functional Product Client asset/API failures or runtime exceptions were
  observed. Chromium requested the pre-existing absent `/favicon.ico`, which
  was the sole console 404.

## Review Evidence

- Independent standards and spec rechecks found no remaining P1-or-higher
  concern after correcting the hidden-tab focus path and replacing the
  temporary test-only source rewrite with real Product Client exports.
- Local CodeRabbit completed one round with one minor documentation finding;
  it was addressed in this ledger. See
  `2026-07-12-settings-consistency-local-coderabbit-round.md`.
- The complete acceptance/review record is in
  `2026-07-12-settings-consistency-review-packet.md`.
- Final scoped checks passed: Product Client syntax and deterministic suite,
  `cargo test -p finite-brain-server --locked` (40 tests), `cargo build -p
  finite-brain-app --locked`, `cargo fmt --all --check`, and `git diff --check`.
  The rebuilt server's `/health` endpoint and `/client` asset version also
  responded locally on port 4039.

## Ticket Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| #10 Settings consistency continuation | AFK | implementation complete | local two-axis + CodeRabbit | addressed | scoped checks pass; local client live |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| #10 Settings consistency continuation | `d88bffa` | current Codex thread | `11b63e3` | standards/spec pass; CodeRabbit minor addressed | scoped checks pass |

## Open Questions

- No blocking product question is currently known. The pass will preserve the
  existing explicit unlock, signer, Vault, access, and invitation flows rather
  than inventing new configuration behavior.

## Escalations

- None.

## 2026-07-13 Continuation: Local Page Discard and Dashboard Token Alignment

### Scope and boundary

- Continue the existing Product Client PR (#16) against `main`; this is a
  focused consistency and reliability slice, not a new product capability.
- Fix the immediate-create then delete path without changing server-side
  authorization, Folder access, Vault policy, or signed tombstone semantics.
- Align foundational Product Client color roles with the finitecomputer
  dashboard while preserving the existing information architecture, layout,
  and interaction model.

### Implemented behavior

- A revision-zero local Page draft is now explicitly labelled and handled as
  **Discard unsaved Page**. Discarding it is local only and never sends a
  tombstone request or requires a signer.
- A persisted Page still uses the existing signed DELETE/tombstone flow. A
  Page with an initial save in flight cannot be deleted until that save settles.
- Reader inputs now follow the fallback Page after either discard or persisted
  deletion, preventing the next edit from reusing the removed Page identity.
- Sync bootstrap removes confirmed deleted objects from the visible projection;
  an actual local-vs-remote conflict remains preserved for conflict handling.
- The basic dark/light surface, neutral selection, action, focus, and popover
  roles use the dashboard-oriented token palette. Blue remains reserved for
  semantic links and search/graph meaning rather than generic selection.

### Verification plan and evidence

- Deterministic Product Client checks cover local discard without a network
  DELETE, persisted signed deletion, fallback input rebinding, sync removal,
  local conflict preservation, and a lock/reopen save race.
- Browser smoke uses a disposable local SQLite database and an opt-in local
  signer only. It verified discard and signed-delete behavior, fresh reload,
  no console errors, light/dark rendering, and a 390px-wide modal layout.
- Final scoped commands passed: Product Client syntax and deterministic suite,
  40 finite-brain-server tests, clippy with warnings denied, workspace fmt,
  `git diff --check`, and `cargo build -p finite-brain-app --locked`.
- The rebuilt main local server is listening at `http://127.0.0.1:4039/client`.
  The browser verification ran separately on a disposable port and database.

### Review record

- Independent final diff review found no remaining actionable concern. It
  specifically rechecked fallback input rebinding for both deletion paths and
  the lock/reopen stale-save guard.
- CodeRabbit's third local uncommitted-review attempt connected through its
  free CLI allowance and reached its summarizing phase, but returned no review
  report or findings. The independent review and the scoped test/browser
  evidence remain the review fallback for this local-only run.

## 2026-07-13 Continuation: Sidebar Navigation Consolidation

### Scope and decision

- Continue the existing Product Client PR (#16) on
  `feature/finitebrain-settings-vault-ui` against the user-selected `main`
  base. This is a tiny, isolated, low-risk Product Client markup/CSS slice;
  the current Codex thread is the recorded implementation owner.
- Remove the separate far-left activity ribbon. Move its existing Files, Graph
  View, Search, Quick switcher, and Vault access controls into one semantic
  navigation row in the header of the existing File sidebar.
- Keep the controls' order, IDs, titles, `aria-label`/`aria-pressed` behavior,
  keyboard focus restoration, command behavior, and active-state semantics.
  No Vault, Folder, Page, Graph View, Session, or access behavior changes.

### Acceptance and verification seams

- The Product Client shell has no `.app-ribbon`; it has one File sidebar and a
  primary navigation landmark within its header.
- Existing script and Rust served-client contracts prove the new landmark and
  reject the old rail. Existing JavaScript IDs preserve handler and focus
  seams.
- Agent-performable visual verification covers desktop, Graph View, Search,
  and a narrow viewport with no horizontal overflow or inaccessible controls.
- Planned checks: Product Client deterministic suite, Product Client static
  verification, finite-brain-server tests, formatting, diff validation, build,
  browser smoke, and review.

### Implementation and verification

- Removed the standalone rail from the Product Client markup and placed its
  existing five controls in `sidebar-primary-nav` within `vault-header`.
  Their IDs and JavaScript behavior remain unchanged.
- The shell grid is now two-column at desktop and medium widths, and a true
  one-column sidebar at narrow widths. The workspace and feedback row shifted
  to their new grid columns; no hidden 44px or 52px rail remains.
- Static Product Client, Rust served-client, and deterministic client
  contracts now require the new header landmark and reject the old rail.
- Browser smoke verified all five 40px header targets, Search, Graph View,
  Files, Quick switcher Escape focus restoration, and Vault access. It passed
  at 1440px, 1000px, 390px, and 320px with no horizontal overflow or browser
  console errors.
- Passed: deterministic Product Client suite, targeted served-client test,
  full finite-brain-server suite (40 tests), clippy with warnings denied,
  formatting, diff validation, and finite-brain-app build.
- The seeded Product Client verifier remains blocked by the documented absent
  local Folder Key manifest; its syntax check passed and the live browser
  smoke covers this slice's public UI behavior.

### Review and publish record

- Independent standards review passed after a single P3 cleanup: remove the
  redundant base Graph shell grid declaration.
- Independent spec review passed; it requested the narrow browser check, which
  was completed before finalization.
- Local CodeRabbit returned a recoverable free-CLI rate limit (26-minute wait)
  with no findings. The fresh independent reviews above are the fallback;
  details are in `2026-07-13-sidebar-navigation-local-coderabbit-round.md`.
- Committed as `00952d4ff808e614e6368ddcb67b269181d762ae`
  (`feat(fbrain): consolidate sidebar navigation`); push remains pending.
- Session and two-axis review artifacts are
  `2026-07-13-sidebar-navigation-session.md` and
  `2026-07-13-sidebar-navigation-review-packet.md`.
