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
