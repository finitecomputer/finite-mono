# Brain Product Language And Setup Reconciliation

## Problem Statement

FiniteBrain currently uses two product words for the same knowledge space:
users naturally say “Brain,” while the Product Client, `fbrain`, public APIs,
errors, skills, and documentation often say “Vault.” This makes the user,
Hermes, and the client appear to operate different products.

The setup paths also fail to reconcile cleanly. A user or their Personal Agent
may create the Personal Brain first, and either the user or an agent may create
an Org Brain first. Although Brain can already establish the correct durable
memberships, the Product Client invents a pending Personal Brain placeholder
and treats it as a loadable space. When an agent-created Org Brain is the user's
only accessible Brain, that placeholder can win startup selection, trigger an
unrequested Personal Brain bootstrap, hide the real Org Brain, and collapse the
failure into an inaccurate signer-and-connection error.

From the user's perspective, every legitimate creation path should converge on
the same server-owned Brain list and access state. A Personal Brain should be
easy to find and create, but it must never be a prerequisite for opening an Org
Brain. The Product Client and Hermes should discover each other's completed
work without duplicating Brains, relationships, confirmations, or setup steps.

## Solution

Make a Greenfield product-language hard cut from **Vault** to **Brain**. The
product is **FiniteBrain**; each contained knowledge space is a **Brain**; the
two kinds are **Personal Brain** and **Organization Brain**, with **Org Brain**
as the accepted short form. Named Org Brains are spoken naturally, such as
“Acme Brain.” Folder remains the access boundary inside a Brain.

Update every product-facing first-party contract together: Product Client,
managed skill, `fbrain`, public APIs, identifiers, errors, docs, and tests.
Current development and deployed test data may be reset because there are no
production users. Do not build a Vault-to-Brain migration, dual vocabulary, or
compatibility API without evidence of a real external consumer. Private
storage names may retain `vault` temporarily only when they do not leak into a
product-facing contract.

Separate accessible Brain state from creation actions. **Create Personal
Brain** is an ordinary action, never a fake Brain. On open, unlock, refresh, or
a completed client mutation, the Product Client obtains the signed accessible
Brain list from Brain and selects only a Brain that actually exists. A targeted
Brain link wins; otherwise the current session selection wins; otherwise an
existing Personal Brain wins; otherwise one accessible Org Brain opens; with
multiple Org Brains and no stronger signal, the user chooses. With no Brains,
the client offers both Personal Brain and Org Brain creation, with Personal
Brain as the primary action.

Personal Brain creation remains atomic. User-first creation establishes the
human as sole owner and the selected, identity-resolved agent as the one
Personal Agent. Agent-first creation uses the existing account-bound Agent
Bootstrap Authority after the managed skill's one lightweight natural-language
check. The client does not ask the user to pair or approve that relationship a
second time. Both paths produce the same durable state and are idempotent.

Org Brain creation also converges. Agent-first creation atomically makes the
acting agent and authenticated human requester initial admins under ADR-0025.
User-first Product Client creation visibly offers **Add &lt;agent&gt; as an
admin**, selected by default; the user may turn it off. When selected, Brain
atomically creates the empty Org Brain and both admin memberships. Initial
bootstrap relationships are active immediately and are not invitations.

Hermes uses the signed Brain list through `fbrain`, never client assumptions.
The managed skill proceeds without clarification only when the user explicitly
says Personal Brain or Org Brain. If the type is implied or ambiguous, Hermes
asks one short natural-language question. If a Personal Brain already exists,
Hermes says so and asks whether to use it; if a same-named Org Brain exists,
Hermes asks whether to use it or create a distinct one. These are flexible
behavioral instructions, not server-enforced scripts or authorization checks.

New Brains start empty. An empty Brain with no Folders or Folder Key Grants is
a valid unlocked Brain, not a locked or incomplete session. When either admin
creates an Org Brain Folder, existing required-recipient policy provides all
admins with Folder Access and current Folder Key Grants. Personal Agent rules
continue providing the owner and Personal Agent access to every Personal Brain
Folder.

## User Stories

1. As a user, I want the product and my agent to call each knowledge space a
   Brain, so that I learn one product language.
2. As a user, I want my single personal space called my Personal Brain, so that
   its purpose is obvious.
3. As a user, I want shared spaces called Organization Brains or Org Brains, so
   that I can naturally say “Acme Brain.”
4. As a user, I want Folder to remain the access boundary inside a Brain, so
   that the hierarchy remains understandable.
5. As a user, I want product commands, errors, documentation, and UI to use the
   same language, so that FiniteBrain feels like one system.
6. As a user with no Brains, I want Personal Brain creation to be the primary
   action, so that the common starting point is clear.
7. As a user with no Brains, I want to create an Org Brain first if I choose,
   so that Personal Brain creation is not mandatory.
8. As a user whose agent created an Org Brain first, I want to open it without
   creating a Personal Brain, so that I can immediately use what I requested.
9. As a user with only one accessible Org Brain, I want it to open by default,
   so that the client does not make me select an obvious result.
10. As a user with multiple Org Brains and no Personal Brain, I want the client
    to honor a direct target or current-session selection, so that it opens the
    Brain I intended.
11. As a user with multiple Org Brains and no selection signal, I want to choose
    one, so that the client does not guess.
12. As a user with both Personal and Org Brains, I want a direct Brain link to
    win, so that navigation is predictable.
13. As a user with both Personal and Org Brains, I want my current client-session
    selection preserved, so that refreshes do not interrupt my work.
14. As a user beginning a truly new client session, I want my existing Personal
    Brain to be the default, so that the normal personal starting point remains
    convenient.
15. As a user without a Personal Brain, I want a clear Create Personal Brain
    action in the switcher, so that setup remains easy to find.
16. As a user working in an Org Brain, I do not want Personal Brain setup popups
    or warnings, so that optional setup does not interrupt me.
17. As a user, I want Create Personal Brain to behave only as an action, so that
    it cannot hide or replace an accessible Brain.
18. As a user, I want opening or unlocking Brain to discover current server
    state, so that work performed by Hermes appears naturally.
19. As a user, I want a manual refresh to discover current Brain state, so that
    I can reconcile changes on demand.
20. As a user, I do not want constant polling solely for setup discovery, so
    that this first phase remains simple.
21. As a user, I want a Brain created by Hermes while I work elsewhere to appear
    without switching my active Brain, so that concurrent work does not steal
    my context.
22. As a user, I want a small indication that a new Brain is available, so that
    I can find it when ready.
23. As a user, I want Hermes to provide an Open Brain link after creation, so
    that I can navigate directly to the result.
24. As a security-conscious user, I want an Open Brain link to carry navigation
    but no authority, so that server membership remains decisive.
25. As a user clicking a new Brain link immediately, I want one bounded refresh
    and retry, so that brief visibility races feel seamless.
26. As a user whose targeted Brain is still unavailable, I want a specific
    availability message, so that I do not mistake it for a signer failure.
27. As a user creating my Personal Brain, I want the selected agent shown by
    readable Managed Agent Email, so that I know which agent will be paired.
28. As a user creating my Personal Brain, I want creation and Personal Agent
    pairing to be atomic, so that setup cannot leave a partial relationship.
29. As a user whose selected agent cannot be resolved, I want nothing created
    and a simple setup error, so that Brain fails safely.
30. As a user whose agent created my Personal Brain, I want the client to
    discover and open it without another approval step, so that setup is not
    duplicated.
31. As a user, I want my User Nostr Identity to remain the sole Personal Brain
    owner, so that Personal Agent access never becomes ownership.
32. As a Personal Agent, I want an exact bootstrap retry to return the existing
    Personal Brain, so that timeouts do not create errors or duplicates.
33. As a user, I want a different agent prevented from self-enrolling after my
    Personal Brain exists, so that one-Personal-Agent policy remains intact.
34. As a user creating an Org Brain, I want the selected agent offered as an
    initial admin, so that the reverse setup path is smooth.
35. As a user creating an Org Brain, I want adding that agent selected by
    default but visible, so that the sensible collaboration default is clear.
36. As a user, I want to turn agent inclusion off, so that I can create a
    human-only Org Brain.
37. As a user including an agent, I want the Brain and both admin relationships
    committed atomically, so that neither participant is stranded outside.
38. As a user whose agent creates an Org Brain for me, I want both of us to be
    admins immediately, so that no later “add me” step is required.
39. As a user, I want initial admins described as active access rather than
    invitations, so that I do not expect a nonexistent acceptance step.
40. As an agent, I want a user-created Org Brain to appear through `fbrain brain
    list`, so that I discover it from authoritative state.
41. As an agent, I want an explicitly named Brain to be selected, so that my
    work lands in the requested place.
42. As an agent facing multiple plausible Brains, I want to ask one short
    clarification, so that I do not write into the wrong Brain.
43. As a user asking Hermes to create a Brain, I want Hermes to ask Personal or
    Org unless I explicitly stated the type, so that it does not over-interpret
    vague language.
44. As a user who already has a Personal Brain, I want Hermes to identify it and
    ask whether to use it, so that it does not pretend to create another.
45. As a user with a same-named Org Brain, I want Hermes to ask whether to use
    it or create a separate Brain, so that names need not become identifiers.
46. As an agent, I want conversational clarification to remain flexible skill
    behavior, so that I can respond naturally rather than emit a fixed script.
47. As a user, I want an empty Brain to open normally, so that no starter Folder
    is required.
48. As a user, I want zero Folder Key Grants to be valid when zero Folders
    exist, so that an empty Brain is not mislabeled as locked.
49. As an Org Brain admin, I want every admin to receive required Folder grants
    when a Folder is created, so that either the user or agent can create the
    first Folder without excluding the other.
50. As a Personal Brain owner, I want the owner and Personal Agent to receive
    required grants for every Folder, so that both setup paths converge on full
    operational access.
51. As a user whose Brain access changes concurrently, I want the affected
    session to fail closed and refresh, so that stale authority is not used.
52. As a user removed from a Brain, I want to see “Your access to this Brain
    changed,” so that the failure is understandable.
53. As a user removed from a Brain, I do not want the client to create or switch
    to a Personal Brain automatically, so that access recovery causes no
    unrelated mutation.
54. As a user retrying a timed-out creation, I want the same Brain returned, so
    that retries are idempotent.
55. As a user intentionally creating another same-named Org Brain, I want that
    allowed through a distinct request, so that display names are not forced to
    be unique identifiers.
56. As a developer, I want one Brain ID vocabulary across the Product Client,
    CLI, public API, sync, and sharing, so that first-party integrations agree.
57. As a developer, I want first-party consumers updated together, so that a
    temporary dual public contract does not become permanent.
58. As an operator, I want current development and test deployments reset
    instead of migrated, so that Greenfield delivery stays simple.
59. As a product owner, I want compatibility work added only for a proven
    external consumer, so that speculative migration does not slow the hard cut.
60. As a tester, I want every user-first and agent-first setup permutation
    exercised through the real local stack, so that the client and `fbrain`
    reconcile across actual identity and signing boundaries.

## Implementation Decisions

- **FiniteBrain** is the product. **Brain** is one contained knowledge space.
  **Personal Brain**, **Organization Brain**, and **Org Brain** replace Personal
  Vault, Organization Vault, and Vault in all product-facing language.
- **Brain ID** and `brainId` replace Vault ID and `vaultId` in product-facing
  contracts. `fbrain brain ...` replaces the retired `fbrain vault ...` form.
  Public Brain routes,
  request and response fields, errors, docs, tests, and managed skill guidance
  use only the new vocabulary.
- First-party callers change together. Current local and deployed development
  data may be reset. No migration, dual API, deprecated alias, or compatibility
  layer is included without a demonstrated external consumer.
- Private database tables or internal-only types may temporarily retain legacy
  names when they do not leak into a public or agent-facing contract. New
  product-facing work may not introduce the retired vocabulary.
- Existing focused Personal Agent and agent-created Organization Brain specs
  remain product authorities for their bounded authorization rules. Their
  terminology and statements superseded by ADR-0026 or ADR-0027 are updated to
  agree with this umbrella spec.
- The accessible Brain list contains only durable Brains returned by signed
  Brain server state. Create Personal Brain is a separate action model and is
  never inserted as a pending or synthetic Brain.
- Startup selection priority is: explicit Brain target; valid current-session
  selection; existing Personal Brain; the sole accessible Org Brain; otherwise
  a user choice among accessible Org Brains. With no Brains, show creation
  actions rather than selecting a placeholder.
- Last-Brain selection is not made into a new durable cross-device preference.
  It survives only within the active client session. Direct links carry a Brain
  target, and a truly new session uses the normal selection priority.
- The Product Client refreshes accessible Brains on open, unlock, manual
  refresh, and completion of a client-side Brain mutation. This scope adds no
  polling or real-time subscription.
- A newly discovered Brain never automatically replaces a valid active Brain.
  The client exposes a restrained availability indication and lets the user
  select it.
- Hermes creation responses name the Brain, summarize initial access, and
  provide a navigation link targeting the Brain ID. Navigation does not confer
  membership or signing authority.
- A targeted open performs at most one refresh-and-retry before reporting that
  the Brain is not yet available to the account. It does not initiate Personal
  Brain creation.
- Opening, unlocking, listing, or loading an existing Brain is read-only with
  respect to Brain creation. Personal Brain creation occurs only through the
  explicit client action or an explicit agent setup flow.
- User-first Personal Brain setup names the selected agent by Managed Agent
  Email, resolves its Agent Principal through Finite Identity, and atomically
  creates the user-owned Personal Brain plus the one Personal Agent
  relationship. Resolution or persistence failure leaves neither partial
  state.
- Agent-first Personal Brain setup retains ADR-0024's standing, account-bound
  authority and one natural-language check. The client trusts the resulting
  Brain state and requires no second pairing ceremony.
- Personal Brain bootstrap remains one-per-user, atomic, concurrency-safe, and
  idempotent for the established Personal Agent. A different agent cannot use a
  retry or race to enroll itself after the Brain exists.
- User-first Org Brain creation visibly offers the selected, identity-resolved
  agent as another initial admin, selected by default. The user may turn it off.
  With it on, creation and both admin memberships are one atomic operation.
- Agent-first Org Brain creation retains ADR-0025: authenticated requester and
  acting agent become initial members and admins atomically.
- Initial bootstrap roles are active memberships, not invitations. Invitations
  remain only for post-creation membership workflows.
- Exact operation retries reuse a stable creation identity and return the
  existing result. A distinct explicit Org Brain creation may reuse a display
  name but receives a different Brain ID.
- New Personal and Org Brains start empty. An unlocked empty Brain needs no
  Folder Key Grants because no Folder exists. Folder creation remains the
  atomic point that creates a Folder Key and every required grant.
- The managed FiniteBrain skill proceeds without a Brain-type clarification
  only when Personal Brain or Org Brain is explicit. Otherwise it asks once in
  natural language. This is behavioral guidance, not a hard-coded prompt or
  authorization boundary.
- When a Personal Brain exists, Hermes identifies it and asks whether to use it
  for the requested work. When a same-named Org Brain exists, Hermes asks
  whether to use it or intentionally create another.
- `fbrain` selects an explicitly named Brain, may use one unambiguous match,
  and exposes ambiguity rather than silently defaulting to Personal Brain.
- A protected request that proves current Brain access was lost locks the
  affected session, clears session secrets/plaintext under existing policy,
  refreshes accessible Brains, and reports an access-change message. It does
  not create or automatically select an unrelated Brain.
- Product error presentation distinguishes setup cancellation/resolution,
  unavailable target, access change, session/signing failure, and server
  connectivity. Sensitive raw details remain out of user-facing messages, but
  every failure is not collapsed into the same generic banner.

## Testing Decisions

- Tests observe externally visible Brains, memberships, roles, selection,
  content access, and errors rather than private helper calls, database rows, or
  exact conversational wording.
- The primary acceptance seam is one full local-stack scenario matrix using
  real Core account-agent associations, Finite Identity resolution, Finite Chat
  Hosted Device user signing, Brain signed HTTP, Product Client navigation,
  managed FiniteBrain skill behavior, an Agent Runtime, and `fbrain`.
- The full-stack matrix starts from reset Greenfield state and covers: user-first
  Personal Brain; agent-first Personal Brain; user-first Org Brain with the
  default selected-agent admin; user-first human-only Org Brain; agent-first Org
  Brain with authenticated requester; and Org-Brain-first access when no
  Personal Brain exists.
- Each matrix path proves the client and agent observe the same Brain ID,
  readable identities, membership roles, empty initial Folder state, and later
  Folder accessibility. It proves that no unintended Personal Brain,
  invitation, duplicate Brain, or partial relationship appears.
- The highest-value regression starts with a hosted user identity and agent,
  creates only an agent-created Org Brain with both admins, opens the Product
  Client, and proves the Org Brain opens unlocked while Create Personal Brain
  remains a separate action.
- The full-stack seam also covers direct Open Brain navigation, one bounded
  visibility retry, concurrent list refresh, no active-context stealing, exact
  creation retries, same-name Org Brain clarification, and access removal while
  a session is active.
- Focused Product Client tests support the full-stack seam by covering the
  deterministic selection matrix, strict separation of accessible Brains from
  creation actions, valid empty unlocked state, refresh triggers, targeted-open
  retry bound, and specific safe error categories.
- Signed Brain HTTP integration tests cover atomic user-first and agent-first
  bootstrap, stable Brain IDs, idempotent retries, competing Personal Agent
  rejection, rollback, and required Folder-grant recipients.
- CLI contract tests cover Brain-named commands and fields, explicit Brain kind,
  ambiguous selection behavior, stable retry identity, and absence of retired
  product language.
- Managed-skill tests cover explicit Personal/Org requests, ambiguous requests,
  existing Personal Brain clarification, same-named Org Brain clarification,
  authenticated requester use, and creation response navigation without
  asserting exact natural-language sentences.
- Hosted Device and dashboard adapter tests prove a user identity can identify,
  sign, list, and open an accessible Org Brain without any Personal Brain.
- Static repository checks reject retired Brain product language in designated
  public surfaces while allowing explicitly documented private legacy storage
  names until they are safely renamed.
- Final verification runs focused tests during implementation, relevant
  component suites, the full local-stack matrix, and the monorepo quality gates.

## Out of Scope

- A Vault-to-Brain data migration or compatibility service.
- Preserving current local, staging, or development test data.
- Supporting a proven external consumer that has not yet been identified.
- Renaming private database tables or internal-only types solely for cosmetic
  consistency when they do not leak into product-facing contracts.
- A durable cross-device last-selected-Brain preference.
- Polling, WebSockets, server-sent events, or another real-time Brain-list
  subscription.
- Creating a Personal Brain automatically during client open, unlock, Org Brain
  access, or error recovery.
- More than one Personal Agent, agent self-enrollment after Personal Brain
  bootstrap, or changes to Personal Agent replacement and revocation policy.
- Automatically including an agent when the user turns off the visible Org
  Brain creation choice.
- Treating initial bootstrap membership as an invitation.
- Changing post-creation invitation, sharing, deletion, recovery, export, or
  Folder access policy beyond terminology and reconciliation required here.
- Adding scripted chat controls, slash commands, modals, or cryptographic proof
  for Hermes's lightweight clarification behavior.
- Seeded starter Folders or content. Every new Brain remains empty.
- Native application key custody changes.

## Further Notes

- ADR-0026 records the default-on but visible user-first Org Brain agent pairing
  decision. ADR-0027 records the true product-language hard cut.
- ADR-0024 remains authoritative for standing agent Personal Brain bootstrap.
  ADR-0025 remains authoritative for authenticated requester inclusion in
  agent-created Org Brains.
- “Create Personal Brain” being an action rather than a synthetic accessible
  Brain is the smallest direct correction for the reproduced prototype bug.
- The Brain server remains the shared source of truth. The Product Client and
  Hermes reconcile by rediscovering signed state rather than coordinating
  directly or inferring which surface created it.
- The existing Product Client tests remain green despite the reproduced bug,
  demonstrating the need for the explicit Org-Brain-first regression and the
  full-stack matrix.
