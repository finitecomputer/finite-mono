# Account-Agent Access Cohorts And Multi-Agent Personal Brains

## Problem Statement

FiniteBrain currently authorizes one Nostr principal at a time. A Brain or
Folder invitation addressed to a human-facing email ultimately adds only that
human's User Nostr Identity, while every hosted agent has a separate Agent
Principal Key. A user who asks an agent to accept an invitation therefore
encounters a cryptographic mismatch: the agent can sign only as itself, cannot
use the human's key, and does not automatically receive the human's Folder Key
Grants.

That behavior contradicts the product's primary interaction model. Users will
normally operate Brain through agents, may own several agents, and expect any
trusted account agent to work with the knowledge they can access. Requiring a
human to visit a client, copy raw keys, pair agents one at a time, or repeat
every grant manually makes the human UI and agent UX disagree.

The current singular Personal Agent model creates the same problem inside the
user's own Personal Brain. Only one selected agent can work there, a later
agent cannot join after bootstrap, and replacement is modeled as a slot swap.
That prevents a user with several trusted account agents from using the same
Personal Brain through whichever agent they are currently talking to.

Existing internal-beta Brains also contain human-only memberships and Folder
access created under the old semantics. Simply inserting agent membership rows
would be unsafe and misleading because readable access requires a valid
encrypted Folder Key Grant for every agent. A coordinated change must preserve
distinct principals, client-owned content encryption, user-data availability,
recoverability, truthful partial state, and old-client read and sync behavior.

## Solution

Treat a Finite user's email as the human-facing address for an **Account Access
Cohort**: the human User Nostr Identity plus the account-owned Agent Principals
authorized for the same Brain or Folder scope. The email does not become a
shared signer. Every participant keeps its own key, membership, grants,
revocation, and audit identity.

For invitations and Organization Brain collaboration, preflight resolves the
human and a snapshot of the current eligible **Account Agent Set**. It shows the
inviter exactly which friendly agent identities will receive access. If an
agent is not grant-ready, the operation pauses before sending and offers one
minimal, explicit reduced-set confirmation. The approved participant set is
fixed when the invitation is sent. An included agent may later accept the one
account-level invitation after the human asks it to do so, signing as itself and
without a human key or manual client step.

Send one email to the human and expose one shared **Account Invitation Inbox**
record to the human client and included agents. Do not copy the invitation or
wake every agent. Render one concise result such as `Invited paul@finite.vip and
2 of his agents: Waffle and Biscuit.`

Personal Brains follow a different lifecycle because they belong to the human:
their **Personal Brain Agent Set** tracks the owner's live Account Agent Set.
Every eligible current and future account agent receives full operational
Personal Brain access while the human remains sole owner. Brain setup is a
separate capability and never blocks agent launch, chat, or unrelated work.

Agents exercise routine administration through durable **Human-Anchored Agent
Authority**, not copied independent admin roles. Standing authority covers
normal work. Changing another account agent's Personal Brain access requires
one-use **Authenticated Human Intent**, while ownership transfer,
Recovery Set changes, and whole-Brain deletion remain directly human-operated.

Quietly reconcile the existing internal-beta population into Account Access
Cohorts. Reconciliation is key-aware and atomic per Brain, or per Folder for a
Folder-only Guest. It creates no invitations or emails and never reports
completion until every added principal has the required current Folder Key
Grants.

## User Stories

1. As a user, I want to address Brain identities by email, so that I never need
   to understand raw Nostr public keys.
2. As an inviter, I want `paul@finite.vip` to mean Paul and the agents trusted
   by Paul's account, so that Paul can use the shared knowledge through his
   normal agent UX.
3. As an inviter, I want Paul and his agents to remain separate principals, so
   that sharing does not create a shared private key.
4. As an inviter, I want to see the friendly names of Paul's included agents,
   so that I know who will receive encrypted data.
5. As an inviter, I want raw npubs hidden from normal confirmation copy, so that
   cryptographic identifiers remain an advanced diagnostic.
6. As an inviter, I want Brain to preflight every participant before sending,
   so that an invitation does not promise access it cannot deliver.
7. As an inviter, I want stopped or temporarily unhealthy agents to remain
   eligible, so that compute health does not rewrite durable identity access.
8. As an inviter, I want agents still being created or with failed identity
   provisioning excluded from eligibility, so that incomplete identities do
   not receive misleading membership.
9. As an inviter, I want agents merely shared with Paul excluded, so that only
   account-owned agents inherit Paul's access.
10. As an inviter, I want a clear warning when one agent cannot receive access,
    so that the operation does not fail without explanation.
11. As an inviter, I want to approve a reduced participant set explicitly, so
    that one broken agent does not necessarily block sharing with everyone.
12. As an inviter, I want no invitation created before I approve an exclusion,
    so that partial access is never accidental.
13. As an inviter, I want the approved participant set frozen at send time, so
    that later account changes cannot expand my authorization silently.
14. As an inviter, I want agents created after sending excluded from that
    invitation, so that a consumed offer does not become an evergreen grant.
15. As an invitee, I want an agent that became permanently ineligible before
    acceptance removable through explicit narrowing, so that the remaining
    participants can still accept.
16. As an invitee, I want acceptance narrowing limited to removing authority,
    so that acceptance can never add an unapproved principal.
17. As an invitee, I want one email notification, so that my inbox is not
    flooded for every agent I own.
18. As an invitee, I want the email to say that my agents are included, so that
    the access scope is not surprising.
19. As an invitee, I want agent names available in Finite rather than listed in
    the email, so that the email remains concise.
20. As a user, I want one Account Invitation Inbox shared by my client and
    included agents, so that every interface sees the same pending offer.
21. As a user, I want the shared inbox to contain one invitation rather than
    one copy per agent, so that acceptance and cancellation cannot drift.
22. As a user, I want any included agent to check my invitation inbox, so that I
    can ask whichever agent I am using about pending access.
23. As a user, I want an agent to summarize an invitation before acting, so
    that I understand the Brain or Folder and included agent count.
24. As a user, I want my agent to accept after I ask, so that I do not need to
    open a separate client.
25. As a user, I want the accepting agent to sign as itself, so that it never
    needs or impersonates my human key.
26. As a security reviewer, I want Brain to verify the accepting agent's
    current account relationship, so that unrelated agents cannot accept.
27. As a user, I want invitations never accepted merely because they arrive,
    so that delivery is not consent.
28. As a user, I want hiding an invitation to be reversible, so that cleaning
    my inbox does not reject access for my entire account.
29. As a user, I want no permanent recipient-decline state, so that an ignored
    invitation may remain available until expiry or sender revocation.
30. As an inviter, I want one minimal participant-aware result, so that I know
    what happened without reading an identity dump.
31. As an agent, I want structured participant facts from the service, so that
    I can render correct language without inferring account relationships.
32. As a CLI user, I want the same concise result as the agent UX, so that
    surfaces do not disagree.
33. As a Brain invitee, I want Paul and his included agents to become Members,
    so that each can use current and future all-members Folders.
34. As a Folder invitee, I want Paul and his included agents to become Guests
    of only that Folder, so that unrelated Brain content remains private.
35. As a Member, I want restricted Folder grants addressed to my email to
    include my cohort agents, so that later access remains agent-usable.
36. As an administrator, I want email-addressed Folder revocation to remove the
    human and cohort-derived agent access together, so that revocation matches
    granting semantics.
37. As an administrator, I want revocation to rotate the affected Folder Key,
    so that removed agents cannot decrypt future content.
38. As an administrator, I want to exclude one cohort agent from one sensitive
    Folder, so that default cohort access still supports explicit exceptions.
39. As an administrator, I want a scoped exclusion to remain durable, so that
    reconciliation cannot silently restore it.
40. As an administrator, I want to remove one agent without removing the human
    or other agents, so that a compromised principal can be isolated.
41. As an administrator, I want targeted agent removal to revoke all of that
    agent's access in the selected scope, so that an independent path does not
    make security removal misleading.
42. As an administrator, I want an excluded or repaired agent added later by
    explicit action, so that recovery is convenient but never automatic.
43. As an administrator, I want adding a later agent to reuse the human's
    existing cohort scope, so that the human need not accept another invitation.
44. As an administrator, I want the human and agents added through one email
    removable as a cohort, so that lifecycle follows the original access intent.
45. As an administrator, I want independently authorized agent access to
    survive removal of the human cohort, so that unrelated grants retain their
    provenance.
46. As an ordinary Brain Member, I want my agents to receive content access
    without becoming independent admins, so that their authority remains tied
    to mine.
47. As a human admin, I want my agents to perform routine admin work for me, so
    that Brain administration remains agent-first.
48. As a human admin, I want my agents' delegated admin power removed when I am
    demoted, so that they cannot outlive my role.
49. As an auditor, I want every delegated action to identify both the acting
    agent and anchoring human, so that attribution remains truthful.
50. As a user, I want normal agent operations to use standing authority, so
    that I do not click or sign for every Brain action.
51. As a user, I want ownership transfer, Recovery Set changes, and whole-Brain
    deletion to remain directly human-operated, so that agents cannot perform
    sovereign actions.
52. As a user, I want to tell one agent to restrict or restore another agent's
    Personal Brain access, so that agent-first UX includes agent management.
53. As a security reviewer, I want peer-agent access changes to consume a
    fresh authenticated human-turn capability, so that background work cannot
    remove peers and one human turn cannot authorize multiple changes.
54. As a user, I want no extra Product Client click for Authenticated Human
    Intent, so that the authenticated conversation remains the interface.
55. As a Personal Brain owner, I want every eligible account agent to use my
    Personal Brain, so that I can work through whichever agent I am talking to.
56. As a Personal Brain owner, I want to remain the sole owner, so that
    multi-agent operation does not distribute ownership.
57. As a Personal Brain owner, I want every ready agent to access every current
    and future Folder, so that Personal Brain behavior is consistent.
58. As a Personal Brain owner, I want new eligible account agents admitted
    automatically, so that I do not pair them one at a time.
59. As a Personal Brain owner, I want a permanently departed agent revoked
    automatically, so that disconnected agents do not retain future access.
60. As a Personal Brain owner, I want temporary agent stops and restarts to
    preserve access, so that infrastructure lifecycle does not create churn.
61. As a new agent, I want Brain connection to run separately from launch, so
    that I can chat and do unrelated work immediately.
62. As a new agent, I want Brain to retry connection in the background and on
    demand, so that transient key-grant setup recovers without manual pairing.
63. As a new agent, I want a minimal connecting message when asked to use Brain
    too early, so that I do not claim missing access or block the conversation.
64. As a user, I want Brain readiness reported separately from overall Agent
    Readiness, so that one product dependency does not make the whole agent look
    broken.
65. As a user, I want Personal Brain readiness complete only when the agent can
    open every Folder, so that partial access is not presented as success.
66. As a Personal Brain owner, I want existing ready agents and content to
    remain available while a new agent connects, so that admission does not
    interrupt my Brain.
67. As a human creating an Organization Brain, I want all my current eligible
    agents included at creation, so that the Brain is agent-usable immediately.
68. As a human asking an agent to create an Organization Brain, I want the same
    human-plus-current-agents result, so that creation paths converge.
69. As an Organization Brain creator, I want myself to be the human admin and
    my agents to act through my authority, so that agents do not become
    independent admins.
70. As an Organization Brain creator, I want later agents added explicitly, so
    that collaborative access remains a fixed approved snapshot.
71. As an internal-beta user, I want my existing Brain access upgraded to the
    new cohort model, so that old test data behaves like newly created access.
72. As an internal-beta user, I want retroactive upgrade to send no invitations
    or emails, so that migration does not look like new sharing activity.
73. As an internal-beta user, I want migration to preserve my existing access
    until complete agent grants can be created, so that upgrade cannot strand
    me.
74. As an operator, I want reconciliation atomic per Brain, so that one Brain
    is internally consistent while other Brains may retry independently.
75. As an operator, I want Folder-only Guest access reconciled atomically for
    that Folder, so that narrow shares remain narrow.
76. As an operator, I want migration failures reported as retryable blockers,
    so that no metadata-only backfill masquerades as access.
77. As an operator, I want a dry-run inventory before mutation, so that agent,
    cohort, capacity, and key-grant blockers are known.
78. As an operator, I want a backup and rollback boundary before reconciliation,
    so that internal-beta mutation remains recoverable.
79. As an old-client user, I want reading, syncing, and chat to continue after
    cutover, so that the access redesign does not break data availability.
80. As a maintainer, I want old clients blocked from writing legacy human-only
    access, so that mixed versions cannot recreate the gap.
81. As an old-client user, I want an explicit update-required error on access
    writes, so that refusal is actionable rather than mysterious.
82. As a user, I want routine Brain work to continue during a Core outage, so
    that an account-service dependency does not disable my agent UX.
83. As a security reviewer, I want new admissions and relationship changes to
    use fresh Core and Identity facts, so that durable authority is not created
    from guesses.
84. As a security reviewer, I want known permanent departures applied
    immediately, so that availability caching does not ignore revocation.
85. As a recovery operator, I want cohorts, authority, exclusions, revocations,
    and grants restored together, so that an empty-target restore preserves
    both access and security.
86. As a privacy-conscious user, I want the Brain server to remain blind to
    Folder Keys, so that multi-agent access does not weaken content encryption.
87. As a capacity-limited admin, I want preflight to report when the full cohort
    exceeds Brain limits, so that Finite never chooses agents silently.
88. As an internal migration operator, I want capacity-blocked Brains left
    unchanged, so that quiet reconciliation never invents exclusions.
89. As a tester, I want the flow proved with independent human and agent keys,
    so that a shared test signer cannot hide identity errors.
90. As a tester, I want readable access proved by decryption, not membership
    rows, so that tests enforce Folder Access Readiness.
91. As an inviter, I want distinct Brain and Folder invitations to the same
    human email to coexist, so that one collaboration scope does not block
    another.
92. As an inviter, I want an exact retry for the same email, resource, scope,
    and plan to return the existing result, so that retry safety does not create
    duplicate invitations or duplicate email delivery.
93. As an existing invitee, I want a pending Finite VIP mailbox invitation
    preserved through cutover, so that synchronization does not silently cancel
    an offer I have not accepted yet.
94. As an inviter, I want an explicit raw-npub invitation to remain
    exact-principal access, so that account-cohort migration does not broaden an
    intentionally narrow grant.
95. As a security reviewer, I want a legacy mailbox invitation blocked from
    human-only acceptance until its cohort grants are ready, so that cutover
    cannot recreate the access gap through an old pending record.

## Implementation Decisions

- A Finite VIP Mailbox Address is the normal human-facing email for invitation,
  access, and cohort operations. Raw principal identifiers remain supported
  only as an advanced diagnostic or explicit principal-targeting path.
- Core owns the authoritative Account Agent Roster and Permanent Agent Departure
  Facts. The roster includes successfully identity-provisioned account-owned
  agents regardless of temporary runtime health and excludes incomplete,
  failed, unlinked, retired, deleted, and merely shared agents.
- Core adds a Brain-facing account-to-agents contract. It returns stable account
  and human-mailbox identity, each eligible Managed Agent NIP-05, agent
  principal binding reference, lifecycle state, and a monotonic roster
  revision. It does not return private keys or grant Brain authority.
- Core exposes a durable, retryable way for Brain to learn Permanent Agent
  Departure Facts. Delivery may be push or pull, but it must be replayable and
  monotonic; a best-effort webhook is insufficient.
- Finite Identity owns Participant Principal Resolution. It resolves the human
  Finite VIP Mailbox Address and Core-enumerated Managed Agent NIP-05 Names into
  separate public User and Agent principals. A Managed Agent NIP-05 is a
  readable identity name, not a deliverable mailbox. Identity does not
  enumerate ownership or create Brain permissions.
- After cohort-write cutover, an email-shaped Finite VIP Mailbox Address always
  uses the account-cohort path even when NIP-05 resolution can immediately find
  the human npub. The current mailbox-to-single-npub optimization is a legacy
  writer path. Exact-principal access requires an explicit raw npub target;
  external and unresolved mailbox bootstrap retains its bounded claim flow.
- Brain owns Account Access Cohorts, Personal Brain Agent Sets, Human-Anchored
  Agent Authority, invitations, Membership, Folder Access, Folder Key Grants,
  exclusions, admissions, revocations, audit, readiness, and reconciliation.
- Finite Chat supplies Authenticated Human Intent for sensitive agent-operated
  changes. The one-use capability is bound to the human, acting agent,
  freshness, and replay protection; Brain records the exact route-derived
  action that consumes it. The personal-agent trust model assigns semantic
  translation of the human's words to the agent, and agent-supplied prose is
  not independently verified authority.
- Finite Skills renders structured Brain outcomes and teaches the agent-first
  flow. It never enumerates account agents itself, infers ownership, parses
  human-facing prose for principals, or claims success beyond the service
  receipt.
- An Account Access Cohort stores its anchoring human identity and email,
  resource scope, provenance, approved agents, independent participant status,
  exclusions, roster revision, and audit history. It is not an authorization
  subject and never signs.
- Cohort provenance distinguishes invitation, Organization Brain bootstrap,
  later email-addressed grant, explicit admission, and internal-beta
  reconciliation. Independent authorization paths remain independently
  revocable.
- Invitation Preflight and commit form one idempotent plan/commit workflow.
  The plan binds target email, Brain or Folder scope, participant principals,
  display identities, readiness, exclusions, grant key versions, capacity,
  expiry, roster revision, and actor. Approval commits that exact plan. If
  authoritative facts change, commit returns a new preflight result instead of
  silently changing the participant set.
- Pending invitation uniqueness includes invitation kind and exact resource
  scope. One Brain invitation and one or more distinct Folder invitations to
  the same Finite VIP Mailbox Address may coexist. An exact same-scope retry
  returns the prior immutable plan, invitation, and delivery receipt rather
  than excluding its reservation to construct a second plan or sending another
  email. Changing that scope's participant exclusions requires revoking the
  pending invitation first.
- A fully ready preflight may commit immediately as the normal one-command happy
  path. Only blockers requiring a reduced set add a confirmation turn.
- Invitation creation persists the fixed Invitation Participant Set and every
  encrypted current Folder Key Grant required for its approved scope. The
  trusted inviter client opens current keys and wraps one grant per participant;
  the server validates envelopes and remains blind to plaintext keys.
- Brain Invitation acceptance atomically consumes the pending invitation,
  enrolls every approved participant as a Member, installs the prepared access
  and grants, creates Account Access Cohort provenance, and records the acting
  human or agent. Folder Invitation acceptance performs the same transition for
  bounded Guest access to one Folder.
- Acceptance revalidates permanent eligibility. If a participant became
  permanently ineligible, Brain returns an explicit narrowing proposal. An
  approved narrowing removes only that participant and then commits the
  remaining set atomically.
- One account-level pending-invitation query is authorized to the human User
  Nostr Identity and approved included Agent Principals. It returns the same
  invitation identifier and state to every viewer.
- Account Invitation Inbox dismissal is reversible account view state. It does
  not mutate invitation status, notify the inviter, consume the offer, or block
  later acceptance before expiry or revocation.
- Email delivery occurs once per committed invitation to the human mailbox,
  never once per agent. The email names the inviter and Brain or Folder and
  states the included agent count. Distinct Brain and Folder invitations each
  receive their own notice; exact retries do not redeliver. Full participant
  details remain in the authenticated inbox.
- Invitation and access results contain a machine-readable participant array
  with relationship, friendly identity, readiness, inclusion, exclusion reason,
  and resulting scope. The stable non-JSON CLI output is one concise summary;
  raw npubs do not appear in ordinary copy.
- Account agents enrolled in an Organization or invited cohort are distinct
  Members or Guests. They do not receive copied admin roles. Durable
  Human-Anchored Agent Authority references the anchoring human and current
  Brain Role for routine delegated authorization.
- Routine delegated authorization uses Brain's durable authority record and
  does not call Core on every request. New admission, restoration, ownership
  change, and other relationship mutations perform fresh authoritative
  resolution.
- Demoting the human immediately disables corresponding delegated admin powers.
  Removing the human revokes cohort-derived agent access in the same scope.
  Independent agent grants remain unless the operation explicitly targets the
  agent.
- Targeted Cohort Participant Revocation removes all access paths held by that
  agent in the selected Brain or Folder scope, rotates every affected Folder
  Key, and writes a durable exclusion. It never silently returns through later
  reconciliation.
- Scoped Cohort Exclusion applies to one restricted Folder and leaves other
  Brain access unchanged. Restoring it is a fresh explicit authorization and
  grant operation.
- Cohort Participant Admission performs fresh Core and Identity verification,
  verifies capacity, prepares current grants, and atomically adds the agent to
  the existing cohort scope. It does not require the human to accept another
  invitation.
- Personal Brain ownership remains solely on the User Nostr Identity. The old
  singular Personal Agent slot and replace/vacate workflow are removed.
- A Personal Brain stores the desired Personal Brain Agent Set derived from the
  owner's live Account Agent Roster. Every eligible agent has the same full
  operational content and collaboration authority but no owner or admin role.
- Personal Brain bootstrap creates one empty human-owned Brain and the desired
  set for every currently eligible account agent. User-first and agent-first
  bootstrap converge on the same state.
- A newly eligible account agent enters the Personal Brain desired set
  automatically. Every Agent Runtime supervisor asks each already-open ready
  Personal Brain to reconcile at startup and every five minutes; a ready owner
  or agent wraps the missing current Folder grants. Its Brain capability remains `setting_up` until durable
  authority and current grants for every Personal Brain Folder are complete.
  It may be `blocked` with an actionable reason or `ready`; these states do not
  replace overall agent lifecycle or Chat readiness.
- Adding a Personal Brain Agent does not rotate current Folder Keys. A trusted
  ready owner or agent client opens current keys and wraps them for the new
  principal. Brain work remains unavailable to that agent until every current
  Folder grant is complete, but existing principals remain available.
- Every newly created Personal Brain Folder includes current grants for the
  owner and every ready Personal Brain Agent. A desired-set agent still setting
  up is reconciled before Brain reports that agent ready.
- Permanent account departure ends Human-Anchored Agent Authority and Personal
  Brain Agent Access, removes cohort-derived relationships, rotates every
  affected current Folder Key, and preserves the human Brain and content.
  Temporary runtime lifecycle never triggers this flow. The agent supervisor
  polls `GET /v1/brains/{brainId}/permanent-agent-departures` before admission
  reconciliation and applies each unapplied fact through the existing exact
  preflight/commit boundary.
- Peer-agent removal, restriction, or restoration inside a Personal Brain is
  agent-operable only with Authenticated Human Intent. Normal content and
  sharing work uses standing authority. Ownership transfer, Recovery Set
  changes, and whole-Brain deletion remain directly human-operated.
- Organization Brain bootstrap creates one human Member/Admin and an Account
  Access Cohort containing a snapshot of all current eligible account agents as
  Members. Agents use Human-Anchored Agent Authority rather than independent
  admin rows. Direct-human and agent-created paths converge.
- Organization Brain agents created after bootstrap require explicit Cohort
  Participant Admission. They do not follow the live Personal Brain rule.
- Existing internal-beta state is reconciled quietly. Reconciliation creates no
  invitation, sends no email, and emits only minimal activity or operator
  receipts.
- `POST /v1/brains/{brainId}/cohort-reconciliation/preflight` is read-only and
  returns a stable exact plan including participant principals, scope, current
  and missing current grants, key versions, capacity, independent Agent access,
  and matching pending invitations. `PUT` to the collection requires that exact
  plan, prepared grants, a signed access-change event, and `backupReference`;
  commit is atomic and exact retry returns the prior receipt.
- Pending legacy Finite VIP invitation conversion uses
  `POST|PUT /v1/brains/{brainId}/invitations/{invitationId}/cohort-conversion[/preflight]`.
  Conversion requires a declared backup, preserves the delivery receipt, and
  never invokes the mailer. Until conversion, acceptance returns HTTP 426 with
  update guidance.
- Peer-agent Personal Brain restriction and restoration accept
  `authenticatedHumanIntent` only on exact-npub `DELETE|PUT` Folder access and
  whole-Brain Personal Agent access routes. Finite Chat mints a short-lived,
  server-verifiable requester assertion for the authenticated human and acting
  agent. Brain combines that proof with the exact route-derived target, scope,
  and operation, persists the canonical composite, and consumes the requester
  assertion id once regardless of which action is attempted. Neither the CLI
  nor the agent supplies an authoritative action binding or handles the human's
  private key. Product Client invitation flows require no extra click.
- The reconciliation inventory includes pending legacy invitations. A pending
  invitation to a resolvable Finite VIP Mailbox Address keeps its identifier,
  expiry, resource kind, and exact Brain or Folder scope but gains a fixed
  participant set and prepared grants without sending another email. Until that
  conversion commits, cohort-aware acceptance fails explicitly and leaves the
  invitation pending; it never falls back to human-only membership.
- A pending explicit-npub invitation remains exact-principal access. Existing
  external or unresolved mailbox bootstrap invitations retain their current
  single-principal claim flow unless and until they resolve to a Finite account;
  they are not silently expanded from email spelling alone.
- Reconciliation first produces a read-only inventory and plan. It validates
  identity resolution, agent eligibility, capacity, current grant coverage,
  trusted-client key availability, and the exact target state before mutation.
- Legacy Organization agent membership is not blanket-classified as
  independent during schema migration. Reconciliation records an independent
  legacy source only when its reviewed plan found existing direct Folder or
  grant evidence; otherwise bootstrap/cohort provenance remains authoritative.
- The reconciliation atomic unit for a Member is one Brain plus every Folder
  that human can access there. A Folder-only Guest is one Folder unit. Separate
  Brains may complete or retry independently.
- A unit commits only when every cohort relationship, authority record, current
  encrypted grant, audit record, and sync record can commit together. Failure
  leaves the previous access state unchanged and retryable.
- Internal-beta reconciliation requires a documented backup and rollback
  boundary and proof against synthetic existing state before any deployed data
  mutation. This spec authorizes implementation, not an unreviewed production
  mutation.
- Cohort Access Cutover is coordinated across Core, Identity, Brain, Chat,
  Product Client, CLI, Runtime skill delivery, and reconciliation tooling.
- `GET /health` advertises `account_cohort_writes_v1` in `capabilities`. After cutover, old clients may
  continue compatible reads, sync, and chat but receive update-required for
  invitation, membership, Folder access, Personal Agent, and collaboration
  writes that could create legacy state.
- Concretely, the removed singular `PUT /v1/brains/{brainId}/personal-agent`
  writer and email-shaped Finite VIP exact-principal Member or Folder grant
  writers return HTTP 426 with cohort-flow guidance. Finite VIP invitation
  creation without a current cohort preflight does the same. Explicit raw-npub
  writes remain available for intentionally narrow access, including targeted
  removal of one cohort agent with a durable exclusion.
- Exact-npub Member removal first calls
  `POST /v1/admin/brains/{brainId}/members/{targetNpub}/removal-preflight`.
  The response's `removedParticipantNpubs` is the authoritative rotation set;
  clients must not infer it from the metadata projection because independent
  membership, an explicit admin role, and overlapping cohorts can retain
  individual participants. The anchoring human must still have their own admin
  role revoked before removal.
  `folderAccessRemovals` additionally names retained Members whose
  cohort-derived access ends in each Folder, so clients rotate exactly the
  affected Folder recipients without treating those identities as removed
  Members.
- Capacity is evaluated over the full resulting Member, Guest/Folder Access,
  Folder Key Grant, sync-record, and pending-invitation state, including the
  exact distinct resources reserved by every other unexpired pending cohort
  plan. Expired plans reserve nothing, overlaps are counted once, and commit
  rechecks the complete envelope inside the serialized write transaction.
  SQLite capacity guards make every competing Member, Folder Access, Folder
  Key Grant, and sync-record writer honor those reservations until expiry or
  terminal invitation state.
  Interactive preflight may offer
  explicit account-agent exclusions (the CLI uses `--exclude-agent` with
  `--approve-reduced`). Those exclusions are supplied to preflight, validated
  as eligible account agents or echoed as already-required readiness
  exclusions, removed from the returned participant set, and
  hashed into the immutable plan id; commit must echo the exact returned
  exclusion set. Quiet reconciliation never chooses an exclusion and
  leaves a capacity-blocked unit unchanged. Account-agent fanout is bounded at
  64 principals per account. Brain supplies the at-most-64 active managed
  agent NIP-05s when polling Core; Core filters for those principals before
  returning at most 256 facts. The empty-filter compatibility request returns
  the newest 256 facts. Brain then filters already-applied facts locally, so
  unrelated later departures cannot starve an older Brain-relevant fact and
  no lifetime fact-count cliff exists.
- Permanent-departure polling chunks each account's active managed-agent set
  into at-most-64-principal Core requests; a Brain may contain more than 64
  accounts or stale cohort participants without disabling known revocation.
- Every accepted administrative control performed through Human-Anchored Agent Authority
  writes an audit record in the same transaction, naming both the acting agent
  and anchoring human alongside the signed record id and type.
- Durable cohort, authority, readiness, exclusion, admission, departure,
  dismissal, and reconciliation records participate in sync, backup, export
  where appropriate, and empty-target recovery. Historical audit remains even
  after active relationships end.
- Server-side logic may validate signed evidence, identity bindings, policy,
  envelope recipients, key versions, and hashes, but never opens, generates,
  derives, logs, or returns a plaintext Folder Key.
- All state-changing commands are idempotent for an exact operation identifier
  and immutable plan. A retry returns the authoritative prior result; a changed
  plan is a new operation.

## Testing Decisions

- Good tests prove externally visible authorization, account relationship,
  participant copy, invitation lifecycle, Folder readability, key-version
  changes, audit attribution, migration state, and recovery. They do not assert
  helper order, private function names, or storage layout when public behavior
  is sufficient.
- The primary acceptance seam is the existing built-process `fbrain` harness:
  one real signed Brain server and store, public Core and Identity contract
  doubles, and independent Finite Homes for the inviter, human invitee, and
  multiple account agents. This is the highest existing seam that can prove
  distinct keys and real encryption without creating a parallel harness.
- The main process story invites one existing Finite VIP Mailbox Address with
  two ready agents, proves the concise participant-aware result and one pending
  inbox record, accepts through one agent, and proves the human and both agents
  can independently sync and decrypt the intended Brain or Folder.
- The same story proves a raw human key is never supplied to an agent, every
  signed action identifies the actual agent, and the server never returns a
  plaintext Folder Key.
- Invitation process cases cover Brain Member and Folder Guest outcomes,
  all-members and restricted Folder scope, one email notification, shared inbox
  visibility, reversible dismissal, single use, expiry, inviter revocation, and
  exact retry. They also prove distinct Brain and Folder invitations to the same
  mailbox coexist while exact same-scope retries neither duplicate records nor
  redeliver email.
- Preflight cases cover stopped agents, incomplete identity provisioning,
  failed agents, merely shared agents, capacity failure, one blocked agent,
  explicit reduced-set approval, stale plan revalidation, and no mutation before
  approval.
- Acceptance narrowing cases permanently depart one approved agent after send,
  prove that acceptance cannot expand the set, require explicit narrowing, and
  prove the remaining set commits atomically.
- Cohort lifecycle cases cover human demotion, human removal, targeted agent
  revocation, scoped Folder exclusion, explicit restoration, later admission,
  independent grants, and every required Folder Key rotation.
- Authority cases prove routine agent operation works from durable Brain state
  during a Core outage, while new admissions fail closed without fresh facts and
  a replayed Permanent Agent Departure Fact revokes known authority.
- Authenticated Human Intent cases prove a human can ask one agent to restrict
  or restore another, Brain stores the exact server-derived action, the
  human-turn assertion rejects replay or rebinding to a second action, and
  background execution without a fresh authenticated turn cannot satisfy the
  requirement.
- Personal Brain process cases start with multiple current account agents,
  prove every ready agent can discover, sync, read, write, share, and delete
  content, and prove the human remains sole owner.
- New-agent Personal Brain cases add an agent after Brain creation, prove agent
  launch and chat readiness remain successful while Brain is `setting_up`,
  prove minimal on-demand connecting behavior, complete background grant
  reconciliation, and then prove independent decryption of every Folder.
- Personal Brain departure cases distinguish temporary stop, restart,
  relocation, transient health failure, permanent unlink, retirement, and
  deletion. Only permanent departure revokes and rotates.
- Organization Brain bootstrap cases prove both direct-human and agent-created
  paths yield one human admin plus the same current-agent Member cohort, no
  independent agent admins, no seeded Folder, and no partial Brain on failure.
- Core contract tests cover authoritative account-to-agent enumeration,
  inclusion independent of runtime health, exclusion of incomplete and departed
  agents, monotonic roster revisions, ownership isolation, and replayable
  Permanent Agent Departure Facts.
- Identity contract tests cover Participant Principal Resolution for one human
  Finite VIP Mailbox Address and several Managed Agent NIP-05 Names, missing or
  conflicting bindings, mailbox-versus-name separation, distinct principals,
  no private-key material, and no inferred account ownership.
- Brain signed HTTP tests cover idempotent preflight/commit, participant-set
  validation, envelope recipients, cohort provenance, authority evaluation,
  inbox authorization, structured receipts, mailbox targets that already
  resolve to a human npub, update-required legacy writes, and atomic rollback.
- Store-level transactional tests support the higher seam for exact rollback,
  uniqueness, capacity, sync projection, audit retention, and recovery replay.
  They do not replace process acceptance.
- Migration tests begin from synthetic copies of every relevant old shape:
  singular Personal Agent, human-only Brain Member, restricted Folder access,
  Folder-only Guest, Organization human/agent bootstrap, accepted invitation,
  pending Finite VIP mailbox invitation, pending explicit-npub invitation,
  independent direct agent grant, and scoped mount access.
- Migration tests prove dry-run counts, per-Brain atomicity, Folder-only unit
  atomicity, no invitations or emails, no automatic exclusions, unchanged state
  on missing keys or capacity, exact retry, preservation of distinct pending
  scopes, cohort-aware acceptance refusal before pending-invitation conversion,
  exact-principal preservation, and rollback from the declared backup boundary.
- Mixed-version tests prove old clients can list, open, sync, and chat after
  cutover but cannot create or mutate invitations, cohorts, members, Folder
  access, or singular Personal Agent state.
- Recovery tests restore onto an empty target and prove human and agent access,
  exclusions, departure revocations, independent provenance, inbox state,
  current grants, and audit attribution match the source state.
- Product Client tests cover email-first participant preview, friendly agent
  roster, explicit exclusions, shared inbox, hidden invitation restoration,
  Personal Brain Agent readiness, scoped exclusions, and absence of raw npubs
  in normal copy.
- CLI tests cover stable JSON participant facts and minimal non-JSON summaries
  for success, exclusion, pending Brain connection, migration completion, and
  update-required.
- Managed-skill scenario tests cover invite-by-email, minimal preflight
  confirmation, participant-aware completion, inbox lookup, agent-assisted
  acceptance, connecting-state explanation, normal standing authority, and
  Authenticated Human Intent for peer-agent access changes.
- Static skill delivery checks prove the canonical and packaged FiniteBrain
  skill copies remain synchronized and do not reconstruct cohort facts.
- The release gate adds one full-stack `just dev smoke` story across real Core,
  Finite Identity, Brain, authenticated Chat requester context, and at least two
  Agent Runtimes. It proves invite-by-email, acceptance through an agent,
  independent content access, and one permanent departure or scoped revocation.
- Final verification runs focused component tests, root formatting and linting,
  workspace tests against real Postgres where required, dashboard tests and
  build, Finite Chat bridge tests, skill static checks, the process acceptance
  suite, and the complete local integration smoke.

## Out of Scope

- Sharing a human private key with an agent or representing an agent signature
  as human-signed.
- Cryptographically proving that the human's natural-language words
  semantically requested the agent-selected action. The personal agent is a
  trusted delegate for that translation; deployments that treat the agent as
  adversarial must require a structured human confirmation surface instead.
- Turning an email, Account Access Cohort, or Account Agent Set into a signing
  principal.
- Granting independent owner or admin roles to every account agent.
- Allowing agents to transfer Brain ownership, change the Recovery Set, or
  delete the whole Brain.
- Automatically adding later-created agents to fixed invitation or Organization
  Brain cohorts. Personal Brains are the deliberate live-set exception.
- Automatically restoring a deliberately excluded or revoked agent.
- Automatically choosing which agents to omit during capacity or readiness
  failures.
- Blocking Agent Runtime launch, Chat readiness, or unrelated agent work on
  Personal Brain Agent Readiness.
- Proactively messaging or waking every included agent when an invitation
  arrives.
- Adding a permanent recipient-declined invitation state.
- Reusable, anonymous, public, multi-email, or non-expiring invitations.
- Changing the existing external or unregistered-email bootstrap beyond the
  account-cohort behavior for identities that already resolve to a Finite
  account.
- Giving Finite Identity ownership of account enumeration or giving Core
  product-level Brain permissions.
- Making the Brain server a Folder Key holder or using Recovery Authority as a
  routine migration shortcut.
- Claiming revocation can erase plaintext or historical keys already retained
  by a formerly authorized client.
- Production data mutation without the repository's required evidence, backup,
  rollback, and explicit authorization process.
- Broad Brain Product Client redesign unrelated to cohort preview, inbox,
  readiness, and access-management surfaces.

## Further Notes

- Published implementation issue:
  [finitecomputer/finite-mono#441](https://github.com/finitecomputer/finite-mono/issues/441).
- ADR-0045 is the governing decision. It supersedes the one-Personal-Agent and
  selected-single-agent bootstrap consequences of ADRs 0023 through 0026 and
  amends targeted invitation, deletion, and mount participant rules.
- Existing invitation expiry, single-use delivery, Member/Guest distinction,
  Folder Access Readiness, client-owned key rotation, server blindness, direct
  deletion, and recoverability decisions remain in force.
- The internal-beta population permits a coordinated hard cut and quiet
  reconciliation, but test data status does not permit speculative or
  metadata-only mutation.
- The central compatibility promise is asymmetric: old readers remain useful;
  old access writers are refused after cutover.
- The central UX promise is equally simple: email identifies the person, Finite
  transparently includes their trusted agents, and every action remains signed
  and attributable to the principal that actually performed it.
