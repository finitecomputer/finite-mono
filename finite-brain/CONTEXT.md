# FiniteBrain Context

## Glossary

### Brain

One knowledge space inside the FiniteBrain product, containing Folders,
content, membership, access, and cryptographic grants. A Brain is either the
user's single Personal Brain or an Organization Brain commonly named in speech
as, for example, “Acme Brain.” **Org Brain** is the accepted short form for an
Organization Brain. _Avoid_: Vault, Personal Vault, Organization Vault,
Organizational Brain.

### FiniteBrain

The product through which humans and agents create, open, organize, share, and
sync Brains. FiniteBrain is the product name; Brain is the contained knowledge
space.

### Brain ID

The stable product-facing identifier for one Brain across the Product Client,
CLI, public API, sync, and sharing contracts. _Avoid_: Vault ID, `vaultId`.

### FiniteBrain Portable v1

The hard-cut implementation target for the Rust rebuild. It is defined by
`docs/specs/finitebrain-portability-spec.md` and covers Brains, Folders, Folder
Objects, Folder Key Grants, sync, sharing, OKF import/export, and compatibility.

### FiniteBrain Policy

Application-specific behavior for Brains, Folders, access, sync, storage,
sharing, OKF, hardening rules, the Product Client, and the Smoke UI.
FiniteBrain Policy belongs in the `finite-brain` workspace, not in
`finite-nostr`.

### Reusable Nostr Primitive

A generic Nostr operation that can be reused across Finite repos without
knowing about Brains or Folders. Examples include NIP-19 identity
encoding, event serialization and verification, NIP-44 encryption adapters,
NIP-59 gift-wrap helpers, and NIP-98-style HTTP authorization helpers.

### Smoke UI

A development-only HTML/CSS interface served by the Rust app for local
end-to-end verification. It is not the product client. It exists to inspect
Brains, Folders, encrypted objects, sync state, grants, invitations, shares,
and mounts while the Rust core and server mature.

### Product Client

The trusted browser experience a Member Identity's controller uses to open a
Brain, connect a Brain Identity Provider, open Folder Key Grants, decrypt accessible Folder Objects,
materialize Pages, edit content, sync changes, run local search/graph indexes,
and perform OKF import/export. Unlike the Smoke UI, the Product Client owns the
normal member workflow.

### Brain Identity Provider

The versioned, product-facing capability contract through which the Product
Client uses an acting Member Identity. FiniteBrain defines the allowed typed
intents, such as identifying the Member, authorizing a Brain-bound request or
revision, and opening or wrapping an appropriately scoped Folder Key Grant.
FiniteBrain retains ownership of Brain, Folder, content-crypto, and grant
policy. It also owns its own hosted-now/native-later adapter. The contract
never exposes a raw identity secret or generic sign/decrypt operation to
Product Client code. Hosted, Electron, and iOS adapters may implement the same
contract for one User Nostr Identity despite their different custody models.
Finite Identity supplies key-storage and lifecycle primitives; it does not own
a universal product adapter. The adapter opens a validated Folder Key Grant,
but the Product Client holds the resulting Session Folder Key and continues to
read, write, encrypt, and decrypt Brain content itself.
Only the official Brain Product Client may invoke the adapter; ordinary
dashboard pages, Sites content, and embedded frames never receive that
capability.

### Dashboard-Aligned Product Theme

The Product Client's presentation language derived from the Finite dashboard:
warm neutral surfaces, blue product accents, Funnel typography, restrained
depth, rounded controls, and the dashboard's system-driven light and dark
appearances. It changes presentation without changing the Product Client's
Brain workspace layout, security state, or member workflows.

### Product Client Spine

The minimum trusted-client workflow that later client features build on:
connect the acting Member Identity's Brain Identity Provider, load Brain state, open current Folder Key
Grants, decrypt readable Pages, edit one Page, encrypt and write the Page back
as a signed revision, and pull/apply sync records without losing unresolved
local edits.

### Member Identity

A Nostr `npub` that can hold Brain Membership, receive Folder Access, and open
Folder Key Grants. FiniteBrain does not classify whether a human, agent, shared
client, or several clients control it; separate keypairs are separate Member
Identities. A product or Agent Runtime may provision and label separate
keypairs, but that client-side policy does not create a different FiniteBrain
authorization class. In particular, an Agent Principal Key receives no Brain
access merely because it belongs to the same Project or dashboard account as a
user; Brain must establish the applicable Personal Brain Agent Access or
Account Access Cohort and every required Folder Key Grant. A Personal Brain
tracks its owner's live Account Agent Set, while invited access uses the fixed
participant rules of the accepted invitation. Other Folder-limited identities
are Guests and receive only their explicit Folder access, independently of the
Folder's native access mode.

### Brain Role

A Member Identity's Brain-wide authorization relationship, such as Member,
admin, or owner. A Brain Role does not by itself prove that the Member Identity
can decrypt a Folder; readable Folder content also requires current Folder
Access and a current Folder Key Grant. Guest is a Folder-limited relationship,
not a Brain Role. _Avoid_: Brain Access.

### Member

A Member Identity that belongs to a Brain. In either a Personal Brain or an
Organization Brain, a Member is entitled to every current and future
all-members Folder, while restricted Folders still require explicit Folder
Access Readiness. Removing Membership removes all of that identity's access to
the Brain rather than converting it into a Guest. _Avoid_: Collaborator, User.

### Guest

A Member Identity that does not belong to a Brain broadly but has explicit
Folder Access Readiness for one or more Folders in it. A Guest never inherits
access to all-members Folders merely because it can open an invited or mounted
Folder. Explicit Guest access is independent of the Folder's native access mode.
When the identity has no direct Folder access and participates in no Mount for
that Brain, the active Guest relationship ends while its audit history remains.
_Avoid_: Limited Member, External Member.

### Brain Invitation

A pending, single-use offer addressed to one human-facing email or Member
Identity to join a Brain. Sending fixes one Invitation Participant Set and
acceptance enrolls that complete set as Members; it never makes the included
identities equivalent, transfers Personal Brain ownership, or implicitly grants
an Organization Brain admin role. Cancellation applies only while pending;
acceptance consumes the invitation and later membership removal is a separate
administrative operation. _Avoid_: Brain Share.

### Invitation Participant Set

The fixed set of distinct Member Identities authorized when a Brain Invitation
or Folder Invitation is sent. For an invitation addressed to a Finite user's
email, it includes that User Nostr Identity and a send-time snapshot of the
user's Account Agent Set, minus any candidates the inviter explicitly approves
excluding after Invitation Preflight. Agents created later do not inherit
access from the invitation. At acceptance, an explicitly approved narrowing may
remove a participant that has since become permanently ineligible, but it can
never add an identity or otherwise expand the send-time authorization. _Avoid_:
Shared Identity, Group Principal.

### Invitation Acceptance Narrowing

The recipient-approved removal of a participant that became permanently
ineligible after an invitation was sent. Finite identifies the affected
participant before acceptance, requires explicit confirmation, and records the
narrowing for both sides. Acceptance then applies atomically to the remaining
Invitation Participant Set. Temporary runtime unavailability is not a reason
to narrow. _Avoid_: Partial Acceptance, Silent Exclusion.

### Account Access Cohort

The durable Brain-owned relationship connecting one email-addressed User Nostr
Identity with the Agent Principals authorized to share that user's Brain or
Folder scope. It may be established by accepted invitation, Organization Brain
bootstrap, explicit access grant, or Existing Access Cohort Reconciliation.
Removing the user's access also revokes every access path the cohort supplied to
those agents, including required Folder Key rotations. An agent keeps access
only when a separate, independently authorized relationship also grants it. A
cohort coordinates access and lifecycle; it is not a shared identity or group
principal. _Avoid_: Account Principal, Shared Membership.

### Cohort Participant Revocation

An admin-authorized removal of one Agent Principal from an Account Access
Cohort without removing the addressed user or other agents. It revokes every
access path the agent holds in the affected Brain or Folder scope, performs the
required Folder Key rotations, and records a durable cohort exclusion so retry
or reconciliation cannot silently restore it. Restoring the agent requires a
new explicit authorization. _Avoid_: Temporary Agent Removal, Cohort Sync.

### Scoped Cohort Exclusion

A durable admin-authorized exception that removes one cohort Agent Principal
from one restricted Folder without removing the agent from the Brain, another
Folder, or the rest of its Account Access Cohort. It rotates that Folder's
key and prevents later email-addressed reconciliation from silently restoring
the excluded access. Restoring the agent to that Folder requires a new explicit
authorization. _Avoid_: Whole-Cohort Removal, Temporary Folder Exception.

### Cohort Participant Admission

An explicit, audited admin authorization that adds one currently account-owned,
grant-ready Agent Principal to an existing Account Access Cohort after the
original invitation was sent or accepted. A fresh readiness check must succeed,
then the agent receives the cohort's current Brain or Folder scope without
requiring the addressed user to accept another invitation. It never follows
automatically from agent creation, repair, or restart. _Avoid_: Automatic Agent
Inheritance, Invitation Replay.

### Email-Addressed Access Grant

A later Brain or restricted-Folder access grant addressed to the human-facing
email of an existing Account Access Cohort. It grants the same scope to the
addressed User Nostr Identity and every non-excluded agent in that cohort after
the same readiness preflight; any reduced set requires explicit approval. It
does not add agents created after the cohort was fixed unless they first receive
an explicit Cohort Participant Admission. _Avoid_: Human-Only Email Grant,
Implicit New-Agent Access.

### Email-Addressed Access Revocation

A Brain or restricted-Folder access removal addressed to the human-facing email
of an Account Access Cohort. It removes the addressed user's access and each
cohort agent's access derived from that user at the same scope, then performs
the required Folder Key rotations. A separate, independently authorized agent
relationship survives unless the operation explicitly targets that Agent
Principal too. _Avoid_: Human-Only Email Revocation, Metadata-Only Removal.

### Account Agent Set

The Agent Principals owned by one Finite account that have completed identity
provisioning and have not been permanently retired or deleted. Runtime health
does not affect membership in this set: stopped, offline, restarting, and
temporarily unhealthy agents remain included. Agents still being created,
failed creations, deleted or retired agents, and agents merely shared with the
account are excluded. _Avoid_: Online Agents, Shared Agents.

### Account Agent Departure

The permanent unlink, retirement, or deletion of an Agent Principal from its
owning Finite account. It automatically ends Human-Anchored Agent Authority and
revokes every Brain or Folder access path the agent received through that
account, including the required Folder Key rotations. Independently authorized
access remains only when the agent identity itself still validly exists.
Temporary runtime stops, restarts, relocation, and transient health failures are
not departures and never trigger revocation. _Avoid_: Agent Offline, Runtime
Stop.

### Invitation Preflight

The read-only readiness check performed before an invitation exists. It
resolves the addressed identity, builds the candidate Invitation Participant
Set, and verifies that every candidate can receive the invitation's required
Brain or Folder access. If a candidate is not ready, no invitation is sent;
the inviter receives a clear proposed reduced set and may explicitly approve
excluding the blocked candidate. The resulting invitation records its approved
participant set, exclusions, reasons, and approving actor. _Avoid_: Partial
Invitation, Best-Effort Invite.

### Invitation Result Summary

The minimal human-facing completion message derived from the authoritative
structured invitation result. A successful multi-agent result reads like
`Invited paul@finite.vip and 2 of his agents: Waffle and Biscuit.`; an approved
exclusion adds one short sentence naming the omitted agent and reason. The
service returns each participant's relationship and display identity, the CLI
renders the stable summary, and the managed FiniteBrain skill repeats it
naturally rather than reconstructing account relationships. Raw principal keys
stay out of ordinary user-facing copy. _Avoid_: Participant Dump, Agent-Inferred
Summary.

### Agent-Assisted Invitation Acceptance

The Brain-owned, invitation-scoped authority for an Agent Principal in a
Finite user's Account Agent Set to accept an invitation addressed to that
user's Finite VIP Mailbox Address after the user asks it to do so. The agent signs as
itself, Finite verifies the current account-agent relationship, and acceptance
enrolls the invitation's full participant set. It never makes the agent the
User Nostr Identity, exposes the user's private key, or causes invitations to
be accepted merely because they arrive. Product audit identifies the acting
agent and the delegated acceptance. _Avoid_: Agent Impersonation, Auto-Accept.

### Account Invitation Inbox

The single account-scoped view of pending Brain Invitations and Folder
Invitations addressed to a user's Finite VIP Mailbox Address. The human Product Client
and every Agent Principal in the invitation's approved participant set read the
same invitation record after Finite verifies their current account relationship;
the record is not copied into separate per-agent invitations. Email to the
addressed user is the only proactive notification. Agents may query the inbox
and explain or accept an invitation after the user asks, but Finite does not
proactively message or wake every agent. Acceptance or cancellation updates the
one record for every viewer. _Avoid_: Agent Invite Copies, Bot Notifications.

### Invitation Email Notice

The one proactive message sent to the human-facing email addressed by an
invitation. It identifies the inviter and Brain or Folder and states the number
of the recipient's agents included, using minimal copy such as `You and 2 of
your agents were invited to Acme.` Agent names and full participant details stay
in the Account Invitation Inbox; Finite sends no separate agent emails. _Avoid_:
Participant Roster Email, Agent Notification Email.

### Invitation Inbox Dismissal

A reversible preference that hides one pending invitation from an account's
default Account Invitation Inbox without changing the invitation's lifecycle.
It neither consumes nor rejects the invitation, grants access, nor notifies the
inviter. The human or an authorized account agent may restore it while the
invitation remains pending. Recipients have no separate permanent-decline
transition; an invitation otherwise remains pending until acceptance, expiry,
or inviter revocation. _Avoid_: Decline, Reject Invitation.

### Human-Anchored Agent Authority

The Brain-owned, revocable authority for an Agent Principal enrolled through a
user's Account Access Cohort to exercise that user's current routine Brain
administration powers on the user's behalf. The agent remains a distinct Member
Identity, signs as itself, and never receives an independent admin or owner
role. The verified account-agent relationship and cohort enrollment provide
standing product authority for normal operations; Brain requires no separate
human signature or approval ticket for each normal action. Changing another of
the user's agents inside a Personal Brain additionally requires Authenticated
Human Intent. The authority exists only while the agent remains account-owned
and the user retains the required Brain Role; demoting the user immediately
removes the corresponding delegated power. It never authorizes Brain ownership
transfer, Recovery Set changes, whole-Brain deletion, or changes to the user's
underlying account-agent relationships. Product audit identifies both the
acting agent and the anchoring user. _Avoid_: Agent Admin, Shared Role, User
Impersonation.

### Authenticated Human Intent

The one-use discretionary capability that proves an Agent Principal is acting
inside a fresh authenticated human turn before one sensitive Brain action.
Finite Chat's signed assertion binds the human, acting agent, freshness, and
nonce; Brain combines it with the exact route-derived target, scope, and
operation and consumes the assertion id atomically, regardless of which action
is attempted. Agent-supplied conversational text or an Agent-computed binding
is not proof that the human's words semantically requested that exact action;
the product's personal-agent trust model assigns that translation to the
agent. It requires no human private-key signature or Product Client
click. Personal Brain changes that remove, restrict, or
restore another account agent require this intent, while normal Brain work uses
standing authority and ownership transfer, Recovery Set changes, and
whole-Brain deletion remain directly human-operated. Product audit records both
the authorizing human and acting agent. _Avoid_: Agent Assertion, Human
Impersonation, Manual Approval Click.

The assertion id is derived from the canonical signed claims rather than the
caller-provided hexadecimal MAC spelling, so case-normalized encodings cannot
bypass one-use consumption.

### Durable Agent Authority Record

The Brain-owned durable evidence created after Finite authoritatively verifies
an account-agent relationship during invitation, admission, or an explicit
relationship change. Routine authorized Brain work relies on this record and
does not require the account service to be reachable for every action. Adding,
removing, or changing an agent requires fresh authoritative verification, and a
known Account Agent Departure revokes the record immediately. The record is
part of Brain recovery state; restoring Brain without its agent authorities and
revocations is incomplete. _Avoid_: Live Account Check, Cached Account Guess.

### Existing Access Cohort Reconciliation

The quiet, one-time transition that converts each existing email-linked human
Brain or Folder access relationship into the intended Account Access Cohort
with every currently eligible account agent. It creates no pending invitation,
sends no invitation email, and never represents metadata-only membership as
usable access. A trusted client opens each accessible current Folder Key,
prepares the agents' encrypted grants, and applies one complete scope
atomically; an incomplete scope leaves existing access intact, reports truthful
retryable state, and converges later. Completion is summarized minimally, such
as `Updated Paul's access for 3 agents.` The atomic unit for an existing Member
is one Brain and every Folder that user can access there; separate Brains
reconcile independently. A Folder-only Guest relationship reconciles atomically
for that Folder. _Avoid_: Retroactive Invitation, Membership Backfill, Silent
Partial Migration, Account-Wide Transaction.

### Cohort Access Cutover

The coordinated release boundary after which every invitation and access
mutation must use account-aware cohort semantics. Older clients retain read,
sync, and chat-compatible paths, but their invitation and access writes fail
with a clear update-required result instead of creating human-only or
single-principal state. Core, Identity, Brain, the Product Client, CLI, and the
managed skill must be cohort-capable before the cutover and Existing Access
Cohort Reconciliation begins. _Avoid_: Mixed-Writer Rollout, Silent Legacy
Write.

### Folder Invitation

A pending, single-use offer of Folder Access Readiness for one Folder. It may be
sent from either a Personal Brain or an Organization Brain. Sending fixes one
Invitation Participant Set and acceptance grants that complete set access to
the invited Folder, but never to any other Folder in the Brain. It creates Guest
relationships when needed, expires if unused, and does not create a relationship
with another Brain. Cancellation applies only while pending; acceptance consumes
the invitation and later Folder Access Revocation is a separate administrative
operation. _Avoid_: Share Link, Folder Share.

### Mount Offer

A pending, single-use offer to connect one source Folder to one named
destination Brain. It is addressed to one destination owner or admin and cannot
be accepted into another Brain. Acceptance creates a Shared Folder Connection
and Folder Mount rather than copying or changing the native access mode of the
Folder. Any Folder may be offered without first becoming a special share source,
and an unused offer expires. _Avoid_: Shared Folder Invitation.

### Shared Folder Connection

The durable, revocable relationship that makes one source Folder a shared
workspace in a destination Brain. Either Brain may be Personal or Organization;
each side retains its own governance, destination participants are Guests of
the source Brain, and either side may end the relationship. _Avoid_: Folder
Share.

### Mount Participant

A destination Brain owner, Personal Brain Agent, admin, or Member selected by
that Brain's governance to use a Folder Mount. A Mount Participant is a Guest
of the source Brain and requires Folder Access Readiness for the mounted source
Folder; a destination Guest is not eligible. _Avoid_: Connection Member, Shared
Folder Member.

### Folder Mount

The destination Brain's visible reference to a source Folder through a Shared
Folder Connection. A Folder Mount is not a copy and does not by itself grant
Folder Access Readiness. It cannot itself become the source of another Folder
Invitation or Mount Offer; only the native source Brain may extend access.
_Avoid_: Shared Folder, Synced Folder.

### Folder Access Readiness

The observable state in which a Member Identity is entitled to a Folder under
current policy and holds a valid Folder Key Grant for the Folder's current key
version. Policy entitlement without a current grant is incomplete and must not
be presented as readable access. _Avoid_: Effective Access.

### Folder Access Revocation

The atomic, client-owned transition that removes one or more identities from a
Folder and advances its Folder Key while granting the new current key to every
remaining authorized identity. It prevents future access but does not claim to
erase plaintext or earlier key material already obtained. Removing a Member
applies this transition to every Folder the identity could access. _Avoid_:
Ungrant, Link Revocation.

### Organization Brain Collaboration

The desired state in which a target Member Identity has the requested
Organization Brain Role and Folder Access Readiness for every current Folder
included by the collaboration scope. The default admin collaboration scope is
all existing Organization Brain Folders; a partial collaboration names every
unready Folder and remains safe to retry. _Avoid_: Admin Sharing, Brain Access.

### User Nostr Identity

The human-controlled Nostr `npub` used across Hosted Web, Electron, and iOS.
In FiniteBrain it is a Member Identity and receives the appropriate Brain
ownership or membership, Folder Access, and Folder Key Grants. Hosted Web uses
it through a server-held Brain Identity Provider; Electron and iOS use the same
identity from protected local storage. The custody difference does not create
another Brain identity. Account Auth may authorize a Hosted Web session but does
not grant Brain access. A User Nostr Identity remains distinct from every Agent
Principal Key. In the first hosted phase, the Finite Chat Hosted Device is the
user-facing setup and custody entry point; Brain's adapter owns only
Brain-specific operations. Hosted Brain assumes that setup already exists. If
it does not, Brain fails closed with a basic setup-required state and never
creates another User Nostr Identity. This is a Greenfield boundary: Brain
carries no legacy Brain or user-key migration path into the first release.

### Organization Brain Requester

The authenticated human whose direct request causes an Agent Principal to
create an Organization Brain on the human's behalf. Organization Brain
Bootstrap atomically creates the Brain, makes this requesting User Nostr
Identity an initial member-admin, and enrolls a snapshot of the requester's
current Account Agent Set as initial Members in the same Account Access Cohort.
Those agents exercise the requester's routine admin powers through
Human-Anchored Agent Authority; they do not receive independent admin roles. It
creates no Folder, Folder Key, or Folder Key Grant; those appear only when an
admin explicitly creates a Folder. If the requester, cohort, membership, or
human admin role cannot be established, no Brain is created. This is a
Brain-enforced bootstrap, not a sequence of later membership mutations.

The managed FiniteBrain skill passes the requester from authenticated message
metadata, never from identity text supplied in the conversation. If that
authenticated requester metadata is unavailable, the agent does not guess,
require a raw requester identity argument, or create an agent-only Brain; it
briefly asks the user to retry from an authenticated chat context. The
agent-facing CLI has no requester-identity override.

A clear natural-language request to create the Organization Brain is sufficient
authorization for this bootstrap. The agent does not add another confirmation
step; after creation it reports the human and included-agent count minimally.

### Personal Brain Agent Access

The automatic, revocable way every distinct Agent Principal in a Personal Brain
owner's live Account Agent Set works in that Personal Brain. The User Nostr
Identity remains the Brain's sole owner. Each Personal Brain Agent has full
operational access to every current and future Folder: it may read, write,
organize, share, invite collaborators, and directly delete content or Folders on
the user's behalf. Brain maintains the Folder Key Grants needed for that access:
every new Personal Brain Folder grants its owner and current Personal Brain
Agent Set regardless of which one creates it. There is no selected-agent,
Agent Workspace, or Folder-by-Folder delegation ceremony for the default path.

Personal Brain Agent Access never authorizes an agent to transfer or delete the
Brain, change its owner or Recovery Principals, manage the owner's account-agent
relationships, or use the user's Brain Identity Provider. Account ownership
must be authoritatively verified; Project or dashboard navigation alone carries
no authority. Each action remains signed and audited as the acting Agent
Principal, with its Managed Agent NIP-05 used as the readable display identity;
Brain never impersonates the human owner.

### Personal Brain Agent

An Agent Principal in the Personal Brain owner's live Account Agent Set with
desired Personal Brain Agent Access. Once Personal Brain Agent Readiness is
complete, every Personal Brain Agent has the same full operational and
collaboration authority across all current and future Brain content, including
accepted Folder Mounts, while ownership, recovery, and whole-Brain deletion
remain exclusive to the human owner. This is not an admin or owner Brain Role.
_Avoid_: Personal Agent, Delegated Agent, Personal Brain Admin.

### Personal Brain Agent Set

The live Account Agent Set of a Personal Brain's human owner. Personal Brain
Bootstrap establishes access for every currently eligible agent, and each newly
provisioned eligible account agent automatically enters the desired set without
a new invitation. Account Agent Departure automatically removes the agent,
revokes its authority, and rotates every affected current Folder Key without
deleting the Personal Brain or its content. Runtime stops, restarts, relocation,
and transient health failures do not change the set. _Avoid_: Selected Personal
Agent, Personal Agent Slot.

### Personal Brain Agent Readiness

The separate capability state proving that one Personal Brain Agent has its
durable authority and a current Folder Key Grant for every Folder in the
Personal Brain. A newly provisioned agent may launch, chat, and perform unrelated
work while Brain prepares these grants in the background. Until readiness is
complete, Brain work reports minimally that the agent is still connecting and
retries automatically or on demand; it never presents partial Folder access as
complete. Brain readiness is not overall Agent Readiness. _Avoid_: Agent Setup
Blocker, Partial Personal Brain Access.

### Direct Deletion

A permanent removal from Brain's live product state with no Trash, undo, or
restore workflow. Brain retains only the minimal deletion marker and audit
metadata needed to synchronize clients and prevent stale or offline edits from
resurrecting the deleted identity; it does not claim erasure of downloaded
plaintext, backups, snapshots, or storage history. _Avoid_: Secure Erasure,
Trash.

### Personal Brain Bootstrap

The creation of a user's single Personal Brain with that user's User Nostr
Identity as sole owner. It seeds no default Folders or Folder Objects; Folders
appear only through an explicit user action or a product workflow the user
authorizes. An account-bound agent may perform bootstrap under its standing
Agent Bootstrap Authority. Both agent-first and user-first setup resolve the
owner's current Account Agent Set and atomically establish the empty Brain's
Personal Brain Agent Set without creating a Folder merely for those
relationships. If the owner or account-agent set cannot be authoritatively
resolved, no Brain or partial agent relationship is created.

### Organization Brain Bootstrap

The creation of an empty Organization Brain with one human initial member-admin
and an Account Access Cohort containing a snapshot of that human's current
Account Agent Set as initial Members. Direct Product Client creation uses the
signing human; agent-created bootstrap uses the authenticated human requester.
The agents receive Human-Anchored Agent Authority rather than independent admin
roles. Agents created later require explicit Cohort Participant Admission,
unlike the live Personal Brain Agent Set. Bootstrap seeds no default Folders,
Folder Objects, Folder Keys, or Folder Key Grants; those appear only when an
admin explicitly creates a Folder.

Initial Brain bootstrap relationships are active memberships and roles, not
pending invitations. Invitations are reserved for adding a Member Identity
after a Brain already exists.

### Agent Bootstrap Authority

The standing authority of an authenticated account-bound Agent Principal to
create its user's single Personal Brain and atomically establish the owner's
current Personal Brain Agent Set. The FiniteBrain skill asks the user once in
natural language before exercising this authority, but that confirmation is
behavioral guidance, not a server-enforced authorization boundary. If the Brain
already exists, the authority converges an eligible newly provisioned agent on
Personal Brain Agent Access only through the live-set admission workflow; it
cannot create a second Personal Brain, transfer or delete the Brain, change
ownership or Recovery Principals, or selectively manage another agent. _Avoid_:
Setup Ticket, Bootstrap Approval.

After successful agent-first bootstrap, the agent resumes the user's original
request without requiring another prompt.

Core is the source of truth for the WorkOS account-to-agent association. Finite
Identity manages each Agent Principal Key inside the agent's protected
environment and resolves its Managed Agent NIP-05 to the public key; its server
never returns a private key. Brain combines Core's associations with Identity's
public Principal facts and owns the resulting Personal Brain Agent Access.
Finite Chat Hosted Device remains the hosted human-key custodian and signer, not
part of the Personal Brain bootstrap path or Brain access authority.

The agent never supplies the Personal Brain owner. Brain derives the owning
account from Core's authenticated account-agent association and resolves that
account's existing User Nostr Identity through Finite Identity; missing,
ambiguous, or conflicting facts fail without creating or changing a Brain.

In Hosted Web, agent display names and Managed Agent NIP-05 Names may describe the
owner's Personal Brain Agent Set, but navigation context carries no authority.
Brain resolves the live set through Core and Finite Identity rather than asking
the owner to pair one selected agent. Raw `npub` values are advanced diagnostics,
not the primary user experience. After admission, each Agent Principal discovers
the user-owned Personal Brain through the signed visible-Brain list and opens
its accessible Folders in a durable Brain Working Tree below the Runtime's
`/data/workspace` boundary.

### Local Data Security Baseline

The FiniteBrain-wide policy for how trusted clients and Agent Runtimes handle
local secret material, decrypted content, derived plaintext state, retention,
and egress. It applies regardless of which UI or editor provides the local
experience.

### Session Folder Key

A Folder Key opened for one running trusted-client session. It is not durable
local state and must be reopened from an encrypted Folder Key Grant when a new
session needs it.

### Session Lock

A trusted-client state in which Session Folder Keys and temporary plaintext
state are unavailable and automatic grant reopening is blocked until the
Member explicitly resumes the grant-opening flow. A Session Lock hides client
content but does not claim to erase a separately created Brain Working Tree.
An empty Brain with no Folders or Folder Key Grants may still have a normal
unlocked session; having no grants to open is not itself a Session Lock.
In Hosted Web, explicitly opening Brain from the authenticated dashboard is
the Member's Resume action and may automatically reopen valid Folder Key
Grants. After a lock, the Member must explicitly open Brain again; Account Auth
selects the hosted session but remains neither Brain authority nor a signer.
The browser Product Client applies the same lock before page navigation or
back/forward-cache suspension and whenever a signed event no longer matches the
Member Identity connected for the current session.
In Hosted Web, Account Auth logout or session expiry also locks the Product
Client and invalidates the Brain hosted-adapter session. Locking never revokes
the underlying Membership, Folder Access, or Folder Key Grants. It also does
not stop an Agent Runtime using its distinct Agent Principal Key and explicit
Folder access; stopping that agent requires explicit access revocation and the
required Folder Key rotation.
A newly delivered invitation fragment is handled as a one-shot pre-session
capability: the client removes it from browser history immediately, holds it in
memory outside the locked content session, and imports it only after explicit
Resume. Explicit Lock, Brain switching, or a failed Resume discards it.

### Ephemeral Client Plaintext

Decrypted content and derived readable state held by a browser or desktop
client only while its session is unlocked. It is not retained as durable local
state after the session ends.

### Encrypted Recovery State

Durable client-side ciphertext that preserves unsent work or other restart
state without retaining readable plaintext. It becomes readable only after the
acting Member Identity unlocks the relevant Folder again.

### Plaintext Egress

Any transfer of decrypted content or content-derived readable metadata beyond
the Trusted Device Boundary. FiniteBrain's cryptographic authorization ends at
decryption; first-party clients deny automatic Plaintext Egress, while a Member
Identity's controller remains responsible for explicitly initiated exports and
for the behavior of third-party clients.

### Paused Brain Working Tree

A Brain Working Tree whose FiniteBrain sync, signing, and automatic Folder Key
opening are stopped while its existing plaintext files remain on the Trusted
Device. _Avoid_: Locked Working Tree.

### Brain Working Tree Removal

The explicit deletion of a Brain Working Tree's local plaintext projection.
It does not claim secure erasure from backups, snapshots, or storage history.

### Trusted Device Boundary

The local OS account and storage boundary trusted to hold a Member Identity's
persistent secret and authorized plaintext. Obtaining that secret is a complete
trusted-client compromise for the Member Identity, not a failure contained to
one Folder or Finite product.

### Folder-scoped LLM Wiki

The FiniteBrain knowledge model. A Brain is a namespace of many LLM wikis, and
each Folder is the enforceable wiki scope because Folder Keys and Folder Access
define who can read it. Folder-local `_index.md`, `config.md`, and `log.md`
describe only that Folder. Root/global indexes must not leak private Folder
titles, summaries, sources, or activity.

### Hybrid Wiki Search

The agent-facing local retrieval capability over the readable Markdown in a
Vault Working Tree. It combines lexical and semantic relevance when available,
falls back to lexical relevance alone, and returns one merged result list from
the acting Member Identity's readable Folders. It returns locations in the
original Pages rather than creating another knowledge authority.

### Markdown Section

The canonical Hybrid Wiki Search retrieval unit: the readable content under a
Markdown heading together with its Page path, Page title, and heading ancestry.
A Page without headings is one Markdown Section, and bounded subdivisions of a
long section retain the same document context.

### Embedding Provider

The replaceable capability that converts a Markdown Section or search query
into a semantic vector for Hybrid Wiki Search. Provider model identity and
version belong to the derived index lifecycle; the provider does not become a
knowledge authority or modify the underlying Page.

### Search Evidence

A ranked Hybrid Wiki Search result that identifies an original Markdown
Section, its location, a short excerpt, local-sync disposition, and contributing
retrieval signals. Search Evidence guides an agent to source Pages; it is not a
generated answer or a new durable knowledge artifact.

### Asset

A non-Markdown source such as a PDF, image, audio file, or other blob whose
bytes live outside Folder Objects. One Asset Source Note points to those bytes.
An Asset is evidence or source material; it is not the primary LLM Wiki
knowledge surface. _Avoid_: Inline Asset.

### Asset Reference

The small, OKF-compatible frontmatter contract in an Asset Source Note. It has
a `type`, `title`, and one canonical `resource`; `description` and known
integrity facts are optional. The `resource` may be an external URI or a
machine-local file URI. It does not contain the Asset bytes and is not a second
Folder Object.

### Asset Integrity

The relationship between an Asset Reference and the exact Asset bytes it
describes. Integrity is verified only when the Asset Reference includes a
provider revision or content hash. A bare `resource` remains useful and
discoverable, but is never presented as immutable evidence.

### Source Note

A Markdown Page that describes one captured source with provenance, extraction
status, and human or agent-readable notes. Source Notes are the readable handles
that LLM Wiki pages cite when synthesizing knowledge from raw material.

### Asset Source Note

The single Brain-resident Markdown representation of a non-Markdown Asset. Its
Asset Reference points to the bytes, while its body lets humans, agents,
search, and graph flows reason over the source. Agents conventionally place it
under the Folder's `raw/` tree; there is no Brain-resident `raw/assets/` blob
directory. _Avoid_: Asset Source Note Pair.

### Graph View

A Product Client view over the acting Member Identity's decrypted accessible Pages. It
renders Page nodes and Page relationships only after Folder Keys are open and
visibility filtering has been applied.

### Graph Replay

A Product Client playback of graph/index changes derived from the client's
applied sync history and decrypted Page index. It is not a server-side graph
event log.

### OKF Import Execution

A Product Client workflow that parses readable OKF, plans import conflicts,
opens destination Folder Keys, encrypts imported Pages client-side, signs
Folder Object revisions, and uploads those revisions through normal secure
object routes. The Rust server does not parse readable OKF or receive
plaintext Page content during import.

### Brain Working Tree

A local agent-facing file projection built from already-decrypted accessible
Pages. It materializes readable Folders as Folder-scoped LLM wiki roots with
local `AGENTS.md` or `HUMANS.md` when present, `_index.md`, `config.md`,
`log.md`, `raw/`, `wiki/`, `inventory/`, `datasets/`, and `output/`
conventions. It is an explicitly created persistent plaintext copy inside the
Trusted Device Boundary, remains until its controller removes it, and is
private to the controlling OS account at its root and FiniteBrain control-state
boundary. It stores only safe locked metadata for inaccessible Folders and maps
file changes back into Product Client encrypted-object write, move, and delete
intents.

### Agent CLI

The terminal control surface for a trusted Agent Runtime working inside a Brain
Working Tree. It explains and controls identity, local daemon state, automatic
sync health, blocked edits, activity, and access reasons while the controller
reads and writes ordinary files; each operation opens the Folder Key Grants it
needs without creating a durable CLI unlock state.

### Agent Sync Daemon

The resident trusted-client process that watches a Brain Working Tree, opens
available Folder Keys for the acting Member Identity, detects file changes,
syncs with the server, and records blocked states that require controller
resolution.

### Brain Update Notification

A server-sent, content-free hint that tells a connected trusted client that an
accessible Brain may have a newer authoritative sequence or changed access.
Clients briefly coalesce bursts of these notifications, then reconcile through
the normal authenticated sync contract. A Brain Update Notification is not a
durable sync record or source of truth; missed, delayed, duplicated, or
reordered notifications never replace sequence-based catch-up.

### Local Agent Signer

A trusted signer available to the Agent Runtime instead of a browser Brain
Identity Provider. It exposes the same conceptual abilities the Product Client
needs: identify the acting npub, sign FiniteBrain events, and perform NIP-44
encryption and decryption for Folder Key Grant handling; its npub is an
ordinary Member Identity with no agent-specific authorization semantics. It
opens only Folder Key Grants addressed to that Agent Principal Key and never
uses the user's Brain Identity Provider or User Nostr Identity.

### Recovery Principal

A distinct, narrowly authorized Principal whose Folder Key Grants provide an independent recovery path when the primary human or agent key is unavailable.

### Email Access Delegation

A revocable, Brain-owned product authorization connecting one verified email
Principal's account context to one Agent Principal. It records the relationship
for audit and revocation but does not make the two the same Principal or convey
Folder Keys. In a Personal Brain it authorizes Personal Brain Agent Access
relationship; Brain separately and automatically maintains the Personal Brain
Agent Set and Folder Key Grants that make the Brain's current and future Folders
readable. The delegation is not itself a content key.

### Email Invite Bootstrap

A temporary email-address invitation state where email proof authorizes the
claim, an out-of-band invite secret unlocks NIP-59-shaped gift-wrapped
bootstrap material, and accepted access becomes durable only after it is bound
to a User npub.

### Invite Secret

The high-entropy client-only secret carried outside the server-visible invite
code, typically in the URL fragment. For Email Invite Bootstraps, this is the
secret material needed to use the Invite Unwrap Key. It unlocks bootstrap
material only after the recipient proves the invited email. It must never be
sent through server-visible channels such as query parameters, request bodies,
server logs, server-side mailer payloads, email bodies, email tracking links,
analytics redirects, or stored database fields.

### Invite Unwrap Key

A temporary Nostr/secp256k1 keypair generated for an Email Invite Bootstrap.
The public key receives the NIP-59-shaped gift-wrapped bootstrap payload; the
private key is carried client-side as an Invite Secret and must not be stored
server-side. This key is a bearer unwrap capability, not a User identity,
member identity, or permission principal.

### Invite Unwrap Proof

A Nostr event signed by the Invite Unwrap Key during Email Invite Bootstrap
Claim. It proves possession of the client-only Invite Secret without sending
the secret to the server, and binds the claim to the invite code, Brain,
invited email, claimant npub, bootstrap payload hash, and email proof
timestamp.

### Invite Instructions

Agent-readable guidance for a Brain Invitation, analogous to Sites `llms.txt`
but split by proof level for Brain's encrypted access model.

### Public Invite Instructions

Unauthenticated Invite Instructions that disclose only generic claim workflow
guidance. They exclude invited email, Brain identity, Folder identity, access
scope, claim state, Folder Keys, and bootstrap plaintext.

### Post-Proof Invite Instructions

Invite Instructions returned only after the invited email is proven through the
Identity Authority. They may disclose the scoped workflow details needed to
claim, open, and sync the Brain, including human-readable Brain and Folder
names, but never Folder Keys or bootstrap plaintext.

### Email-Targeted Brain Invitation

A Brain Invitation addressed to an email instead of a known Native Principal
npub. In v1,
external email-shaped targets use an Email Invite Bootstrap even if they have
prior email-only proof; only concrete npub/hex targets or active Finite VIP
NIP-05 bindings use the normal npub-bound path. Email targets belong to
invitation flows; direct permission mutations remain for known User npubs. Any
invited email must prove control through the Identity Authority. Invitation
proof authorizes only the invitation claim; it does not create or rebind a
Finite VIP NIP-05 Principal Link unless the claimant separately and explicitly
uses the identity-link flow as the same Principal.

### Email Invite Bootstrap Claim

The acceptance act that grants the invitation's scoped access to the claimant
Native Principal npub after email proof, using the bootstrap material to create
durable npub-addressed access without requiring the inviting admin to come back
online. This is product authorization, not global identity equivalence. Claim
is all-or-nothing: Brain must verify email proof, consume the pending bootstrap,
record the claimant npub, create membership/access metadata, and insert every
required durable Folder Key Grant in one atomic operation.

### Email Invite Bootstrap Authorization

An admin-signed authorization for a future email recipient whose User npub is
not known yet. It fixes the invited email, Brain, authorized Folder scope,
Folder key versions, Invite Unwrap Key, bootstrap payload hash, expiry, and
single-use claim bounds that a later Email Invite Bootstrap Claim must match.
For email-targeted Brain Invitations, the authorized Folder scope includes
current all-members Folders because the accepted recipient becomes a Brain
Member.

### Claim-Authorized Folder Key Grant

A durable Folder Key Grant created by an invited recipient after a valid Email
Invite Bootstrap Claim. The inviting admin authorized the access, while the
recipient's User npub finalized the encrypted grant. The grant is valid only
within the pending invitation's authorized email, Brain, Folder, key-version,
expiry, and single-use claim bounds.

### Blocked Sync State

A local condition where automatic sync cannot safely complete without
resolution. Examples include missing auth, missing Folder Key Grant, locked
Folder, stale base revision conflict, revoked access, unavailable server, or a
working-tree change that cannot be mapped to a secure object intent.

### Hard Cut

A compatibility boundary where FiniteBrain does not carry legacy route,
storage, client, or migration behavior forward. Hard-cut work may import data
through explicit new-format flows such as OKF, but it does not preserve old v1
runtime compatibility as a feature requirement.
