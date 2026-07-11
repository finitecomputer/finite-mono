# Spec: Dashboard-Aligned FiniteBrain Product Theme

## Problem Statement

The FiniteBrain Product Client is embedded inside the Finite dashboard, but it
currently presents an independent Obsidian-like purple and charcoal visual
identity. The contrast in typography, color, surfaces, controls, status
language, and light-mode behavior makes Brain feel like a separate application
inside the dashboard rather than a first-party Finite product surface.

The Product Client already contains important, dense workflows for Vaults,
Folders, Pages, local search, Graph View, access management, invitations,
sharing, OKF operations, and Session Lock. Those workflows, their structure,
and their security behavior must not be redesigned or weakened in order to
improve visual cohesion.

## Solution

Introduce a Dashboard-Aligned Product Theme for the entire Product Client. The
theme will carry the Finite dashboard's warm neutral surfaces, blue product
accents, Funnel typography, JetBrains Mono, rounded controls, restrained
shadows, status colors, focus treatments, and system-driven light and dark
appearances through every existing Product Client surface.

The Product Client will retain its current ribbon, file explorer, workspace
grid, information density, responsive structure, DOM identity, and member
workflows. Obsidian-like interaction patterns remain where they are already
useful, but Obsidian's purple/charcoal visual identity does not. The result
should read as a native Finite dashboard surface whether it is embedded under
the dashboard's Brain route or opened directly from the Brain origin.

## User Stories

1. As a signed-in Finite user, I want Brain to look native to the dashboard, so that moving into my knowledge workspace feels continuous.
2. As a user who prefers light mode, I want Brain to use the dashboard's warm light appearance, so that the embedded client does not become an unexpected dark panel.
3. As a user who prefers dark mode, I want Brain to use the dashboard's restrained dark appearance, so that it remains comfortable without reverting to a separate purple theme.
4. As a user moving between Agent, Connections, Brain, and Chat, I want typography and control styling to remain consistent, so that I can recognize one product system.
5. As a user, I want the existing ribbon navigation to remain in the same place and expose the same actions, so that the reskin does not disrupt muscle memory.
6. As a user, I want the Vault controls, signer controls, and Session Lock controls to retain their existing behavior, so that visual changes do not affect security.
7. As a user, I want the file explorer to retain its hierarchy, expansion behavior, context menus, and density, so that navigating large Vaults remains efficient.
8. As a user, I want Files, Search, Access, Page, and Graph surfaces to share the Finite color and surface language, so that no part of Brain feels unfinished.
9. As a user, I want buttons, fields, selects, checkboxes, menus, dialogs, and disclosure panels to use coherent hover, focus, active, disabled, and busy states, so that interactions feel polished and predictable.
10. As a keyboard user, I want visible focus treatments with sufficient contrast in both themes, so that the reskin remains navigable.
11. As a user with a locked content session, I want the locked state to remain immediately recognizable and retain the same Resume flow, so that the theme never obscures the security boundary.
12. As a user with an unlocked content session, I want ready, warning, error, muted, and success states to remain distinguishable, so that operational state is clear without relying on text alone.
13. As a user editing a Page, I want the editor, reading view, slash menu, save state, and code content to retain their layout and behavior, so that theme work does not alter authoring.
14. As a user viewing the local graph, I want nodes, links, controls, filters, empty states, and replay overlays to be legible in light and dark modes, so that graph exploration remains useful.
15. As a Vault administrator, I want access-management panels, people lists, invitation flows, share-link controls, and destructive actions to retain their hierarchy and behavior, so that visual polish does not increase permission risk.
16. As a mobile user, I want the existing compact ribbon/sidebar/workspace behavior to remain intact and visually coherent, so that Brain remains usable at narrow widths.
17. As a standalone Brain user, I want the same theme and fonts as the dashboard-embedded client without depending on dashboard assets, so that direct access remains first-party and reliable.
18. As a privacy-conscious user, I want fonts to load locally from the Brain origin, so that the visual upgrade introduces no third-party network requests.
19. As a returning user, I want the Product Client to continue clearing Session Folder Keys and Ephemeral Client Plaintext at the same lifecycle points, so that theme work does not weaken Session Lock.
20. As a developer, I want the theme concentrated in the Product Client presentation module, so that future visual maintenance has strong locality and does not spread into cryptographic or sync implementation.
21. As a developer, I want the existing Product Client interface and DOM hooks preserved, so that current JavaScript behavior and smoke automation continue to work.
22. As a reviewer, I want screenshots of realistic seeded states in both themes and viewport classes, so that visual completeness can be judged against evidence.
23. As a reviewer, I want automated proof that font assets and core client assets are served correctly, so that standalone deployment behavior is covered.
24. As a maintainer, I want all existing Product Client and FiniteBrain regression gates to remain green, so that the reskin does not conceal functional regressions.

## Implementation Decisions

- Treat Product Client presentation as the primary module being modified. Its existing HTML identifiers and JavaScript behavior remain the stable interface used by member workflows and tests.
- Preserve the existing three-column workspace structure, ribbon, sidebar, main workspace, breakpoint behavior, control inventory, information hierarchy, and interaction density.
- Permit only minimal non-behavioral markup hooks needed to apply consistent branding or typography. Do not rename or remove existing identifiers, controls, regions, or accessibility relationships.
- Do not change Product Client state, event binding, cryptography, authorization, storage, sync, Page, Graph View, access, invitation, sharing, or OKF behavior.
- Replace the purple/charcoal visual identity throughout the Product Client with a native Finite visual identity derived from the current dashboard source.
- Define a coherent presentation token layer for warm neutral backgrounds, elevated surfaces, text roles, borders, blue accents, success/warning/error states, control dimensions, radii, and shadows. Reuse those tokens across the full stylesheet rather than scattering replacement literals.
- Support system-driven light and dark appearances through `prefers-color-scheme`, matching the dashboard's current theme seam without introducing client-side theme state.
- Self-host Funnel Sans, Funnel Display, and JetBrains Mono from the Product Client origin. Use the same vendored font sources and SIL Open Font License provenance as the dashboard.
- Serve font assets through explicit Rust public routes with correct font content types and the Product Client's static-asset cache policy.
- Use Funnel Sans for interface copy, Funnel Display selectively for brand/display headings, and JetBrains Mono for code, identifiers, paths, and technical values.
- Retheme every current surface, including base shell, ribbon, Vault controls, Session Lock, toolbars, file tree, search, access views, Page reader/editor, Graph View, menus, command palette, context menus, disclosures, lists, badges, pills, empty states, busy states, and responsive layouts.
- Preserve semantic status distinctions and destructive-action prominence in both appearances.
- Preserve or improve keyboard focus visibility, readable contrast, reduced-motion compatibility, and existing accessibility semantics.
- Keep standalone `/client` and dashboard-embedded `/client` visually and functionally equivalent.
- Do not introduce a frontend framework, build step, external font provider, remote imagery, analytics, or durable browser theme state.

## Testing Decisions

- The highest test interface is the real Rust-served `/client` running against the existing seeded Product Client fixture and NIP-07 smoke signer.
- Browser verification will exercise representative locked and resumed states at desktop and mobile widths in both light and dark modes.
- Visual evidence will cover Files, Search, Page reading/editing, Graph View, Access management, menus/dialogs, fields, status states, and responsive behavior rather than only the initial empty shell.
- Browser checks will confirm that navigation, Session Lock, Vault loading, Page selection/editing, Graph switching, and Access switching still behave through the same controls.
- Existing Product Client tests and the static verifier will be extended only where needed to prove the theme/font contract and preservation of required DOM hooks, storage prohibitions, security lifecycle hooks, and critical JavaScript behavior.
- Rust public-route tests will verify that every font asset returns the expected local bytes, content type, and cache policy alongside the existing HTML, CSS, JavaScript, config, and smoke-signer routes.
- Tests should assert externally visible behavior or asset contracts, not individual decorative CSS declarations.
- JavaScript syntax, Product Client tests, the seeded verifier, focused Rust server tests, the full FiniteBrain test suite, formatting, Clippy, build, and diff checks are required before completion.
- Visual review is agent-performable and must be repeated after worthy review fixes.
- Existing Product Client smoke and public-route tests are the prior art; no parallel test harness should be introduced.

## Out of Scope

- Changing Product Client information architecture, workspace geometry, navigation placement, or feature inventory.
- Rewriting the Product Client in React, Next.js, or another framework.
- Changing dashboard navigation or the dashboard's Brain iframe container.
- Adding a manual theme switcher or persisting theme preferences.
- Changing NIP-07, Session Lock, key handling, storage, cryptography, authorization, sync, access policy, invitation semantics, sharing semantics, OKF behavior, or server data models.
- Redesigning the Smoke UI.
- Adding new Product Client features or removing existing controls.
- Production deployment, production configuration, live data operations, or customer-facing rollout.

## Further Notes

- The Dashboard-Aligned Product Theme is a presentation contract, not a new authorization or application state.
- The theme should be evaluated against the current dashboard source rather than remembered approximations.
- The existing untracked research document is unrelated and must remain untouched.
- The Feature Dev run targets `main` because this monorepo does not have a `staging` branch; the human explicitly approved that exception.
