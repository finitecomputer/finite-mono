# UI Changes Notes

Status: second implementation pass complete and verified locally.

Working surfaces:

- `/dashboard/machines/runtime_ca7c708251ba9125c885/chat`
- `/dashboard/machines/runtime_ca7c708251ba9125c885/connections`

## Recording Rules

- Preserve the user's wording and intent.
- Record observations separately from proposed solutions.
- Add enough route, state, and element context to find the UI again.
- Capture acceptance criteria when the desired result is clear.
- Mark ambiguity as an open question instead of resolving it during annotation.

## Implementation Pass — 2026-07-13

- UI-001: implemented a shared Agent navigation above Topics. Devices is no
  longer exposed. Sites is deliberately visible-but-disabled until it has a
  real product destination; Skills remains permission-gated by the existing
  dashboard authorization rule.
- UI-002: implemented. Preview is the only chat top-right action.
- UI-003: deferred. The legacy interaction was traced, but the current Hosted
  Web Device API does not expose the active runtime's slash-command/skill
  registry. Adding the old relay command would violate the current dashboard
  product boundary; see Backend and architecture notes below.
- UI-004: implemented. Initial retry attempts stay in the loading state and
  surface an error only after retry exhaustion.
- UI-005: implemented by removing nested chat-row left padding.
- UI-006: implemented. Only the selected chat receives selected-row styling.
- UI-007: implemented. The Topics-level creation action was removed; per-topic
  New Chat and the bottom New Chat action remain.
- UI-008: implemented with a deterministic client-side name hash selecting an
  accessible bounded foreground/background palette.
- UI-009: implemented as a first frontend information-architecture pass.
  `/dashboard` remains the Account view, includes agent cards, direct Chat and
  Agent actions, New Agent, and Billing. Agent-scoped non-chat pages now share
  a left Agent shell instead of the `Agent / Connections / Chat` tab bar.
- UI-010: implemented. Connections renders the complete layout immediately;
  when live status is unavailable, state is labeled unavailable and mutating
  actions remain disabled. No integration is presented as connected without a
  real status response.
- UI-011: implemented with a shared Account menu used in Account, Agent, and
  chat contexts.

### Backend and architecture notes

- **Backend/API work required for UI-003:** expose a product-owned, typed,
  read-only command/skill catalog for the selected hosted agent. It should
  include names, aliases, descriptions, argument hints, and CLI-only
  visibility. The dashboard can then implement legacy keyboard/filtering
  parity without routing a product feature through the legacy relay protocol.
- **Product route/data contract required for Sites navigation:** chat Preview
  already opens real site artifacts, but there is no agent-scoped Sites page.
  The nav stays disabled until that destination and its authorization/data
  contract exist.
- **Frontend rearchitecture completed for UI-009:** the shell split is
  dashboard-only and does not change Core or runtime behavior. A later polish
  pass may lift the chat topic state into a shared route layout if seamless
  route-to-route persistence becomes a requirement; that is not necessary for
  the current Account/Agent distinction.

## Second Implementation Pass — 2026-07-13

- UI-012: implemented one persistent `AgentSidebar` in the dashboard shell.
  Agent, Connections, Brain, and Chat remain separate routes whose main content
  changes inside that shell. The old Chat-specific sidebar implementation was
  removed rather than retained as a visual copy. Topics and chats remain
  visible on every agent route, and selecting one enters Chat.
- UI-013: implemented. The selected-agent switcher is part of the shared
  sidebar and now appears in Chat in the same position as every other agent
  route.
- UI-014: implemented desktop collapse and one responsive mobile drawer model
  for the shared sidebar. The Chat top bar and non-Chat agent pages both expose
  the drawer trigger at mobile widths.
- UI-015: implemented with a shared `AgentHeroCard`. Account renders the same
  large status-aware card used by the individual Agent view, once per agent,
  with direct Chat and Agent actions.
- UI-016: implemented. Standard-billing accounts always receive a visible
  billing action wired to the existing Stripe portal/checkout server action.
  Missing local Stripe configuration now produces an explicit server error
  instead of silently suppressing the entry point.
- UI-017: implemented. The redundant centered Account label and its reserved
  header content were removed.

### Second-pass architecture and backend impact

- The route model did not change: Agent, Connections, Brain, and Chat remain
  distinct URLs. The rearchitecture is limited to the dashboard frontend shell
  and shared components.
- No new backend endpoint or data-model change was required. The sidebar reuses
  the existing hosted-chat state, update, and action endpoints; Billing reuses
  the existing Stripe portal/checkout action.
- Chat still maintains its transcript-specific live projection while the
  persistent sidebar maintains its route-level topic projection. That means
  two read-only update consumers are active on the Chat route. Consolidating
  those into one shared hosted-chat provider would be a larger, riskier state
  refactor; it is not required for one shared sidebar or route persistence, but
  remains a worthwhile internal optimization if a strict single-stream
  invariant is desired.
- Local browser verification covered Account, Agent, Connections, and Chat. A
  route transition retained one sidebar, one agent switcher, and the same three
  topic groups while the main heading changed to Connections. Chat rendered one
  shared sidebar, no legacy nested sidebar, the transcript composer, and
  Preview.

## Notes

### UI-012 — Use one shared Agent sidebar across all Agent views

- Surfaces: Chat, Agent, Connections, and Brain views for a selected agent.
- Observation: Chat and the non-chat Agent views currently render separate
  sidebar structures. They look related but are not the same persistent shell;
  Chat owns a richer sidebar containing Topics and chats, while Agent,
  Connections, and Brain use a separate sidebar implementation.
- Desired outcome: use the same Agent sidebar and shell across Chat, Agent,
  Connections, and Brain.
- Content model: the entire sidebar should be shared—not parallel components
  that happen to look alike. Its stable content includes the brand, selected
  agent/switcher, agent navigation, Topics and chats, New Chat controls,
  responsive/collapse behavior, and account menu.
- Visibility clarification: Topics and chats remain visible in Agent,
  Connections, and Brain, not only in Chat. Selecting a chat from any Agent
  route navigates the main content into that chat while preserving the same
  sidebar instance and state.
- State implication: moving among Agent, Connections, Brain, and Chat should
  preserve the selected agent and the shared sidebar's presentation state when
  practical. Chat-only state must not leak into unrelated agents.
- Acceptance direction: the sidebar does not visually jump or reconstruct into
  a different navigation model when changing Agent routes; shared controls have
  identical placement, styling, keyboard behavior, and responsive behavior;
  Topics appear when appropriate for Chat without duplicating the surrounding
  shell.
- Planning note: evaluate an agent-scoped route layout with composable sidebar
  content before implementation. Avoid duplicating the Hosted Web Chat topic
  state into a second source of truth.
- Feasibility: possible with a moderate frontend rearchitecture. Move the
  sidebar out of both `HostedWebChat` and `DashboardShell` into one
  agent-scoped route shell, then extract the existing hosted chat state loading
  and topic/chat actions into a shared client provider or store owned by that
  shell. Route pages render only the main workspace content.
- Backend impact: no new backend is currently expected. The existing hosted
  chat state/action endpoints already provide the topic and chat projection.
  Re-evaluate only if keeping that projection alive outside Chat reveals an
  ownership/claim restriction that cannot be resolved in the shared client
  shell.
- Risk/size: moderate rather than a rewrite. The sensitive parts are avoiding
  duplicate EventSource connections, preserving pending chat state during
  navigation, and keeping server-rendered Agent pages composable inside a
  client-owned shell.

### UI-013 — Put the selected-agent switcher in Chat

- Surfaces: shared Agent sidebar, including Chat.
- Observation: Agent, Connections, and Brain currently display the selected
  agent switcher (`Paul Sol 1`), while Chat does not.
- Desired outcome: the selected-agent switcher belongs to the shared sidebar
  and therefore appears in Chat in the same location and form.
- Acceptance direction: the selected agent is always explicit; switching an
  agent from Chat moves into the selected agent's context without retaining or
  displaying topics from the previous agent.
- Dependency: implement as part of UI-012, not as another Chat-specific copy of
  the switcher.

### UI-014 — Make the shared Agent sidebar collapsible and mobile-accessible

- Surfaces: Agent, Connections, Brain, and Chat at desktop and mobile widths.
- Observation: the non-chat Agent sidebar has no collapse control and becomes
  fully hidden on mobile, leaving no clear way to access its navigation.
- Desired outcome: the one shared sidebar supports desktop collapse and a
  mobile drawer/sheet interaction from every Agent route.
- Desktop acceptance direction: a visible, keyboard-accessible collapse control
  uses the same behavior and compact state across Agent routes; collapsing does
  not discard the selected agent or chat state.
- Mobile acceptance direction: the sidebar starts closed when appropriate but
  every route exposes a persistent menu trigger; opening it shows the complete
  shared sidebar; navigation closes the drawer and returns focus predictably.
- Responsive constraint: do not solve mobile by permanently removing the
  navigation from the document or by creating a separate mobile-only nav model.

### UI-015 — Reuse the Agent-view hero card for every Account-view agent

- Surface: Account view agent collection.
- Observation: Account currently renders each agent as a compact utility row,
  while the Agent view already has a more expressive hero card with the agent's
  visual identity, name, status, and primary Chat action.
- Desired outcome: reuse that Agent-view hero card in Account for each agent.
  Large cards are intentional; the agent collection is the Account view's hero
  content and may contain multiple full-size cards.
- Component constraint: extract one shared agent hero/card component rather
  than maintaining Account and Agent copies that drift visually.
- Action model: each card should provide direct Chat access and a clear path
  into the corresponding Agent view. The entire card may be navigable only if
  that does not conflict with its explicit actions and keyboard semantics.
- Multiple-agent acceptance direction: cards stack or form a responsive grid
  without shrinking into the current compact row; every card preserves the
  same visual quality and status truthfulness as the single Agent-view hero.
- Data constraint: reuse the real project/runtime status already available to
  Account. Do not synthesize an online state when the runtime is unavailable.

### UI-016 — Make Billing an actionable Stripe entry point

- Surface: Account view Billing card.
- Observation: the Billing section can render as inert descriptive content,
  especially when the current Stripe-management eligibility condition hides
  the button.
- Desired outcome: always present an obvious Billing action. For standard
  billing, it must enter the real Stripe flow using the existing server action:
  open the Stripe customer portal when a customer/subscription exists, or the
  appropriate Stripe checkout/setup flow when one does not.
- Acceptance direction: the Billing card is never a dead end; `Manage billing`
  or the appropriate setup label is visible and keyboard-accessible; clicking
  it results in a real Stripe destination or a clear configuration/error state,
  not silent absence.
- Local-development behavior: if Stripe is not configured locally, retain the
  visible action and explain why the real Stripe destination cannot be opened.
  Do not make the entire card appear operational or fabricate a portal URL.
- Backend impact: no new billing backend is expected because
  `openBillingPortalAction` and checkout fallback already exist. Planning must
  verify why the Account view currently suppresses the action and remove that
  presentation-only dead end safely.

### UI-017 — Remove the redundant Account header label

- Surface: centered `Account` text in the Account-view top header.
- Observation: the surrounding content and lack of an active agent already
  make the Account context clear.
- Desired outcome: remove the centered `Account` label and its reserved layout
  space.
- Acceptance direction: the header retains the Finite brand and shared account
  menu, balances cleanly without an empty center artifact, and does not replace
  the removed label with another redundant context title.

### UI-001 — Put the primary product navigation above Topics

- Surface: chat sidebar, above the `Topics and chats` navigation.
- Observation: primary navigation is currently split between the account menu
  at the bottom and controls elsewhere in the shell.
- Desired outcome: place the main navigation above Topics. The intended
  destinations are Connections, Sites, Brain, Skills, and Agent. Hide Devices
  because it is not currently supported.
- Related constraint: Preview remains a contextual action in the top-right,
  not part of this primary navigation.
- Acceptance direction: the supported product destinations have one obvious,
  consistent home above Topics; unsupported Devices navigation is absent.

### UI-002 — Make Preview the only top-right navigation action

- Surface: chat top bar, top-right actions.
- Observation: Connections, Devices, and Preview currently appear together.
- Desired outcome: Preview is the only navigation/action displayed in the
  top-right. For now it opens the Sites preview.
- Product direction: this control can later become a "contextual artifact
  viewer" as the product supports additional artifacts such as Markdown
  documents, PDFs, and ebooks.
- Acceptance direction: Connections and Devices are absent from the top-right;
  Preview remains available and opens the current Sites preview experience.

### UI-003 — Add legacy-style skills autocomplete to the composer

- Surface: `Message your agent` chat composer.
- Observation: the current composer does not expose the skills autocomplete
  available in legacy Finite.
- Desired outcome: give this composer the same skills autocomplete behavior as
  legacy Finite.
- Evaluation direction: document the legacy trigger, filtering, keyboard
  navigation, selection, dismissal, and insertion behavior before implementing;
  verify parity for each behavior rather than only matching appearance.

### UI-004 — Remove the transient error flash during chat loading

- Surface: chat message/loading region.
- Observation: an error sometimes flashes immediately before chat finishes
  loading successfully.
- Desired outcome: smooth the loading transition so recoverable initialization
  states do not briefly present as failures.
- Acceptance direction: initial connection and room hydration show a stable
  loading/recovery state; an error is shown only after the app has evidence of
  a real failure or an exhausted retry/timeout; successful loading does not
  flash an error first.

### UI-005 — Remove inner indentation from chat rows in the topic sidebar

- Surface: chat rows nested inside a topic, exemplified by `SaaS QA`.
- Observation: left indentation inside the chat row's box unnecessarily reduces
  the room available for its label.
- Desired outcome: remove the inner left indentation so chat titles have more
  horizontal space.
- Acceptance direction: the clickable row keeps a coherent topic hierarchy
  without spending additional text width on padding inside the row box; long
  labels visibly gain room.

### UI-006 — Do not give the active topic a selected background

- Surface: topic summary row, exemplified by `Home`.
- Observation: both the active topic and active chat can receive distinct
  background treatment.
- Desired outcome: reserve the selected background for the active chat. The
  containing topic should not change background merely because one of its chats
  is active.
- Acceptance direction: selecting a chat highlights that chat only; its topic
  remains identifiable without an active-row background.

### UI-007 — Remove the top-level New Topic plus button

- Surface: `TOPICS` section header and its `New topic` button.
- Observation: the sidebar presents too many plus buttons.
- Desired outcome: remove the top-level plus button beside the Topics heading.
- Acceptance direction: the header-level plus is absent while the intended
  remaining creation actions continue to work.

### UI-008 — Give topics deterministic name-based colors

- Surface: topic identity marker/row, exemplified by `Home`.
- Desired outcome: assign each topic a color in the UI using a deterministic
  hash of the topic name.
- Constraint: this is presentation-only; do not add persisted color state or
  change the topic data model.
- Acceptance direction: the same topic name gets the same color across renders
  and reloads, different names distribute across the palette, and foreground/
  background combinations retain accessible contrast.

### UI-009 — Separate the product into Account and Agent views

- Surface: high-level dashboard information architecture, currently represented
  by the `Agent`, `Connections`, and `Chat` section tabs.
- Observation: the current top-level view mixes account-scoped and agent-scoped
  navigation and does not provide a clear multi-agent home.
- Desired outcome: define two primary product contexts:
  - **Account view:** the out-of-chat home for the person. It shows all of their
    agents, supports launching a new agent, lets them enter chat with a selected
    agent, and provides billing management.
  - **Agent view:** the selected agent's working environment. Chat is its main
    surface, with the left sidebar providing agent-scoped navigation such as
    Connections, Skills, Sites, Brain, and Agent.
- Navigation implication: replace the current `Agent / Connections / Chat`
  section-tab model. Connections is agent-scoped and belongs in the Agent
  view's sidebar rather than beside Chat as a peer high-level context.
- Acceptance direction: users can always tell whether they are managing their
  account/fleet or working with one specific agent; moving from an agent card
  into chat enters that agent's context; returning to Account exposes the full
  agent list and billing rather than a single-agent page.

### UI-010 — Make Connections inspectable in local development

- Surface: local Agent view Connections page.
- Observation: the page currently collapses to `Your agent is taking longer
  than expected. Try again.`, which prevents visual review and UI iteration.
- Desired outcome: unbreak the real local path where practical. At minimum,
  provide a truthful local-development state that renders the Connections UI
  even when connection operations or their backing integration are unavailable.
- Constraint: do not present fake integrations as connected or operational.
  Unsupported actions and unavailable live state should remain explicitly
  labeled or disabled.
- Acceptance direction: local developers can see and inspect the complete
  Connections layout without production access; the generic timeout does not
  replace the whole page; degraded capabilities are represented honestly.

### UI-011 — Reuse the Agent-view account dropdown in Account view

- Surface: account email and Sign out controls in the current non-chat header.
- Observation: this header renders the account email and Sign out as separate
  controls, while the existing Agent/chat view already has an account dropdown.
- Desired outcome: reuse the same account dropdown component and interaction in
  both Account and Agent views.
- Acceptance direction: account identity and Sign out have consistent visuals,
  menu contents, keyboard behavior, and responsive behavior in both contexts;
  there is no second bespoke email/sign-out treatment.

## Open Questions

- UI-012: resolved—Topics and chats remain visible while viewing Agent,
  Connections, and Brain.
- UI-012: should sidebar collapsed/open state persist across agent routes and
  reloads, and should it be remembered independently for each agent?
- UI-012/UI-013: resolved—the selected-agent switcher belongs in the shared
  sidebar and appears in Chat.
- UI-014: decide whether desktop collapsed state persists across reloads or only
  during the current browser session.
- UI-015: decide whether multiple large agent heroes use a single-column stack
  at all desktop widths or a two-column grid on sufficiently wide screens.
- UI-016: confirm the desired local-development error treatment when Stripe
  configuration is intentionally absent; the action itself should remain
  visible.

- UI-001: confirm the exact primary-nav ordering and whether labels are always
  visible or collapse to icons at narrower widths.
- UI-002: decide when the user-facing label should evolve from `Preview` to the
  broader contextual-artifact-viewer concept.
- UI-003: identify the legacy Finite source and exact interaction contract that
  defines autocomplete parity.
- UI-007: confirm whether per-topic plus buttons and the bottom `New chat`
  action are the complete intended set of remaining creation controls.
- UI-008: choose the bounded accessible color palette before implementation;
  the hash should select from that palette rather than generate arbitrary RGB.
- UI-009: decide the persistent control that moves from Agent view back to
  Account view, and how the selected agent is represented when entering or
  switching Agent views.
- UI-009: define the initial Account view layout and empty/loading/error states
  for zero, one, and multiple agents, plus the billing entry point.
- UI-010: determine which local Connections data can come from the real agent
  runtime and which unavailable integrations need explicit development-only
  fixtures or read-only placeholders.
