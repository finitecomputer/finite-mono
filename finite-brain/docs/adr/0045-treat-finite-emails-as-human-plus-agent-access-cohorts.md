# ADR 0045: Treat Finite Emails As Human-Plus-Agent Access Cohorts

Status: accepted

Date: 2026-08-07

FiniteBrain treats a human-facing Finite VIP Mailbox Address as the product
address for one human and the account-owned agents authorized to work for that
human. The human
User Nostr Identity and every Agent Principal remain separate cryptographic
identities: each signs as itself, receives its own Folder Key Grants, appears
separately in audit, and can be revoked separately. The email is an addressing
and relationship boundary, never a shared signer or group principal.

This decision makes agents first-class Brain participants. It replaces the
assumption that adding a human normally adds only one human npub and removes the
special one-Personal-Agent slot from Personal Brains.

## Invitations And Later Access

For an existing Finite account, a Brain or Folder invitation addressed to the
human's email runs a read-only preflight before it is sent. Preflight resolves
the User Nostr Identity, snapshots the current eligible Account Agent Set, and
verifies that every participant can receive the intended access. Eligible
agents are account-owned agents with completed identity provisioning that have
not been permanently unlinked, retired, or deleted. Runtime health does not
matter.

If a participant is not ready, no invitation is sent yet. The inviting agent or
client returns one minimal explanation and asks whether to send to the reduced
set. An explicit yes fixes that exclusion in the invitation. The approved
exclusions are supplied to preflight itself, so the returned participants,
capacity outcome, exclusions, and plan id are one immutable decision rather
than a commit-time rewrite. The approved participant set is frozen when the invitation is sent; agents created later do
not inherit access from that consumed invitation. If an approved participant
becomes permanently ineligible before acceptance, the recipient may explicitly
narrow acceptance to the remaining set. Narrowing can only remove authority,
never add it.

Acceptance grants the complete approved set atomically. Brain Invitations make
the human and included agents Members. Folder Invitations make them Guests of
that Folder only. Later access grants or revocations addressed to the human's
email apply symmetrically to that human and the non-excluded agents in the same
Account Access Cohort. A folder-level exclusion may remove one agent from one
sensitive Folder without removing it from the rest of the Brain. Restoring an
excluded or previously unready agent is always explicit and audited.

An included agent may accept the shared invitation after the human asks it to
do so. It signs as itself; Brain verifies its account relationship and records
the acting agent and anchoring human. The agent never needs the human's private
key and does not accept invitations merely because they arrive.

## Invitation Delivery And Copy

Finite sends one email per committed invitation to the addressed human, never
one message per included agent. Minimal email copy states that the human and an
agent count were invited; it does not contain the full agent roster. The human
Product Client and every included agent read one shared
Account Invitation Inbox record. Finite does not copy the invitation or wake
and message every agent.

The authoritative service result contains structured participant relationship
and display facts. The CLI renders a concise summary and the managed skill
repeats it naturally, for example:

`Invited paul@finite.vip and 2 of his agents: Waffle and Biscuit.`

Normal user-facing copy uses emails and friendly agent identities, not raw
npubs. Distinct Brain and Folder invitations to the same mailbox may coexist;
an exact same-scope retry returns the existing invitation and does not redeliver
email. The inbox supports reversible hiding, not a permanent recipient-decline
state. Ignoring or hiding an invitation leaves it pending until acceptance,
expiry, or inviter revocation.

## Roles And Agent Authority

Agents receive their own content access but do not receive independent admin or
owner roles merely because the human is an admin. Instead, Brain stores durable
Human-Anchored Agent Authority. An enrolled account agent may exercise the
human's current routine Brain powers while the human retains the required role.
Demoting or removing the human removes the corresponding delegated power.

The verified account-agent relationship and Brain-owned authority record are
standing authorization for normal work. Routine operations do not require a
human signature, approval click, or live call to Core. This preserves Brain and
chat availability during dependency outages. Adding, removing, or changing an
agent requires fresh authoritative account facts, and a known permanent agent
departure is discovered by the normal agent supervisor, then revokes
cohort-derived access and rotates affected Folder Keys through Brain's exact
preflight/commit boundary. Every administrative control accepted through delegated
authority is audited with both the acting agent and anchoring human.
Temporary stops, restarts, relocation, and transient failures do not revoke
access.

Brain polls permanent departures with its bounded active managed-agent NIP-05
set. Core applies that relevance filter before its 256-fact response bound, so
unrelated account history cannot age a still-relevant revocation out of view.

Changing another account agent's Personal Brain access is fresh-human-turn
gated but remains agent-operated. The acting agent must carry one-use
Authenticated Human Intent from an authenticated conversation. Finite Chat mints a
short-lived server-verifiable requester assertion for the exact human and
acting agent. Brain combines it with the route-derived target, scope, and
operation, consumes the assertion id once regardless of attempted action, and
persists that canonical composite without receiving the human's private key.
Brain records both the authorizing human and acting agent. Ownership
transfer, Recovery Set changes, and whole-Brain deletion remain directly
human-operated.

This deliberately trusts the personal agent to translate the human's natural
language into the selected operation. The assertion proves the fresh human
turn, exact human and acting agent, and one-use boundary; it does not
cryptographically prove the semantics of the human's words. An adversarial-
agent threat model would require an additional structured human confirmation
surface, which this agent-first, no-extra-click decision does not add.

Removing the human from a Brain or Folder removes cohort-derived agent access
at the same scope. Independently authorized access survives. An authorized
admin may revoke one agent without removing the human or other agents; targeted
revocation removes that agent's access, rotates affected keys, and records a
durable exclusion so reconciliation cannot silently restore it.

## Personal Brains

A Personal Brain remains owned solely by its human User Nostr Identity, but it
tracks the owner's live Account Agent Set as its Personal Brain Agent Set.
Every eligible current agent has full operational access across all current and
future Personal Brain Folders. Every newly eligible account agent enters the
desired set automatically; every permanent account-agent departure leaves it
automatically. This replaces the selected one-Personal-Agent slot and its
remove-and-replace ceremony.

Personal Brain Agent Readiness is separate from overall Agent Readiness. A new
agent may launch, chat, and perform unrelated work while Brain prepares its
durable authority and per-Folder encrypted grants. Until complete, Brain work
reports minimally that it is still connecting and retries in the background or
on demand. Brain never presents partial Personal Brain access as complete and
never blocks the rest of the agent on Brain setup.

Personal Brain bootstrap resolves the owner and current Account Agent Set and
creates the empty human-owned Brain with the desired Personal Brain Agent Set.
Any account agent may perform agent-first bootstrap for the authenticated human.
Every later Folder grants its current key to the owner and every ready Personal
Brain Agent. Agents may operate, collaborate, share, and delete content and
Folders, but may not transfer or delete the Brain, change recovery, or alter the
underlying account-agent relationships.

## Organization Brain Bootstrap

Organization Brain creation starts with one human member-admin and an Account
Access Cohort containing a snapshot of that human's current eligible agents as
Members. Those agents act through the human's admin authority rather than
receiving independent admin roles. User-first and agent-first creation produce
the same shape. Agents created later require explicit cohort admission; the
Organization Brain cohort is a fixed collaborative snapshot, not a live
Personal Brain Agent Set.

## Existing Internal-Beta State And Cutover

The current internal-beta Brain population will be reconciled retroactively and
quietly. Existing human access becomes an Account Access Cohort with every
currently eligible account agent. This creates no pending invitations and sends
no invitation emails.

Reconciliation is a cryptographic access operation, not a membership-row
backfill. A trusted client must open the current Folder Keys and prepare one
encrypted grant for every added agent. For an existing Member, the atomic unit
is one Brain and every Folder that human can access there; separate Brains may
converge independently. Folder-only Guest access reconciles atomically for that
Folder. Any incomplete unit leaves existing access unchanged, reports truthful
retryable state, and never claims completion from metadata alone.

The shipped reconciliation control is an explicit read-only preflight followed
by an atomic commit that binds the exact stable plan and a named
`backupReference`. Its inventory includes pending mailbox invitations and
independent Agent access. Pending internal-beta Finite VIP invitations convert
in place to fixed cohort plans: ID, code, expiry, resource kind, scope, and the
existing delivery receipt remain unchanged, and conversion never calls the
mailer. Legacy human-only acceptance fails with update-required until the
conversion is complete.

Core, Identity, Brain, Product Client, CLI, and the managed skill must support
cohorts before cutover. After cutover, old clients retain compatible read, sync,
and chat behavior, but invitation and access writes fail with an explicit
update-required result. Old writers must never recreate human-only access.
The retired singular Personal Agent replacement route also returns
update-required. Exact raw-npub writes remain the explicit narrow-access escape
hatch, including targeted removal of one account agent without removing its
anchoring human or siblings.

## Availability And Recovery

Brain stores durable cohort provenance, authority, exclusions, admissions, and
revocations as product state. Routine authorization does not depend on a live
Core lookup. These records and every encrypted Folder Key Grant are part of the
Brain Recovery Set; a restore that omits them is incomplete.

The server remains blind to Folder Keys. It may validate identities, policy,
signed evidence, grant envelopes, and key versions, but only a trusted client
opens a current Folder Key and wraps it to another principal.

## Superseded Decisions

This ADR supersedes ADR-0023's exactly-one Personal Agent slot and replacement
workflow, ADR-0024's single-agent Personal Brain bootstrap consequence,
ADR-0025's two-admin agent-created Organization Brain shape, and ADR-0026's
selected single-agent Organization Brain pairing. It narrows ADR-0041's
"single-recipient" rule: an invitation is still targeted to one email and
single-use, but that email now resolves to one fixed participant set rather
than one cryptographic principal.

It also amends ADR-0022's Personal Brain deletion actor, ADR-0038's Personal
Brain Mount participant default, and ADR-0036's singular Personal Agent
governance language to use the Personal Brain Agent Set.

ADR-0016's core rule remains: humans and agents are distinct Member Identities.
ADR-0039's client-owned key rotation rule also remains unchanged.
