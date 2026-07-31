# Zero-Plumbing Brain Creation, Folder Creation, And Trustworthy Authoring

## Problem Statement

FiniteBrain's core collaboration workflow now works cleanly for registered
identities, but the surrounding happy paths still ask agents to carry
implementation details that the product already knows or can derive.

Creating an Organization Brain currently requires a raw Brain ID, kind flag,
display name, and raw requester identity. Creating a Folder still treats the
first positional value as a raw Folder ID and teaches examples that repeat the
active Brain, display name, and path. These inputs make ordinary creation
depend on a model correctly transcribing identifiers and plumbing on every
attempt.

Folder Invitations have a related first-contact gap. Brain Invitations already
support an Email Invite Bootstrap when an email does not yet resolve to a
registered Member Identity, but Folder Invitations reject the same recipient.
From the user's perspective, inviting a new person to one Folder should not
require that person to register before the invitation can be sent.

Finally, an agent that expects a domain skill may continue from model memory
when that skill is absent. That can produce polished-looking Brain content
without authoritative sources. FiniteBrain should guide the agent to inspect
the skills that actually exist, use authoritative documentation, preserve
those sources under `raw/`, and stop without writing when authoritative
sources cannot be found.

## Solution

Make the normal FiniteBrain creation and first-authoring workflows express
human intent rather than internal identifiers.

The normal Brain creation command becomes:

```text
fbrain brain create organization "Hermes Agent Knowledge"
```

The CLI derives a readable stable Brain ID from the display name. In an
authenticated Agent Runtime, Runtime-supplied requester context automatically
includes both the requesting human and acting Agent Principal as initial
Organization Brain admins. If authenticated requester context is unavailable,
the agent flow fails closed instead of creating an agent-only Organization
Brain. When a human runs the CLI directly, the signing human is the sole
initial admin. The raw requester-identity CLI option is removed.

From inside an open Brain Working Tree, the normal Folder creation command
becomes:

```text
fbrain folder create "Hermes"
```

The CLI uses the active Brain, derives a readable stable Folder ID, preserves
the supplied value as the display name, derives a safe Folder path, and applies
the existing Brain-kind access default. A derived Brain or Folder name
collision fails clearly and identifies the existing resource; the CLI never
silently creates a numbered duplicate.

Folder Invitations reuse the existing Email Invite Bootstrap model already
used by Brain Invitations. An inviter may target an email that has no
registered Member Identity. The pending invitation is bound to that canonical
email and one Folder. After the recipient registers or verifies the exact
email, the invitation atomically binds the claimant Member Identity, creates
Guest access when needed, and establishes Folder Access Readiness only for the
invited Folder.

The managed FiniteBrain skill gains a fail-closed missing-skill workflow. The
agent first inspects the skills actually installed. If the expected domain
skill is absent, it uses authoritative primary documentation, captures durable
Source Notes or Assets under `raw/`, and writes sourced synthesis under
`wiki/`. If no authoritative source can be found, it explains the blocker and
writes no Brain content from model memory.

## User Stories

1. As a user, I want to create an Organization Brain by naming its type and
   display name, so that I do not supply internal identifiers.
2. As an agent, I want the CLI to derive the Brain ID, so that I cannot mistype
   a slug that the product can calculate.
3. As a user, I want the derived Brain ID to remain readable, so that links,
   diagnostics, and advanced automation remain understandable.
4. As a user, I want my original Brain display name preserved, so that
   identifier normalization does not alter product presentation.
5. As an authenticated human asking an agent to create an Organization Brain,
   I want both of us included as initial admins, so that I can immediately
   govern the Brain created for me.
6. As an agent, I want authenticated requester context supplied automatically,
   so that I do not transcribe a raw human identity.
7. As a security-conscious user, I want agent creation to stop when
   authenticated requester context is missing, so that an agent-only
   Organization Brain is not silently created.
8. As a human using the CLI directly, I want my signing identity to become the
   initial admin, so that direct creation does not require chat context.
9. As a user, I want the raw requester-identity flag removed, so that
   conversational identity text cannot be substituted for authenticated
   Runtime context.
10. As a user, I want Brain creation to remain atomic, so that the Brain and
    its required initial admin relationships cannot partially succeed.
11. As a user, I want a new Organization Brain to remain empty, so that
    simplification does not create sample Folders or content.
12. As a user, I want a duplicate derived Brain name rejected clearly, so that
    retries do not create accidental parallel Brains.
13. As an agent, I want a duplicate response to identify the existing Brain,
    so that I can ask whether to use it.
14. As a user, I want to create a Folder from its display name inside an open
    Brain, so that I do not repeat Brain or Folder plumbing.
15. As an agent, I want the active Brain inferred from the Brain Working Tree,
    so that mutations naturally target the context I opened.
16. As a user, I want the CLI to derive the Folder ID and path, so that the
    display name is the only ordinary Folder input.
17. As a user, I want the Folder's display name preserved, so that readable
    product language is not replaced by a slug.
18. As a Personal Brain owner, I want the existing owner access default
    preserved, so that simplified creation does not change Personal Brain
    policy.
19. As an Organization Brain admin, I want the existing restricted access
    default preserved, so that simplified creation does not widen access.
20. As a user, I want duplicate derived Folder names rejected clearly, so that
    one retry cannot create `hermes-2` silently.
21. As an agent outside an open Brain, I want Folder creation to return
    actionable context guidance, so that the CLI does not guess a Brain.
22. As an advanced operator, I want explicit context selectors available only
    where ambiguity or automation genuinely requires them, so that the happy
    path stays small without eliminating deliberate automation.
23. As an inviter, I want to invite an unregistered email address to one
    Folder, so that registration does not have to happen before coordination.
24. As an invitee, I want the invitation email to guide me through identity
    registration or verification, so that I can claim the intended access.
25. As an invitee, I want the claim bound to the exact canonical invited email,
    so that another registered identity cannot take my invitation.
26. As an invitee, I want acceptance to create bounded Folder access rather
    than Brain Membership, so that unrelated Brain content remains private.
27. As a non-Member invitee, I want acceptance to establish a Guest
    relationship, so that my relationship accurately reflects one-Folder
    access.
28. As an existing Member, I want Folder Invitation acceptance to preserve my
    Membership while granting only the invited restricted Folder, so that
    access semantics remain consistent.
29. As an inviter, I want unregistered-email Folder Invitations to retain the
    normal single-use, expiry, inspection, and pending cancellation lifecycle,
    so that bootstrap does not create a weaker invitation type.
30. As a security-conscious user, I want Folder key material prepared by an
    authorized client and wrapped through the existing Email Invite Bootstrap
    contract, so that the server never becomes a Folder Key holder.
31. As an invitee, I want claim to establish current Folder Access Readiness
    atomically, so that acceptance cannot report success without a usable
    current Folder Key Grant.
32. As an inviter, I want stale bootstrap material invalidated by relevant key
    rotation, so that an old invitation cannot grant obsolete access.
33. As an agent, I want to inspect the skills actually installed before using
    an expected domain skill, so that I do not assume capabilities that are
    absent.
34. As a user, I want a missing domain skill to trigger authoritative-source
    research, so that Brain content is grounded in evidence.
35. As a user, I want captured documentation stored durably under `raw/`, so
    that synthesized knowledge remains traceable.
36. As a user, I want non-Markdown source files stored under `raw/assets/` with
    paired Source Notes, so that provenance survives beyond the agent session.
37. As a user, I want sourced synthesis written under `wiki/`, so that source
    material and curated knowledge remain distinct.
38. As a reader, I want the resulting Page to reference its durable sources,
    so that I can evaluate important claims.
39. As a user, I want the agent to stop when no authoritative source can be
    found, so that model memory is not presented as organizational knowledge.
40. As a user, I want the agent to explain the missing skill or source blocker,
    so that I know what is required to continue.
41. As a maintainer, I want both packaged FiniteBrain skill copies to remain
    identical, so that local and hosted agents learn the same workflow.
42. As a tester, I want ordinary commands exercised without redundant flags,
    so that tests protect the intended agent experience rather than low-level
    compatibility.
43. As a tester, I want independent identities to claim and inspect access, so
    that invitation success is proved from both sides.
44. As a security reviewer, I want failure paths proved non-mutating, so that
    missing context, duplicate names, invalid email proof, and unavailable
    sources cannot leave misleading durable state.

## Implementation Decisions

- The canonical happy-path Organization Brain command is `fbrain brain create
  organization "<display-name>"`.
- Brain kind and display name are positional intent. The CLI derives the Brain
  ID using one core-owned, deterministic, lowercase kebab-case normalization
  that produces an existing valid stable ID.
- Identifier derivation normalizes whitespace and punctuation into single
  hyphen separators, trims separators, enforces the existing stable-ID length
  contract, and returns actionable validation when a non-empty valid ID cannot
  be derived.
- Derived identifiers are not silently suffixed. A collision fails and reports
  the existing resource. Intentional parallel resources require an explicitly
  distinct display name.
- In an Agent Runtime, authenticated requester context is supplied
  automatically from authenticated message metadata. Typed text, quoted text,
  email, profile data, the Agent Principal, and model inference are never
  requester authority.
- The agent-facing CLI removes `--requesting-user-npub` and any equivalent raw
  requester override. The signed server operation may retain an internal
  requester field because Brain still atomically creates two initial admins,
  but ordinary callers do not author that field.
- Missing authenticated requester context fails closed in an Agent Runtime.
  The managed skill asks the user to retry from authenticated chat and does not
  create an agent-only Organization Brain.
- Direct human CLI creation has no Organization Brain Requester distinct from
  the signer. The signing human becomes the sole initial admin. Product Client
  creation retains its existing explicit agent-pairing choice.
- Organization Brain creation remains atomic and starts with no Folders,
  Folder Keys, Folder Key Grants, or sample content.
- The canonical happy-path Folder command inside an open Brain is `fbrain
  folder create "<display-name>"`.
- Folder creation infers the active Brain from an unambiguous Brain Working
  Tree, derives the Folder ID with the same core-owned identifier normalization,
  preserves the supplied display name, and derives a safe top-level Folder
  display path from that name.
- Folder creation preserves existing access defaults: owner for Personal
  Brains and restricted for Organization Brains. It continues to prepare the
  initial Folder Key and required grants in memory.
- Missing or ambiguous active Brain context fails with actionable choices and
  the minimum advanced selector needed. The CLI never guesses.
- Explicit Brain, Folder ID, display-path, access, role, parent, and recipient
  controls are advanced automation or policy inputs. Normal managed-skill
  examples omit them.
- Folder Invitation creation first attempts native identity resolution. A
  concrete Member Identity continues through the existing npub-bound path.
- An email that does not resolve to a registered identity uses the existing
  Email Invite Bootstrap architecture rather than failing or creating a
  placeholder Member Identity.
- The Folder Email Invite Bootstrap is bound to one canonical invited email,
  one native source Brain, one Folder, its current key version, one expiry, and
  one single-use invite code. It never creates a relationship with another
  Brain.
- The authorized inviter prepares only the invited Folder's bootstrap
  authorization and encrypted current Folder Key Grant. The server stores and
  delivers opaque bootstrap material and remains unable to decrypt Folder
  content.
- Claim requires current Identity Authority proof that the claimant Member
  Identity controls the exact invited email. Registration and verification use
  the existing identity flow; FiniteBrain does not create a second identity
  system.
- A successful claim atomically consumes the pending invitation, binds the
  claimant Member Identity, creates or preserves the correct Guest/Member
  relationship, installs explicit access to the invited Folder, and establishes
  a current Folder Key Grant.
- Folder Email Invite Bootstraps preserve Folder Invitation lifecycle rules:
  targeted, single-recipient, single-use, seven-day default expiry,
  one-hour-to-thirty-day custom expiry, pending cancellation, and no
  cancellation-as-revocation after acceptance.
- Relevant Folder Key rotation invalidates stale pending bootstrap material.
  Claim fails closed and requires a fresh invitation rather than granting an
  obsolete key.
- Only native source Brain governance may create the Folder Invitation.
  Folder Mounts cannot be re-invited.
- The managed FiniteBrain skill explicitly requires agents to inspect the
  available installed skills before assuming a named domain skill exists.
- When the expected domain skill is absent, the agent uses authoritative
  primary documentation. It captures Markdown Source Notes under `raw/`,
  non-Markdown Assets under `raw/assets/`, and provenance sufficient to
  evaluate the synthesis.
- Sourced synthesis is written under `wiki/` and connected to its durable
  Source Notes. The existing `index.md` and `log.md` closure rules still apply.
- If no authoritative source can be found, the agent stops, explains the
  blocker, and writes no Page, Source Note, inventory placeholder, or
  model-memory draft for the requested content.
- Both the component-packaged and Runtime-bundled FiniteBrain skills change
  together and remain byte-identical under existing static checks.
- This spec extends the intent-based happy-path work in #249 and the universal
  Folder Invitation delivery slice in #261. It does not restore retired
  sharing commands or alter the Member/Guest/Mount model.

## Testing Decisions

- Good tests assert externally observable commands, signed requests, durable
  resources, access postconditions, Working Tree files, and failure
  non-mutation. They do not assert private helper names, exact SQL layout, or
  incidental prompt wording.
- The primary product acceptance seam is the existing built-process suite: one
  built `fbrain` executable, the real signed FiniteBrain server and store,
  Runtime-equivalent requester context, and independent Finite Homes and Member
  Identities.
- The process seam creates an Organization Brain with `fbrain brain create
  organization "Hermes Agent Knowledge"` and proves the derived ID, preserved
  display name, acting Agent admin, authenticated requester admin, empty Folder
  set, and absence of requester plumbing flags.
- The same seam proves missing Agent Runtime requester context creates no Brain
  and direct human CLI creation produces one signing-human admin.
- Creation tests prove duplicate Brain and Folder names fail without creating
  numbered or partial resources.
- From the opened Brain Working Tree, the process seam runs `fbrain folder
  create "Hermes"` and proves active-Brain inference, derived ID/path, preserved
  display name, correct Brain-kind access default, initial key version, and
  readable canonical Folder projection after sync.
- The process seam creates a Folder Invitation for an email that initially
  resolves to no Member Identity, inspects its pending state, performs real
  identity registration or verification for a second independent Finite Home,
  claims the invitation, and proves that identity can open, edit, and sync only
  the invited Folder.
- Invitation tests prove exact-email binding, wrong-identity rejection,
  duplicate claim idempotence or rejection according to the existing contract,
  expiry, pending cancellation, accepted-access persistence, and stale
  bootstrap invalidation after Folder Key rotation.
- The recipient-side assertion proves Folder Access Readiness: policy access
  and a current Folder Key Grant. Merely observing a Guest row is insufficient.
- Member-versus-Guest assertions prove an unregistered non-Member becomes a
  Guest, an existing Member remains a Member, and neither gains unrelated
  restricted-Folder access.
- The managed-agent scenario seam presents an expected domain skill that is
  absent from the actual installed skill catalog.
- With authoritative documentation available, the scenario proves the agent
  inspects installed skills, captures durable sources under `raw/` or
  `raw/assets/`, writes sourced synthesis under `wiki/`, updates durable
  navigation/history, and does not describe unsupported claims as
  authoritative.
- With no authoritative documentation available, the scenario proves the
  agent reports the blocker and makes no requested Brain content changes.
- Managed-skill static tests continue proving both skill copies are identical,
  happy-path examples omit raw IDs and requester flags, and fallback guidance
  names the canonical `raw/` and `wiki/` conventions.
- Existing core identifier, safe-path, requester-bootstrap, Email Invite
  Bootstrap, Folder Invitation, Guest access, sync, packaged-skill, Brain
  language, API-route, and monorepo gates remain green.

## Out of Scope

- Changing Personal Brain ownership, the one-Personal-Agent rule, or Personal
  Agent replacement.
- Automatically adding an agent when a human creates an Organization Brain
  directly; Product Client pairing remains an explicit choice.
- Creating an agent-only Organization Brain when authenticated requester
  context is missing.
- Allowing raw requester identities, emails, or inferred conversational
  identities as requester authority.
- Silently generating numbered Brain or Folder duplicates.
- Renaming existing Brain IDs, Folder IDs, Folder paths, or durable content.
- Automatically migrating existing resources to derived identifiers.
- Changing Folder access defaults or adding viewer/editor roles.
- Anonymous, reusable, public, multi-recipient, or non-expiring invitations.
- Turning Folder Invitation acceptance into Brain Membership.
- Changing Mount behavior, destination participant governance, or mounted
  Folder resharing rules.
- Making the server a Folder Key holder.
- Building a new registration or email-verification system instead of reusing
  Finite Identity Authority.
- Generating Brain content from model memory when authoritative sources are
  unavailable.
- Automatically installing, generating, or downloading a missing domain skill.
- Broad Product Client visual redesign or enabling unfinished sidebar
  navigation.
- Production deployment or mutation of existing user data.

## Further Notes

- ADR-0025 remains authoritative for atomic agent-created Organization Brain
  requester bootstrap and is clarified to remove the raw requester CLI flag.
- ADR-0009 already defines the privacy and cryptographic shape of Email Invite
  Bootstraps. Folder Invitations narrow that proven mechanism to one Folder;
  they do not create a parallel bootstrap protocol.
- #249 remains the parent design for the hard-cut intent-based access,
  invitation, and Mount surface. #261 remains the delivery slice for universal
  Folder Invitations. This spec supplies the additional unregistered-email and
  zero-plumbing acceptance criteria those issues did not state.
- The current collaboration workflow for registered identities is not being
  redesigned. Email sharing, admin assignment, Folder Key granting, discovery,
  opening, editing, syncing, and conflict verification already work cleanly.
- The source-quality fallback is agent policy enforced through managed-skill
  guidance, static checks, and scenario tests. The CLI cannot determine whether
  prose came from model memory and should not pretend to enforce epistemic
  provenance.
