# Personal Agent And Direct Deletion

## Problem Statement

FiniteBrain currently exposes a product model that no longer matches how
Finite expects a human and their agent to work together. A new Personal Vault
is seeded with permanent `getting-started` and `restricted` Folders, the agent
is limited to a dedicated Agent Workspace with Folder-by-Folder expansion,
agent-first setup depends on the exact `/brain setup` command and a one-use user
authorization issued through Finite Chat Hosted Device, and neither the human
nor agent can delete an entire Folder.

The resulting experience is confusing and brittle. Humans see bootstrap
artifacts they cannot remove, agents cannot naturally complete first-time
setup, and the product asks users to reason about Nostr keys and per-Folder
delegation even though a Personal Agent is intended to be fully trusted to
operate on behalf of its user.

## Solution

A Personal Vault starts empty and has exactly one fully trusted **Personal
Agent**. The human's User Nostr Identity remains the sole owner, while the
Personal Agent uses its own Agent Principal Key and receives full operational
and collaboration access to every current and future Folder. Brain maintains
the necessary per-Folder cryptographic grants automatically; Agent Workspace
and Folder-by-Folder agent delegation disappear from the product model.

Both initialization paths converge on the same state. User-first setup
atomically creates the empty Personal Vault and adds the selected,
identity-resolved agent. Agent-first setup lets the first account-bound agent
create the user's Personal Vault and establish itself as the Personal Agent
after one lightweight natural-language double-check governed by the canonical
FiniteBrain skill. There is no slash command, modal, button, exact wording,
setup ticket, or Chat-owned Brain grant.

Folders and individual content support direct permanent deletion with
role-specific authority. Brain keeps only the minimum signed deletion record
needed for audit, sync, and stale-client resurrection prevention. There is no
Trash or restore workflow.

## User Stories

1. As a human user, I want my new Personal Vault to start empty, so that Brain
   does not create unwanted permanent starter content.
2. As a human user, I want the Personal Vault to remain solely owned by my User
   Nostr Identity, so that adding an agent never transfers ownership.
3. As a human user opening Brain first, I want one setup action to create my
   Personal Vault and add the selected agent, so that both of us can use the
   same Vault immediately.
4. As a human user, I want the selected agent shown by readable Managed Agent
   Email, so that I never need to identify it by an `npub`.
5. As a human user, I want user-first setup to create neither the Vault nor
   relationship when agent verification fails, so that setup cannot leave
   partial state.
6. As a human user talking to my agent first, I want to ask naturally for Brain
   setup, so that I do not need to know a slash command.
7. As a human user, I want the agent to double-check once in ordinary language
   before agent-first setup, so that the behavior feels considerate without
   becoming a security ceremony.
8. As a human user, I want a negative or unclear reply to leave Brain unchanged,
   so that setup does not proceed when I have not clearly agreed.
9. As a human user, I want the agent to resume my original request after setup,
   so that I do not need to send a separate continuation message.
10. As a human user, I want Brain to derive the Personal Vault owner from trusted
    Core and Identity facts, so that an agent cannot create a Personal Vault for
    an arbitrary person.
11. As a human user, I want exactly one Personal Agent in this phase, so that the
    Personal Vault model stays simple.
12. As a human user, I want other agents prevented from self-enrolling after my
    Vault exists, so that account-owned agents do not silently gain access.
13. As a human user, I want to replace my Personal Agent atomically by readable
    agent email, so that the old agent loses access only when the replacement is
    fully ready.
14. As a human user, I want a removed agent prevented from re-enrolling itself,
    so that revocation remains meaningful.
15. As a human user, I want a permanently deleted Core agent automatically
    removed from Brain, so that a decommissioned agent cannot retain future
    server access.
16. As a human user, I want normal agent stops and restarts to preserve Personal
    Agent access, so that routine runtime lifecycle does not disrupt Brain.
17. As a human user, I want my Personal Vault and content preserved when an
    agent is removed or deleted, so that compute teardown never purges user
    data.
18. As a Personal Agent, I want full read and write access to every current
    Personal Vault Folder, so that I can work across the user's whole Brain.
19. As a Personal Agent, I want every future Folder to grant me access
    automatically, so that the user never manages Folder-by-Folder agent
    permissions.
20. As a Personal Agent, I want to create and organize Folders and content, so
    that I can maintain the user's Brain end to end.
21. As a Personal Agent, I want to create and manage shares and invitations on
    the user's behalf, so that collaboration does not require switching
    surfaces.
22. As a Personal Agent, I want to directly delete individual content and
    complete Folder subtrees, so that I can clean and reorganize the Vault.
23. As a Personal Agent, I want deletion to require only my signed standing
    authority, so that each destructive action does not trigger another human
    approval.
24. As a Personal Agent, I want my own principal and readable email recorded in
    existing history, so that Brain never pretends the human signed my actions.
25. As a Personal Agent, I want setup retries to return the existing successful
    result, so that network retries never create duplicate Vault state.
26. As a human owner, I want every new Folder to grant both me and the current
    Personal Agent automatically, so that neither client selects cryptographic
    recipients manually.
27. As a human owner, I want owner-only Folder grants when the Personal Agent
    role is vacant, so that I can keep using Brain while choosing a replacement.
28. As a human owner, I want one clear permanent-delete confirmation in the
    Product Client, so that an accidental click does not immediately destroy a
    subtree.
29. As a human owner, I want the confirmation to show the Folder name and
    subtree size, so that I understand what will be deleted.
30. As a user deleting a Folder, I want the entire subtree deleted atomically,
    so that no orphaned Pages, Assets, nested Folders, or metadata remain.
31. As a user deleting a shared Folder, I want its invitations, share links,
    access, grants, mounts, and sync relationships cleaned up atomically, so
    that no dangling authority remains.
32. As a user deleting individual content, I want the item deleted without
    deleting its containing Folder, so that cleanup can be precise.
33. As a syncing user, I want signed deletion to win over stale offline edits,
    so that old clients cannot resurrect permanently deleted identities.
34. As a user, I want deleted content removed permanently from Brain's live
    product state with no Trash or restore flow, so that deletion has one
    understandable meaning.
35. As a user, I want Brain to be honest that deletion cannot recall downloaded
    plaintext or erase every backup, so that the product makes no false
    secure-erasure claim.
36. As a Personal Vault collaborator, I want to create and edit content within
    my write grants but not permanently delete content or Folders, so that
    destructive authority stays with the owner and Personal Agent.
37. As an Organization Vault admin, whether human- or agent-controlled, I want
    to delete individual content and Folders, so that administrators can
    maintain the organization.
38. As a non-admin Organization Vault member, I want to create and edit content
    where allowed but never permanently delete content or Folders, so that
    destructive authority remains administrative.
39. As an organization, I want multiple human and agent members or admins, so
    that multi-agent collaboration lives in Organization Vaults rather than
    complicating Personal Vaults.
40. As a user who needs broader portability, I want to share or export Personal
    Vault content, so that a single Personal Agent does not prevent
    collaboration through supported product flows.
41. As a security auditor, I want Core to remain authoritative for
    account-to-agent ownership, so that Brain cannot infer ownership from
    dashboard navigation.
42. As a security auditor, I want Finite Identity to manage the agent key in the
    protected agent environment and expose only its public identity, so that
    Brain, Core, and Chat never receive the private key.
43. As a security auditor, I want Brain to own Personal Agent authorization, so
    that identity resolution never silently becomes a cross-product grant.
44. As a hosted user, I want Finite Chat Hosted Device to remain only my
    human-key custodian and signer, so that Chat does not become the source of
    Brain agent access.
45. As a future Sites implementer, I want the Core/Identity/product-owned-role
    split documented, so that Sites can reuse the paradigm without depending on
    Brain or Chat internals.

## Implementation Decisions

- **Personal Vault Bootstrap** creates the user's single Personal Vault with the
  User Nostr Identity as sole owner and seeds no default Folders or Folder
  Objects.
- **Personal Agent** is a product role held by exactly one Agent Principal in a
  Personal Vault during this phase. It is neither an owner nor a Personal Vault
  admin.
- Personal Agent Access includes full operational and collaboration authority
  across every current and future Personal Vault Folder: read, write, create,
  organize, share, invite, and direct deletion.
- Ownership, Recovery Principal management, whole-Vault deletion or transfer,
  and control of Personal Agent removal or replacement remain
  human-owner-only.
- Agent Workspace and Folder-by-Folder agent delegation are removed from the
  active product model.
- Brain automatically issues the owner and current Personal Agent the Folder
  Key Grants required for every new Personal Vault Folder, regardless of which
  principal creates it.
- A vacant Personal Agent role results in owner-only grants until the owner
  assigns a replacement.
- User-first setup is one atomic owner-authorized operation that creates the
  empty Vault and adds the currently selected, identity-resolved agent as the
  one Personal Agent.
- Dashboard selection and prefill carry no authority. The user-first client
  displays the Managed Agent Email before confirmation, and verification
  failure writes no state.
- **Agent Bootstrap Authority** is standing authority available only to an
  authenticated account-bound agent while the user's Personal Vault does not
  exist. It may create that Vault and atomically establish the caller in the one
  Personal Agent role.
- Once a Personal Vault exists, an unpaired agent cannot self-enroll. A removed
  agent cannot use bootstrap authority to return.
- The agent never supplies the owner identity. Brain derives the owning WorkOS
  account from Core's authenticated account-agent association and resolves that
  account's existing User Nostr Identity through Finite Identity. Missing,
  ambiguous, or conflicting facts fail closed.
- Core is authoritative for WorkOS account-to-agent association.
- Finite Identity manages the Agent Principal Key inside the protected agent
  environment and is authoritative for Managed Agent Email-to-public-Agent
  Principal resolution. Its server never receives or returns the private key.
- Brain combines Core and Identity facts and is authoritative for Personal
  Agent Access.
- Finite Chat Hosted Device remains the hosted human User Nostr Identity
  custodian and signer; it does not participate in Personal Agent bootstrap or
  grant Brain access.
- Agent-first setup is atomic and idempotent. A retry by the established
  Personal Agent returns the existing result; partial failure creates neither
  Vault nor relationship; any different agent fails after the Vault exists.
- The canonical managed FiniteBrain skill owns one concise behavioral
  double-check step. With no Personal Vault, it asks once in natural language.
  A clear affirmative proceeds; a negative or unclear response leaves Brain
  unchanged and acknowledges the skip once.
- The natural-language response is behavioral guidance, not a server-enforced
  authorization boundary. There is no exact command, button, modal, or setup
  ticket.
- After successful agent-first bootstrap, the skill resumes the user's original
  task immediately.
- If a Personal Vault exists and the caller is not its Personal Agent, the skill
  explains that the owner must replace the current Personal Agent in Brain
  settings and leaves the Vault unchanged.
- Personal Agent replacement is one atomic owner-authorized operation: verify
  replacement identity, rotate every current Folder key, remove old grants,
  create replacement grants, and swap the role. Failure preserves the old
  relationship and all current Folder state.
- Permanent deletion of the underlying Core agent immediately blocks new Brain
  actions, removes the Personal Agent relationship, and rotates every current
  Folder key. Runtime restart with the same principal does not revoke.
- Personal Agent actions remain signed by the Agent Principal and use Managed
  Agent Email only as the readable actor where history is already shown. No
  extra confirmation or impersonation is introduced.
- **Direct Deletion** permanently removes the target from Brain's live product
  state. There is no Trash, undo, or restore workflow.
- Folder deletion atomically deletes the complete subtree: Pages, Assets,
  nested Folders, Folder-local metadata, and all live operational state.
- Individual Page, Asset, and other content deletion remains available without
  deleting the containing Folder.
- In Personal Vaults, only the owner and Personal Agent may directly delete
  content or Folders. Other collaborators may create and edit within their
  write scope but cannot delete.
- In Organization Vaults, direct deletion of both content and Folders is
  admin-only. Agent-controlled Member Identities may be admins. Non-admin
  members may create and edit where allowed but cannot delete.
- Human Product Client Folder deletion uses one confirmation naming the Folder,
  subtree item counts, and permanent effect. It does not require typing the
  name or a second confirmation.
- Personal Agent direct deletion needs no separate human approval beyond the
  standing Personal Agent relationship.
- Folder deletion atomically removes Folder-specific invitations, share links,
  Folder Access, Folder Key Grants, delegated scope records, mounts, and
  Working Tree sync relationships. Any cleanup failure leaves the entire live
  subtree and relationships unchanged.
- Brain retains the minimum signed deletion marker and audit metadata needed to
  propagate deletion, identify the actor, reject stale revisions, and prevent
  resurrection.
- A stale or offline client cannot upload revisions under deleted Folder or
  object identities. Intentionally recreated content receives new identities.
- Direct deletion makes no claim of erasing plaintext already downloaded by a
  client or ciphertext retained in backups, snapshots, or storage history.
- Personal Vault bootstrap, Personal Agent relationship, replacement,
  revocation, automatic grants, and direct deletion are concurrency-safe and
  fail closed on ambiguity.
- Existing unreleased development fixtures may be reset rather than migrated.
  Organization Vault bootstrap defaults remain unchanged.
- The portability specification, README, shared context, ADR status, managed
  skill, CLI reference, Product Client language, and service contracts must
  agree on the new model.

## Testing Decisions

- Tests assert external behavior and durable invariants rather than internal
  helper structure.
- The primary acceptance seam is a full local-stack smoke that exercises real
  Core, Finite Identity, Brain, hosted human identity, and Agent Runtime
  boundaries.
- The full-stack smoke covers agent-first setup from a natural request through
  automatic continuation, plus user-first setup with the selected Managed Agent
  Email.
- The Brain signed HTTP integration seam covers empty bootstrap, owner
  derivation, one-Personal-Agent uniqueness, full current and future Folder
  visibility, automatic grants, idempotent retry, competing-agent rejection,
  and rollback.
- The same Brain integration seam covers atomic replacement, Core-deletion
  revocation, retained human access, key rotation, and failure rollback.
- Direct-deletion integration tests cover owner, Personal Agent, Personal Vault
  collaborator, Organization Vault admin, and Organization Vault non-admin
  authorization matrices.
- Direct-deletion tests cover single content deletion, complete nested Folder
  subtree deletion, relationship cleanup, minimal tombstones, stale-revision
  rejection, exact retries, and transaction rollback.
- Product Client tests cover user-first setup, readable agent email,
  occupied/vacant Personal Agent state, atomic replacement UX,
  delete-menu visibility, subtree counts, and the single permanent
  confirmation.
- Managed skill tests cover one natural-language double-check, affirmative and
  negative or unclear branches, automatic continuation,
  existing-Vault/unpaired-agent guidance, and absence of the retired
  `/brain setup` dependency.
- Static skill delivery checks verify the canonical managed FiniteBrain skill
  and component reference remain synchronized without duplicating behavioral
  authority into CLI reference prose.
- Existing server router tests, Product Client JavaScript tests, Hosted Device
  HTTP tests, Hermes adapter tests, and devfinity smoke patterns are reused where
  they already provide the relevant highest seam.
- Final verification runs focused tests throughout implementation, full
  component tests, the monorepo quality gates relevant to all changed
  components, and the complete local integration smoke.

## Out of Scope

- More than one active Personal Agent in a Personal Vault.
- Agent self-enrollment after a Personal Vault exists.
- Trash, undo, restore, retention windows, or recursive recovery UX.
- Claims of physical or cryptographic erasure from client devices, backups,
  snapshots, or historical storage.
- Changing Organization Vault membership or admin semantics beyond deletion
  authorization and confirmation that agent-controlled identities may be
  admins.
- Implementing the corresponding Personal Agent paradigm in Finite Sites during
  this work.
- Native application key custody or client-generated user-key delivery.
- Legacy released Personal Vault migration. This is a Greenfield hard cut and
  local development fixtures may be reset.
- Giving the Personal Agent the human's private key, Brain Identity Provider,
  ownership role, recovery control, or whole-Vault deletion authority.
- Replacing the existing human User Nostr Identity setup prerequisite in Finite
  Chat Hosted Device.

## Further Notes

- Accepted domain decisions are recorded in the FiniteBrain glossary and ADRs
  0021 through 0024. ADR-0020 is superseded.
- The one-Personal-Agent limit is a deliberate scope reduction. Users who need
  multiple agents can use Organization Vaults, sharing, or export.
- The natural-language check is intentionally loose agent behavior. Brain's
  actual security boundary is verified account-agent ownership plus the rule
  that Agent Bootstrap Authority exists only before the Personal Vault is
  created.
- The implementation must preserve the recoverability-first invariant: agent
  removal, replacement, runtime teardown, and Folder deletion must never delete
  the human-owned Personal Vault itself.
