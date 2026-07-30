# Intent-Based FiniteBrain Access, Invitations, And Mounts

## Problem Statement

FiniteBrain exposes the same authorization concepts through overlapping CLI
commands and HTTP resources. `access grant` delegates to a permission primitive;
`permissions` mixes membership, administrator roles, and Folder grants;
`share` mixes one-person links, share-source preparation, cross-Brain
invitations, and acceptance; and `mount` only lists the final projection.
Agents must understand implementation order, generated identifiers, Folder Key
rotation payloads, server URLs, signing origins, config locations, and Working
Tree paths before they can express a simple user intent.

This ambiguity already caused real first-contact friction on the released
version. An Agent was taught stale `vault` language, passed repeated
`--config-dir`, `--server`, Brain ID, and path values, received a successful
`doctor` report before a signed request failed from an authorization-origin
mismatch, and had to reconstruct the correct discovery and open sequence.
PR #246 corrects several released-version defaults and Product Client
regressions, but it is based on `main`, while the newer collaboration work is
stacked through PR #172. The final design must deliberately preserve those
first-use fixes on the newer line and must not reproduce flag-heavy examples in
the redesigned access and sharing commands.

The current authorization model also lacks a precise distinction between a
Brain Member and a person who can access only an invited or mounted Folder.
Accepted Share Links and Shared Folder Connections create “limited members,”
while Personal and Organization Brains interpret membership differently. A
one-Folder share can therefore be confused with joining a Brain, and the
existing Organization-only restrictions make sharing capabilities depend on
Brain kind rather than governance.

From the user's perspective, FiniteBrain should expose five understandable
ideas: inspect access, establish complete Organization Brain collaboration,
invite a person, mount a Folder into another Brain, or deliberately use a
low-level administrative primitive. Normal hosted-agent work should require
only the meaningful target or destination. The CLI, HTTP API, Product Client,
Runtime defaults, skills, and tests must all tell that same story.

## Solution

Hard-cut the CLI and signed HTTP API to an intent-based access and sharing
surface.

`access` becomes read-only. `collaborator ensure-admin` remains the
Organization-Brain-only desired-state workflow that establishes both Admin
Brain Role and Folder Access Readiness. `invite brain` offers Brain Membership
to one recipient. `invite folder` offers one Folder to one recipient as Guest
access. `mount` creates and manages a durable, non-copying Shared Folder
Connection between any source and destination combination of Personal and
Organization Brains. `admin` contains the low-level member, admin-role, and
Folder-access mutations.

Introduce Guest as a first-class relationship. A Member belongs to a Brain and
is entitled to every current and future all-members Folder. A Guest belongs
only to explicitly granted Folders and never inherits all-members access.
Brain Invitations create Members. Folder Invitations and Mount participation
create Guests in the source Brain when needed. Personal versus Organization
continues to determine governance—owner and Personal Agent versus admins and
Members—but no longer determines whether a Brain may send or receive an
invitation or Mount.

Replace `/_admin` with `/v1` for normal signed resources and `/v1/admin` for
low-level mutations. Remove all legacy commands, aliases, routes, redirects,
and compatibility behavior in one hard cut.

Make the hosted-agent happy path plumbing-free. The Runtime and binary provide
production server, signing-origin, Finite Home, config, and Working Tree
defaults. Commands infer the current Brain and Folder from an unambiguous Brain
Working Tree. `fbrain open personal` resolves and opens the unique Personal
Brain without copying its generated ID. Explicit server, config, Brain,
Folder, and path overrides remain available for ambiguity resolution, local
development, and advanced automation. `doctor` performs a real signed Brain
request using the same transport and authorization origin as ordinary
commands.

Preserve PR #246's adjacent internal-preview behavior: the unfinished Brain
sidebar link remains hidden, direct internal navigation remains available,
long notes remain scrollable, canonical Brain language is used throughout the
bundled skill, and the hosted Runtime supplies the correct defaults. Broad
Product Client visual redesign remains separate.

## User Stories

1. As a user, I want access commands to use one vocabulary, so that I do not
   need to understand overlapping permission aliases.
2. As an Agent, I want normal commands to express intent rather than mutation
   order, so that a model mistake cannot leave access half-configured.
3. As a user, I want `access` to be read-only, so that inspection cannot
   accidentally mutate authorization.
4. As a user, I want to list access for the current Brain, so that I can
   understand its relationships.
5. As a user, I want to explain access for the current Folder, so that I can see
   why an identity can or cannot open it.
6. As an Organization Brain admin, I want one command to make a known Agent a
   complete admin collaborator, so that role success cannot hide missing Folder
   grants.
7. As a recipient Agent, I want collaboration reported complete only after all
   current Folders are readable, so that I can begin work immediately.
8. As a Personal Brain owner, I want the Personal Agent relationship to remain
   separate from Organization admin collaboration, so that my Brain keeps one
   owner and one Personal Agent.
9. As a user, I want to invite one person or Agent into either Brain kind, so
   that sharing capability does not depend on governance type.
10. As an invited person, I want a Brain Invitation to make me a Member, so
    that the resulting relationship is clear.
11. As a Member, I want access to current and future all-members Folders, so
    that Membership has the same meaning in both Brain kinds.
12. As a Member, I want restricted Folders to remain explicitly granted, so
    that Membership does not override Folder policy.
13. As a user, I want to invite one person or Agent to one Folder, so that I do
    not need to add them to the whole Brain.
14. As a Folder invitee, I want to become a Guest rather than a Member, so that
    unrelated all-members Folders remain private.
15. As a Guest, I want only explicitly granted Folders, so that my access is
    understandable and bounded.
16. As a user, I want Folder Invitations to work from Personal and Organization
    Brains, so that direct Folder sharing is universal.
17. As a user, I want Brain Invitations to work from Personal and Organization
    Brains, so that either governance model may add Members.
18. As a security-conscious user, I want every invitation bound to one email or
    Member Identity, so that forwarding a link cannot extend access.
19. As an inviter, I want one invitation per recipient, so that each lifecycle
    is independently inspectable and revocable.
20. As an invitee, I want an invitation accepted only once, so that a consumed
    delivery handle cannot be reused.
21. As an inviter, I want unused invitations to expire after seven days by
    default, so that forgotten links do not remain valid indefinitely.
22. As an inviter, I want to select an expiry between one hour and thirty days,
    so that I can match the expected coordination window.
23. As an inviter, I want to cancel a pending invitation, so that an unused
    offer can be withdrawn.
24. As an administrator, I want accepted access removed through the actual
    Member or Folder-access workflow, so that canceling an old invitation is
    never confused with revocation.
25. As a user, I want to connect one Folder to another Brain like a shared Slack
    channel, so that both sides work in one source-backed workspace.
26. As a user, I want Mounts to work across Personal-to-Personal,
    Personal-to-Organization, Organization-to-Personal, and
    Organization-to-Organization combinations, so that Brain kind does not
    impose arbitrary sharing limits.
27. As a source controller, I want a Mount Offer bound to one destination Brain
    and one destination controller, so that it cannot be redirected elsewhere.
28. As a destination controller, I want accepting a Mount Offer to create a
    visible Folder Mount without copying content, so that both sides edit one
    workspace.
29. As a destination controller, I want only myself included initially, so that
    acceptance does not expose the Folder to every Member.
30. As a Personal Brain owner, I want my current Personal Agent included when I
    accept a Mount, so that the Agent retains full operational access to my
    Brain.
31. As a source controller, I want acceptance to disclose every initial
    Personal Brain participant before mutation, so that owner-plus-Agent access
    is not surprising.
32. As an Organization Brain admin, I want to add or remove my own Members as
    Mount Participants, so that my organization controls its side of the shared
    workspace.
33. As a Personal Brain owner, I want to manage eligible identities from my own
    Brain as Mount Participants, so that Personal governance remains local.
34. As a source controller, I want destination Guests excluded from Mount
    participation, so that another Brain cannot transitively reshare through
    identities it does not govern.
35. As a source controller, I want only the native source Brain to create new
    invitations or Mount Offers for its Folder, so that mounted Folders cannot
    be chained onward.
36. As either Brain's controller, I want to end the entire Mount, so that either
    side can leave the shared workspace.
37. As a destination controller, I want to remove one Mount Participant without
    removing everyone else, so that roster changes stay local.
38. As a security-conscious user, I want participant removal and Mount
    revocation coupled to Folder Key rotation, so that removed identities
    cannot decrypt future content.
39. As a user, I want the CLI to generate rotation state automatically, so that
    I never author a raw rotation payload.
40. As a user, I want revocation to make no change when a complete safe rotation
    cannot be prepared, so that partial removal does not create false security.
41. As a remaining participant, I want replacement grants committed with the
    new key version, so that revocation does not strand authorized users.
42. As a source controller, I want any native Folder to be invited or mounted,
    so that I do not perform a separate share-source conversion.
43. As a source Brain Member, I want sharing to leave native Folder permissions
    unchanged, so that external access does not alter internal policy.
44. As an administrator, I want low-level member, role, and Folder-access
    mutations grouped under `admin`, so that their risk and scope are explicit.
45. As an advanced operator, I want low-level admin-role mutation available for
    Organization Brains, so that narrow maintenance remains possible.
46. As an advanced operator, I want direct Folder-access grant and revocation
    available, so that I can repair exceptional states without pretending to
    run a broader workflow.
47. As a removed Member, I want all Brain and Folder access removed together,
    so that Membership removal does not silently demote me to Guest.
48. As an administrator, I want a former Member to receive Guest access only
    through a separate explicit action, so that retained access is intentional.
49. As an administrator, I want a Guest with no direct Folder access or Mount
    participation removed from the active roster automatically, so that zero
    access is not presented as a live relationship.
50. As an Agent in a hosted Runtime, I want server, signing-origin, config, and
    Working Tree defaults supplied automatically, so that normal work needs no
    infrastructure flags.
51. As an Agent inside a Brain Working Tree, I want commands to infer the
    current Brain, so that I do not copy generated Brain IDs.
52. As an Agent inside a Folder directory, I want commands to infer the current
    Folder, so that I do not pass a redundant Folder selector.
53. As an Agent, I want `open personal` to resolve the unique Personal Brain, so
    that first contact does not require parsing and copying its ID.
54. As an Agent, I want ambiguous Brain or Folder context to return actionable
    choices, so that the CLI never guesses where to mutate data.
55. As a developer, I want explicit server, config, Brain, Folder, and path
    overrides preserved, so that local harnesses and intentional automation
    remain possible.
56. As an Agent, I want advanced overrides omitted from normal skill examples,
    so that I learn the safe happy path first.
57. As an Agent, I want the bundled skill to use only Brain terminology, so that
    stale Vault commands cannot derail first use.
58. As an Agent, I want `doctor` to exercise a real signed Brain request, so
    that it cannot report success before the next command fails authentication.
59. As an operator, I want transport and signing-origin precedence covered by a
    Runtime contract, so that deployment configuration cannot silently drift.
60. As a user, I want a missing config directory treated as normal before first
    use, so that on-demand secure initialization is not presented as failure.
61. As a developer, I want the CLI and HTTP API to use the same domain
    language, so that one client does not have to translate legacy sharing
    terms.
62. As an API client, I want normal signed resources under `/v1`, so that
    ordinary reads and workflows are not mislabeled as admin operations.
63. As an API client, I want low-level mutations under `/v1/admin`, so that
    administrative primitives are visibly separated.
64. As a maintainer, I want old commands and routes removed rather than aliased,
    so that skills and clients cannot continue learning two interfaces.
65. As a maintainer, I want existing durable content and grants preserved
    through the schema transition, so that a public-interface hard cut does not
    become user-data loss.
66. As a security reviewer, I want ambiguous legacy limited-member state
    migrated without widening access, so that the new Member model cannot expose
    all-members Folders accidentally.
67. As a tester, I want the built executable, real signed server, real store,
    and independent Finite Homes in the primary acceptance path, so that helper
    mocks cannot hide integration failures.
68. As a tester, I want the complete Personal/Organization Mount matrix covered,
    so that universal sharing is proved rather than inferred.
69. As a tester, I want happy-path commands exercised without plumbing flags,
    so that the acceptance test reflects the Agent UX contract.
70. As a tester, I want legacy commands and routes proved absent, so that the
    hard cut cannot regress into compatibility aliases.
71. As an internal tester, I want long Brain notes to remain scrollable, so that
    access redesign does not regress PR #246's Product Client fix.
72. As a product owner, I want the unfinished Brain sidebar link kept hidden
    while direct internal navigation remains available, so that testing can
    continue without advertising unfinished navigation.

## Implementation Decisions

- The canonical CLI groups are `access`, `collaborator`, `invite`, `mount`, and
  `admin`.
- `fbrain access list` and `fbrain access explain` are read-only. The current
  Brain and Folder are inferred when the Working Tree makes them unambiguous;
  advanced selectors remain available.
- The Organization-Brain-only desired-state command is `fbrain collaborator
  ensure-admin`. It retains the complete, partial, and indeterminate receipt
  semantics established by Organization Brain Collaboration and defaults to all
  existing Organization Brain Folders.
- Personal Brain Personal Agent pairing and replacement remain their own
  product workflows. Personal Brains do not gain an admin role.
- Brain Invitation commands are `fbrain invite brain
  create|list|inspect|accept|revoke`.
- Folder Invitation commands are `fbrain invite folder
  create|list|inspect|accept|revoke`.
- Mount Offer commands are `fbrain mount offer
  create|list|inspect|revoke`.
- Active Mount commands are `fbrain mount accept|list|inspect|revoke`, with
  `fbrain mount participant add|remove` for the destination roster.
- Low-level commands are `fbrain admin member add|remove`, `fbrain admin role
  grant|revoke`, and `fbrain admin folder-access grant|revoke`. The only
  grantable or revocable Organization role in this surface is `admin`;
  ownership and Personal Agent authority are not generic roles.
- The old `access grant|revoke`, `permissions`, plural `collaborators`, plural
  `invites`, `share`, `shared`, share-source, Share Link, and Shared Folder
  Invitation commands and aliases are removed without compatibility shims.
- **Member** is a Brain-wide relationship in both Brain kinds. Members are
  entitled to current and future all-members Folders. Restricted Folders still
  require explicit Folder Access Readiness.
- **Guest** is a first-class Folder-limited relationship in both Brain kinds.
  Guests never inherit all-members access. The active Guest relationship ends
  automatically when no direct Folder access or Mount participation remains,
  while audit history is retained.
- Brain Invitations create Members. Folder Invitations create explicit Folder
  access and a Guest relationship when the recipient is not already a Member.
  Existing Members remain Members when they accept a Folder Invitation.
- Removing a Member removes all of that identity's access to the Brain using
  the atomic revocation workflow. It does not preserve or create Guest access.
- Brain Invitations and Folder Invitations are bound to one email or concrete
  Member Identity, single-use, and independently inspectable.
- Invitations expire after seven days by default. Creators may choose a duration
  between one hour and thirty days with `--expires-in`; non-expiring pending
  invitations are unsupported.
- Invitation revoke cancels only a pending offer. After acceptance, Membership
  or Folder access is removed through the corresponding administrative
  operation.
- **Mount Offer** is bound to one source Folder, one destination Brain, and one
  destination owner or admin. It is single-use and follows the same seven-day
  default and one-hour-to-thirty-day expiry bounds.
- Accepting a Mount Offer creates one durable **Shared Folder Connection** and
  one destination **Folder Mount**. The public interface exposes this as one
  Mount resource; the internal store may retain separate connection and
  projection records.
- A Folder Mount is a source-backed reference, never a copy. Reads and writes
  continue to address the native source Folder.
- All four source/destination combinations of Personal and Organization Brains
  are supported.
- An Organization destination initially includes only the accepting admin as a
  Mount Participant. A Personal destination initially includes its owner and,
  when present, its current Personal Agent. Acceptance previews and receipts
  name every initial participant.
- Destination governance may add or remove only identities it governs: owner,
  current Personal Agent, admins, and Members. Destination Guests are
  ineligible.
- Mount Participants are Guests of the source Brain and require a current
  Folder Key Grant for the mounted source Folder.
- The destination controls its participant roster. The source cannot manage
  individual destination participants, but either source or destination
  governance may revoke the entire Mount.
- A mounted Folder cannot be used as the source of another Folder Invitation or
  Mount Offer. Only native source Brain governance may extend access.
- Any native Folder may be invited or mounted. `sharedFolderSource` and its
  preparatory command and route are removed. Native Folder access mode and
  external Guest grants are orthogonal.
- Folder Access Revocation is one atomic, client-owned workflow used by direct
  Folder removal, Member removal, Mount Participant removal, and Mount
  revocation. The trusted client opens the current Folder Key, generates the
  next key, prepares grants for every remaining authorized identity, and
  submits removal, key-version advancement, and replacement grants together.
- Users never provide a raw key-rotation payload. If complete rotation cannot
  be prepared, no access mutation commits and the command returns the exact
  blocker. Success requires authoritative postcondition verification.
- The server remains blind to Folder Keys and grant plaintext. Responses, logs,
  receipts, local state, and diagnostics contain no secrets.
- The normal signed HTTP prefix is `/v1`. `/v1/admin` is reserved for low-level
  member, role, and Folder-access mutations. All former `/_admin` signed routes
  migrate in the same hard cut, including unrelated Brain, content, search, and
  sync resources.
- Read-only access resources are `GET /v1/brains/{brainId}/access` and `GET
  /v1/brains/{brainId}/folders/{folderId}/access`.
- Desired-state collaboration is `POST
  /v1/brains/{brainId}/collaborators/ensure-admin`.
- Brain Invitation collection operations are `GET|POST
  /v1/brains/{brainId}/invitations`; Folder Invitation collection operations
  are `GET|POST
  /v1/brains/{brainId}/folders/{folderId}/invitations`.
- Invitation lifecycle operations are `GET|DELETE
  /v1/invitations/{invitationId}` and `POST
  /v1/invitations/{invitationId}/accept`.
- Mount Offer collection operations are `GET|POST
  /v1/brains/{brainId}/folders/{folderId}/mount-offers`; lifecycle operations
  are `GET|DELETE /v1/mount-offers/{offerId}` and `POST
  /v1/mount-offers/{offerId}/accept`.
- Active Mount operations are `GET /v1/brains/{brainId}/mounts`,
  `GET|DELETE /v1/mounts/{mountId}`, and `PUT|DELETE
  /v1/mounts/{mountId}/participants/{targetNpub}`.
- Low-level administration uses `PUT|DELETE
  /v1/admin/brains/{brainId}/members/{targetNpub}`, `PUT|DELETE
  /v1/admin/brains/{brainId}/roles/admin/{targetNpub}`, and `PUT|DELETE
  /v1/admin/brains/{brainId}/folders/{folderId}/access/{targetNpub}`.
- Share Link, share-source, Shared Folder Invitation, and Shared Folder
  Connection public resources are removed. There are no redirects, aliases, or
  dual-write clients.
- The API hard cut updates Nostr HTTP authorization URLs, Product Client
  callers, Runtime proxies, Smoke UI callers, managed skills, examples, and
  every first-party integration together.
- The no-plumbing precedence contract is: explicit advanced override when
  supplied; saved Working Tree context where applicable; Runtime-provided
  FiniteBrain configuration; canonical binary defaults. Identity continues to
  resolve from the Finite identity contract rather than per-tool signer state.
- Normal hosted commands infer Brain from the current Brain Working Tree and
  Folder from the current managed Folder directory. Ambiguous or absent context
  returns safe choices and the minimum advanced selector needed; it never
  guesses.
- `fbrain open personal` resolves the authoritative visible-Brain list and opens
  the unique Personal Brain at the default Working Tree root. Missing Personal
  Brain returns the existing setup guidance; the command does not silently
  create one.
- Config state defaults below Finite Home and is created securely on demand.
  The absence of a pre-created directory is normal.
- The hosted Runtime supplies canonical production transport and signing
  origins plus durable config and Working Tree roots. The binary has a
  production server fallback, while explicit overrides remain available for
  local and proxy diagnostics.
- `fbrain doctor` performs a signed, read-only `/v1` Brain request through the
  same URL-selection and signing path as ordinary commands. A transport-only
  health response cannot produce an overall healthy result.
- Both bundled copies of the FiniteBrain skill remain identical, teach only
  current Brain vocabulary and canonical commands, and keep plumbing flags in
  an explicitly advanced section.
- PR #246's hidden sidebar, direct internal Brain navigation, hosted defaults,
  and long-note scrolling behavior are carried onto the implementation base.
  Its browser expectation for the intentionally hidden navigation link must be
  reconciled so the Dashboard suite is green.
- The public-interface hard cut does not authorize deletion of durable Brain
  content. A forward schema migration preserves Brains, Folders, objects,
  grants, audit history, accepted relationships, and mount state.
- Migration uses durable provenance to classify relationships. Accepted Brain
  Invitations and explicit broad Organization Membership remain Members;
  accepted one-Folder links and connection participants become Guests.
  Existing Personal Brain non-owner/non-Personal-Agent limited identities
  become Guests with their explicit Folder grants. Ambiguous state fails closed
  without widening access and is surfaced for repair.
- Production migration or mutation requires the repository's normal
  evidence, backup, rollback, and explicit-authorization process; this spec
  itself performs no deployment or production mutation.

## Testing Decisions

- Good tests assert externally observable command behavior, signed HTTP
  contracts, Membership versus Guest access, Folder readability, source-backed
  writes, key-version changes, durable migration results, and secret-free
  receipts. They do not assert private helper order or storage implementation
  when the public behavior is sufficient.
- The primary acceptance seam is one built `fbrain` executable communicating
  with the real signed FiniteBrain server and SQLite store through independent
  Finite Homes and Member Identities. This extends the existing built-process
  collaboration acceptance seam rather than creating a parallel harness.
- The happy-path acceptance starts with Runtime-equivalent defaults and runs
  `doctor`, Brain discovery, `open personal`, sync, collaboration, invitations,
  and context-inferred access operations without `--server`, `--config-dir`,
  generated Brain IDs, explicit Working Tree paths, or redundant Brain/Folder
  selectors.
- A split transport/signing-origin scenario proves that `doctor` fails when a
  real signed command would fail and succeeds when the canonical origins agree.
- The suite covers Brain Invitations and Folder Invitations from both Personal
  and Organization Brains, proving that the former creates Members and the
  latter creates Guests without unrelated all-members access.
- The suite covers Personal-to-Personal, Personal-to-Organization,
  Organization-to-Personal, and Organization-to-Organization Mounts.
- Personal destination acceptance proves owner plus current Personal Agent
  initial participation; Organization destination acceptance proves only the
  accepting admin is initially included.
- Participant tests prove destination-local add/remove authority, Guest
  ineligibility, source inability to micromanage the destination roster, and
  rejection of mounted-Folder resharing.
- Revocation tests prove participant removal, complete Mount revocation by
  either side, current key-version advancement, replacement grants for every
  remaining identity, and no mutation when rotation preparation is incomplete.
- Invitation lifecycle tests cover identity binding, single use, default and
  custom expiry, expiry bounds, pending cancellation, duplicate acceptance, and
  the refusal to treat accepted invitation cancellation as access revocation.
- Administrative tests cover Member removal across every readable Folder,
  explicit admin-role grant/revoke constraints, last-admin protection, direct
  Guest grants, orphan Guest cleanup, and authoritative postcondition receipts.
- API contract tests cover every new `/v1` resource, `/v1/admin` isolation,
  Nostr authorization binding to the new paths, and consistent CLI/API
  observable state.
- Negative hard-cut tests prove retired CLI groups and aliases return unknown
  command and every retired `/_admin`, Share Link, share-source, Shared Folder
  Invitation, and Shared Folder Connection route returns not found without
  mutation.
- Synthetic migration tests begin from representative existing Member, Share
  Link, Personal limited-member, and Shared Folder Connection states. They
  prove content and grants survive, relationship classification does not widen
  access, ambiguous state fails closed, and rollback restores the prior
  synthetic database.
- Product Client tests cover Member and Guest presentation, invitation and
  Mount terminology, participant rosters, locked/incomplete states, and
  revocation receipts without exposing keys.
- The existing headless browser regression for long-note scrolling remains
  green. Dashboard navigation tests assert that Brain is absent from the normal
  sidebar while direct internal navigation remains functional.
- Runtime image contract tests prove canonical server, signing origin, Finite
  Home config, and Working Tree defaults are present.
- Managed-skill static and scenario tests prove canonical singular commands,
  Brain-only terminology, no happy-path plumbing flags, current CLI/API
  references, no raw rotation instructions, and identical bundled skill copies.
- Existing Brain language, Brain product matrix, collaboration smoke, Product
  Client, CLI, store, server, identity, Runtime, Dashboard, and monorepo gates
  remain green.
- Final verification includes a fresh two-Agent local smoke in which one Agent
  creates and fills a Brain, collaborates with the other, exchanges direct
  invitations, establishes a Mount, edits through the mounted Folder, changes
  participants, revokes the relationship, and verifies the resulting access
  from both independent identities.

## Out of Scope

- Broad visual redesign or replacement of the vanilla-JavaScript Product
  Client.
- Enabling Brain in normal dashboard sidebar navigation before that navigation
  is ready.
- Changing Personal Brain ownership, the one-Personal-Agent rule, or Personal
  Agent replacement authority.
- Adding owner transfer or a generic Personal Brain admin role.
- Adding viewer/editor role granularity; Folder access remains binary.
- Reusable, anonymous, public, or non-expiring invitation links.
- Allowing destination Guests to become Mount Participants.
- Allowing mounted Folders to be re-shared or chained to another Brain.
- Copying mounted Folder content into the destination Brain.
- Making the server a Folder Key holder or allowing it to generate, decrypt, or
  rotate Folder Keys.
- Claiming erasure of plaintext, old keys, downloads, backups, or content
  already obtained before revocation.
- Changing the complete, partial, and indeterminate collaboration receipt
  semantics established by the existing Organization Brain Collaboration work.
- Production deployment, production data mutation, or rollout sequencing.

## Further Notes

- This spec builds on PR #172's desired-state Organization Brain collaboration
  and access-truth work while superseding its legacy CLI and `/_admin` public
  surface.
- PR #246 is evidence from the released-version first-contact experience and a
  source of required Runtime-default, skill-language, navigation, and scrolling
  behavior. Because it targets `main`, implementation must deliberately port or
  rebase its relevant changes onto the final #172-derived line rather than
  assuming they are present.
- At the time of synthesis, PR #246's Rust, skill, runtime, and smoke checks
  passed, while its Dashboard check failed because an older browser test still
  expected the intentionally hidden Brain navigation link. That expectation
  must be updated as part of preserving the agreed hidden-navigation behavior.
- ADR-0034 remains authoritative for desired-state Organization Brain
  Collaboration. ADRs 0035 through 0042 record the hard-cut surface,
  Brain-kind-neutral sharing, Member/Guest distinction, Mount roster defaults,
  atomic revocation, removal of share-source state, targeted offers, and bounded
  expiry.
- The experience report that prompted PR #246 should remain a regression input:
  an Agent asking to see its user's Personal Brain should discover, authenticate,
  open, and sync it without reconstructing infrastructure configuration.
