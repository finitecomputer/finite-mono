# Spec: Obsidian-Style FiniteBrain Settings, Vault, And Access Shell

## Problem Statement

The FiniteBrain Product Client currently puts Vault selection, signer/session
controls, organization creation, Folder access, sharing, invitations, and Vault
member administration into a dense file-sidebar surface. This makes the file
explorer compete with management tasks and makes the session/Vault identity
hard to discover. The supplied Obsidian references show a clearer pattern:
the bottom sidebar row owns identity and Vault context, the Vault name opens a
small switcher, Manage Vaults is a dedicated modal, and a gear opens a settings
modal with a navigable left rail.

## Solution

Refactor the Product Client shell around three presentation surfaces:

1. A compact bottom account/Vault footer with a Vault switcher trigger, session
   identity/status summary, and a gear Settings trigger.
2. A Vault switcher popover that lists the current Member Identity's visible
   Vaults, marks the selected and loaded Vault, and links to a dedicated Manage
   Vaults modal.
3. A Settings modal with a left navigation rail and sections for Session,
   Vault, Access & sharing, and Invitations. The Access ribbon opens this modal
   directly to Access & sharing. The Files sidebar remains focused on file
   browsing and Search remains focused on page search.

The existing Product Client request, crypto, authorization, sync, and Session
Lock implementations remain the source of truth. The refactor relocates and
recomposes their controls without changing their externally visible security
or data behavior.

## User Stories

1. As a Product Client user, I want the bottom sidebar row to show my current
   Vault, Member Identity, and session status, so that context is visible where
   I expect account controls.
2. As a Product Client user, I want a gear icon beside the bottom row, so that
   settings are discoverable without opening the file explorer controls.
3. As a Product Client user, I want clicking the Vault name to open a compact
   switcher, so that I can see and choose visible Vaults without a large form.
4. As a Product Client user, I want the switcher to distinguish selected,
   loaded, locked, and unavailable states, so that choosing a Vault never
   implies that its encrypted content is already open.
5. As a Product Client user, I want a Manage Vaults action in the switcher,
   so that Vault administration has a dedicated surface.
6. As a Product Client user, I want Manage Vaults to list visible Vaults with
   role and kind metadata, so that personal and organization Vaults are easy to
   distinguish.
7. As a Vault owner, I want to create an organization Vault from Manage Vaults,
   so that creation is not mixed into file browsing.
8. As a Product Client user, I want the existing explicit Load/Resume behavior
   to remain available from Manage Vaults, so that selecting a Vault does not
   silently reopen encrypted grants.
9. As a Product Client user, I want Settings to open as a modal over the
   workspace, so that management tasks do not permanently consume sidebar
   space.
10. As a Product Client user, I want Settings to have a left navigation rail,
    so that I can move between management categories without losing context.
11. As a Product Client user, I want a Session section that shows signer and
    Session Lock state, so that I can resume or lock the client deliberately.
12. As a locked-session user, I want the Settings modal to show only safe
    session status and a Resume action, so that the security boundary remains
    obvious.
13. As an unlocked-session user, I want the Settings modal to expose Lock
    session, so that I can clear Session Folder Keys and temporary plaintext.
14. As a Product Client user, I want a Vault section with the current Vault
    summary and Manage Vaults entry point, so that Vault context has one home.
15. As a Vault administrator, I want Access & sharing to contain Folder
    selection, access summaries, people, Folder Key state, and share-link
    actions, so that permissions are managed together.
16. As a Vault administrator, I want existing Folder grant, remove, create
    share link, accept link, and revoke link actions to work from the new
    Settings surface, so that the refactor does not weaken access workflows.
17. As a Vault administrator, I want organization member and administrator
    controls to remain available in Access & sharing, so that existing Vault
    administration remains complete.
18. As a Product Client user, I want Invitations to contain create, inspect,
    accept, revoke, and email-invite flows, so that invite work is separated
    from ordinary file navigation.
19. As a Product Client user, I want the Access ribbon to open Settings to
    Access & sharing, so that the existing navigation affordance remains useful
    without restoring a dense third sidebar mode.
20. As a Product Client user, I want Files and Search sidebar modes to retain
    their existing behavior, so that the information architecture becomes
    clearer without disrupting navigation.
21. As a keyboard user, I want modal close, Escape, focus-visible, and menu
    semantics to work for the switcher and both modals, so that the refactor is
    operable without a pointer.
22. As a mobile user, I want the switcher and settings surfaces to fit narrow
    viewports without horizontal overflow, so that management remains usable
    on compact screens.
23. As a privacy-conscious user, I want the refactor to retain the existing
    prohibition on durable browser storage of Session Folder Keys and readable
    client state, so that presentation changes do not weaken local security.
24. As a maintainer, I want the management surfaces to reuse the existing
    Product Client state and request interfaces, so that UI changes remain
    local to the presentation module.
25. As a reviewer, I want deterministic browser captures of locked, unlocked,
    switcher, Manage Vaults, Settings, Access, and Invitations states, so that
    the end-to-end result is judged from evidence rather than markup alone.

## Implementation Decisions

- Keep the Product Client's existing `FiniteBrainProductClient` state and
  request functions as the authoritative interface for Vaults, Folder access,
  invitations, sharing, and Session Lock.
- Add a small presentation state for the active Settings section and the open
  overlay surface (Vault switcher, Settings modal, or Manage Vaults modal).
  Close overlays on explicit close, Escape, or backdrop activation.
- Replace the footer's details-only interaction with a compact row that has a
  dedicated Vault switcher trigger and a dedicated Settings trigger while
  retaining the identity/status summary and session controls.
- Render the Vault switcher from the same normalized visible-Vault data used by
  the current select and Vault list. Selection updates the active Vault through
  the existing reset/lock path; it does not silently bypass explicit Load or
  Resume semantics.
- Give Manage Vaults a modal surface for visible Vaults, role/kind/status
  metadata, signer connection, explicit Load/Resume, and organization creation.
- Move the dense Vault, Folder access, sharing, member, invitation, and shared
  Folder controls out of the file sidebar and into modal sections. Preserve
  existing element identity where it is already part of Product Client tests or
  event binding, or introduce one new stable hook where a surface needs a
  distinct presentation seam.
- The Settings modal uses a left rail with Session, Vault, Access & sharing,
  and Invitations sections. Its content area remains independently scrollable
  so long access and invitation forms do not resize the workspace.
- The Access ribbon targets Settings → Access & sharing. Files and Search stay
  as sidebar modes; the obsolete dense Access sidebar mode is no longer a
  primary navigation destination.
- Keep the modal and popover presentation on the existing token layer: warm
  neutral surfaces, blue accents, semantic status tones, local Funnel fonts,
  restrained depth, visible focus, and reduced-motion support.
- Keep modal labels and summaries explicit about encrypted state. A selected
  Vault is not necessarily a loaded Vault, and a locked session never implies
  readable content is present.
- Do not add durable browser storage, new backend routes, schema changes,
  cryptographic operations, authorization policy, or production configuration.

## Testing Decisions

- The primary seam is the real Rust-served Product Client at `/client`, using
  the existing local smoke signer and seeded local Vault data. Browser checks
  should exercise the complete interaction paths rather than inspect private
  DOM implementation details.
- Browser verification must cover: bottom-row rendering; Vault switcher open,
  selection, outside-click, and Escape; Manage Vaults open/close, role/status
  display, creation and explicit Load/Resume; Settings navigation and close;
  Access & sharing actions; Invitations; Session Lock/Resume; and desktop and
  narrow-mobile layouts.
- Extend the existing deterministic `product-client.test.js` contract suite
  only for externally visible helper behavior, preserved IDs, overlay labels,
  accessibility hooks, and local-storage/security prohibitions. Do not encode
  individual decorative CSS declarations as the feature contract.
- Run JavaScript syntax checks, Product Client tests, focused Rust server tests,
  formatting, build, diff hygiene, and the existing workspace gates required by
  the repository.
- Capture screenshots from realistic seeded states in light and dark themes,
  at desktop and mobile widths, and review them for clipping, focus visibility,
  modal scroll behavior, and preserved action hierarchy.

## Out of Scope

- Changing Vault, Folder, Member Identity, invitation, share-link, sync, or
  cryptographic semantics.
- Adding new backend routes, database fields, persistent client settings, or
  production deployment/configuration.
- Rewriting the Product Client in a frontend framework or adding a build step.
- Adding a manual theme preference; the existing system-driven appearance seam
  remains authoritative.
- Redesigning the dashboard shell, ribbon icon inventory, Page editor, Graph
  View, or Smoke UI beyond the navigation integration needed for the new modal
  surfaces.
- Removing security status language or implying that a selected Vault is loaded
  when it is only selected.

## Further Notes

- This spec uses the FiniteBrain glossary terms `Vault`, `Folder`, `Member
  Identity`, `Session Lock`, `Session Folder Key`, and `Ephemeral Client
  Plaintext` as defined by `finite-brain/CONTEXT.md` and its accepted ADRs.
- The branch targets `main` because this monorepo has no `staging` branch. It
  starts from the current dashboard-themed Product Client branch so the UI
  refactor is evaluated on the current visual baseline.
