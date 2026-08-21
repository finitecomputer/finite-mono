# Finite Brain user-surface map

Status: living checklist. Every PR that adds, removes, or orphans a
user-facing Brain capability updates this file. The map exists so "does X
exist?" is a lookup, not archaeology.

Grounded in the server route catalog (44 routes under `/v1`), the `fbrain`
verb list, the hosted-device signing operations, and the dashboard state
after the web-client deletion. Verified against the tree on 2026-08-21,
after the auth-kernel cut removed the Core/Identity coordination machinery
(invitation plans, email-proof invites, the departure-fact consumer,
account-bound agent bootstrap).

## Where this chain came from (design lineage)

- **#441 — "Account-agent access cohorts and multi-agent Personal Brains"**
  is the umbrella design issue. It spawned 18 concrete user stories,
  **#442–#459** (mailbox preview, cohort invite/accept, shared invitation
  inbox, reduced-set approval, departure handling, human-anchored authority,
  cohort folder access, bootstrap with cohorts, peer-agent chat management,
  reconciliation, migration, restore/release story).
- **First attempt (closed):** Austin's **#467** (backend: ADR-0045,
  account-agent access cohorts) and **#465** (frontend: unified invite UX in
  the Product Client). Closed as superseded after review rejected the
  boundary mixing and stored-entity cohort concepts; the auth-bridge ideas
  live on as ADR-0046 viewer sessions.
- **Rewrite (this stack):** ADR-0046 in **#489** (roster/departure facts,
  invitation plans with provenance, approval artifacts, per-principal
  invitations), **#529** (papercuts), **#530** (pending key wraps), **#540**
  (web Product Client deleted; chat surface is the human UI), **#543**
  (blessed CLI invite path, `approvals` verbs, CLI↔CLI roundtrip).
- **Next:** chat approval card (same artifact route as
  `fbrain approvals approve`, signed via the hosted device's
  `approveBrainAction`), invitation cards in chat, and the `brain://`
  single-file viewer (brain-surface proposal).

All of #441–#459 remain open; the cross-map below tracks which stories the
rewrite actually serves.

## 1. Life of a brain

| Capability | CLI/agent | Human surface | State |
| --- | --- | --- | --- |
| Create personal brain (user-first) | — | none | **no route** — the agent-first bootstrap route was removed in the auth-kernel cut; re-homed to Finite Core |
| Create organization brain | `brain create organization` | none | OK (agent-driven) |
| List brains / metadata | `brain list`, `brain metadata` | none | viewer territory |
| Rename a brain | — | — | **no route** |
| Delete / archive a brain | — | — | **no route** |
| Transfer ownership | — | — | no route (delegation-grant covers admin roles) |
| Encrypted export (backup) | `brain export` | none | OK |
| Restore/import an export | — | — | **no route** — backup without a user-facing restore |
| Doctor / repair / status | `doctor`, `repair`, `status` | n/a | OK |

## 2. Content

| Capability | CLI/agent | Human surface | State |
| --- | --- | --- | --- |
| Folders create / list / delete, hierarchy | `folder …` | none | OK |
| Rename a folder | — | — | **no route** (delete + recreate) |
| Notes write / read / edit / delete | working tree + `sync` + daemon | none | core loop solid |
| Move / rename objects | move route + tree | none | OK |
| Conflicts + resolve | `conflicts`, `resolve` | none | OK |
| Search (lexical + semantic) | `search`, `search-index` | none | human surface = viewer |
| Wiki | `wiki check` | none | OK |
| Activity / audit | `activity` | none | OK |
| Asset-aware OKF/Obsidian import | — | died with #540's client | **orphaned** (omit-aware sync remains) |
| Graph view / replay | — | died with #540's client | future (viewer Phase 3) |

## 3. Sharing and membership

| Capability | CLI/agent | Human surface | State |
| --- | --- | --- | --- |
| Invite by npub / resolvable NIP-05 | `invite brain create` | none | OK |
| Invite by email (capability Invite Token) | `invite-token create --email` | none | OK (token redeems to membership; email is delivery only) |
| ~~Invite by email → plan → per-principal~~ | — | — | **removed** (auth-kernel cut) |
| ~~Cohort folder invite (mailbox → one Folder)~~ | — | — | **removed** (auth-kernel cut); Folder guests are npub share links |
| Accept invitation | `invite brain accept` | invitation card ✓ (chat) | CLI done |
| Discover my invitations | `invite brain list` (`my-invitations`) | none | OK |
| Revoke invitation | `invite brain revoke` | none | OK |
| Public invite instructions | `…/llms.txt` routes | n/a | OK |
| Members add / remove; roles grant / revoke | `admin …` | none | OK |
| Self-leave a brain | — | — | **no route** (admin removal only) |
| Folder access grant / revoke / ensure | `admin folder-access …`, `admin ensure-access` | none | OK |
| Mounts (cross-brain folders) | full verb set | none | expert-only, fine |
| Departure / offboarding revocation | `admin member remove` (explicit) | n/a | automatic Core-departure consumer **removed** (auth-kernel cut) |
| Approval cards (agent asks, human signs) | `approvals list/approve/deny` (#543) | approval card ✓ (chat, hosted signature) | done; `delegation-grant` only — invite-commit **removed** (auth-kernel cut), validation is purely local |
| File a delegation-grant request | — | — | route exists, no verb |
| Key delivery to invitees | pending wraps on sync (#530) | n/a | automatic; key-holder agent must be online (accepted trade) |

## 4. Identity and signing

Signer ops (`signer …`), `auth import`, and all six hosted-device human
operations (`identifyMember`, `authorizeHttpRequest`, `authorizeBrainEvent`,
`openGrantPayload`, `wrapGrantPayload`, `approveBrainAction`) exist and are
tested. Two standing gaps:

- **Account-principal CLI login does not exist** — an expert human cannot act
  as their real account principal from `fbrain`; `auth login/redeem` is now
  only the Directory's `name@finite.vip` claim flow (the email-only proof
  path died with email-targeted invites), and no export path puts a
  hosted-device key into a CLI home.
- The only human-usable path to the hosted ops is scripted HTTP until the
  chat approval card lands.

## 5. Consumption surfaces

- Agent chat: everything the CLI does, once the skill teaches it.
- Human chat: approval cards and invitation cards render in the chat
  (hosted human-principal signature via `approveBrainAction` /
  `authorizeHttpRequest`); the `brain://` viewer is the remaining surface.
- Human browser: nothing, by design since #540.

## Issue cross-map (#441–#459 → surface)

| Issue | Story | Served by |
| --- | --- | --- |
| #441 | umbrella: cohorts + multi-agent Personal Brains | ADR-0046 roster facts + plans (rewrite) |
| #442 | preview everyone included by a mailbox invitation | ✗ removed (auth-kernel cut): no pre-send resolution; certainty arrives at redemption |
| #443 | invite and accept a ready cohort into a Brain | ✗ removed (auth-kernel cut): "smart" invites belong in the inviter's agent's skill — it sends multiple capability links |
| #444 | invite a cohort into one Folder | ✗ removed (auth-kernel cut); per-npub share links remain |
| #445 | shared Account Invitation Inbox | ✓ `my-invitations` (CLI); human surface pending |
| #446 | approve a reduced participant set | ✗ removed with invitation plans (auth-kernel cut) |
| #447 | narrow acceptance after an agent departs | ✗ removed (auth-kernel cut): acceptance never re-checks a roster |
| #448 | routine administration via human-anchored authority | mechanical ✓ (approvals + hosted ops); chat card pending |
| #449 | later mailbox-addressed Folder access for the cohort | ✗ removed (auth-kernel cut); `ensure-access` by npub/NIP-05 remains |
| #450 | revoke, exclude, and restore one cohort agent | partial: explicit admin revoke ✓; restore = re-invite |
| #451 | bootstrap Organization Brains with the creator's cohort | ✗ removed (auth-kernel cut): `initialAgent*` fields rejected; add agents after creation |
| #452 | bootstrap Personal Brains with every current agent | ✗ removed (auth-kernel cut): account-bound registration re-homed to Finite Core |
| #453 | connect a new agent without blocking launch | ✗ replace-personal-agent route removed (auth-kernel cut) |
| #454 | revoke permanently departed agents | ✗ consumer removed (auth-kernel cut): explicit admin revoke or a future offboarding workflow |
| #455 | manage peer-agent access from authenticated chat | ✗ no chat surface |
| #456 | quiet internal-beta cohort reconciliation plan | ops planning, not code |
| #457 | reconcile existing access with complete grants | partial: `ensure-access` |
| #458 | convert pending invitations + cohort-write cutover | ✗ migration never rewritten after #467 closed |
| #459 | restore cohorts + full release story | ✓ as the #527 drill: empty-target restore proven (process drill + slice Act 15) with runbook |

## Gap register

In flight: chat approval card. Planned: invitation cards, `brain://` viewer,
graph/wiki browsing. Standing: account-principal CLI auth.

Lifecycle holes nothing has ever covered (predate every PR above; product
decisions, not oversights): brain rename, brain delete/archive, self-leave,
export→restore, folder rename.

Orphaned by the client deletion (#540): asset-aware OKF import, graph
view/replay — both need a new home (agent-side import verb, viewer Phase 3).

Rewrite debt from the closed first attempt: #455 (chat peer-agent
management — next after the card), #458 (decided: skip — the rewrite is
additive, not cutover; one compatibility test instead), #459 (landed as the
#527 restore drill: process-level empty-target restore proof, slice Act 15
departure, runbook at finite-brain/docs/runbooks/brain-restore-drill.md).
#444 landed as folder-scoped plans with per-principal share links; its
approval-card escalation rides the card work.

Auth-kernel cut (2026-08): the roster/plan/departure machinery from #489 is
deleted; the kernel-shaped replacements shipped as capability Invite Tokens
(#652) plus the pre-existing npub invitations and share links. What was
honestly given up: pre-send certainty that "this email is that npub" and
automatic cross-product departure revocation.
