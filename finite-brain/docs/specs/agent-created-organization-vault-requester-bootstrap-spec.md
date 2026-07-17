# Agent-Created Organization Vault Requester Bootstrap

## Problem Statement

When an authenticated human asks their agent to create an Organization Vault,
FiniteBrain currently makes only the acting Agent Principal an initial member
and admin. The agent must then add the human as a member and promote them to
admin through separate commands. This can leave the human unable to access a
Vault created on their behalf, asks the human to provide identity information
Brain already has in the authenticated chat event, and permits partial failure
between Vault creation and human access.

From the user's perspective, saying “create an Organization Vault” should
produce a Vault that both the user and their agent can administer immediately.
The user should not need to follow with “add me,” provide an email address, or
understand the underlying Nostr identities.

## Solution

An agent-created Organization Vault accepts an **Organization Vault
Requester**: the User Nostr Identity taken from the authenticated human's direct
chat request. The managed FiniteBrain skill passes that public identity into
the Organization Vault creation operation. Brain atomically creates the Vault
with both the acting Agent Principal and Organization Vault Requester as
members and admins and issues both identities the required initial Folder Key
Grants.

If the complete bootstrap cannot succeed, Brain creates no Vault. If the
managed skill cannot obtain authenticated requester metadata, it does not
guess, ask for an email address, or create an agent-only Vault; it briefly asks
the user to retry from an authenticated chat context. A clear natural-language
request to create the Organization Vault is sufficient authorization and does
not trigger an additional confirmation.

Organization Vaults created directly by a human through the Product Client are
unchanged: the signing human starts as the initial admin, and Brain does not
automatically add their Personal Agent.

## User Stories

1. As an authenticated human, I want my agent to create an Organization Vault
   with me already included, so that I can open it immediately.
2. As an authenticated human, I want to become an initial Vault admin, so that
   I can manage membership, access, and content without asking the creating
   agent for another change.
3. As an authenticated human, I want the creating agent to remain an initial
   Vault admin, so that it can continue operating the Vault on my behalf.
4. As an authenticated human, I want my existing authenticated User Nostr
   Identity used automatically, so that I do not provide an email address,
   `npub`, or hex key in conversation.
5. As an authenticated human, I want a natural-language creation request to be
   sufficient authorization, so that the flow has no redundant confirmation
   ceremony.
6. As an authenticated human, I want the agent to report that both of us are
   admins, so that the successful access state is clear.
7. As an authenticated human, I want the Vault and my access created together,
   so that a partial failure cannot strand me outside a Vault created for me.
8. As an authenticated human, I want failed bootstrap to leave no Vault, so
   that retrying starts from a clean state.
9. As an agent, I want the authenticated sender identity exposed as the
   Organization Vault Requester, so that I do not infer who requested the
   Vault.
10. As an agent, I want one Organization Vault creation operation to establish
    both admins, so that I do not sequence separate membership and promotion
    commands.
11. As an agent, I want missing authenticated sender metadata to stop this
    flow, so that I never guess which human should receive administration.
12. As an agent, I want to resume the user's broader task after successful
    creation, so that creating the Vault is a prerequisite rather than the end
    of the conversation.
13. As a Vault administrator, I want both initial admins to receive the
    required Folder Key Grants, so that membership metadata never claims access
    an identity cannot cryptographically exercise.
14. As a human creating an Organization Vault in the Product Client, I want the
    existing flow to remain unchanged, so that my Personal Agent is not added
    without an explicit action.
15. As a security reviewer, I want the requester taken only from authenticated
    message metadata, so that quoted or typed identity text cannot redirect the
    bootstrap.
16. As a security reviewer, I want the creator and requester recorded as their
    distinct Member Identities, so that Brain preserves authorization and
    attribution instead of impersonating the human.
17. As an operator, I want the operation to preserve Brain's controller-kind
    agnosticism, so that the server continues authorizing Nostr Member
    Identities rather than classifying callers as humans or agents.
18. As a developer, I want existing Organization Vault creation without a
    requester to remain available to the Product Client, so that this narrow
    agent flow does not change unrelated creation paths.
19. As a developer, I want invalid, missing, conflicting, or identical
    requester identity input in the agent bootstrap path to fail before durable
    state changes, so that the promised two-principal result is explicit.
20. As a developer, I want both managed FiniteBrain skill copies to express the
    same requester behavior, so that packaged agents do not drift from the
    canonical component guidance.

## Implementation Decisions

- The Organization Vault creation contract gains an optional requesting User
  Nostr Identity intended for authenticated agent-on-behalf-of-human creation.
  Its JSON representation is `requestingUserNpub`; the agent CLI exposes it as
  `--requesting-user-npub`.
- The CLI accepts the authenticated public-key account identifier unchanged and
  normalizes it to the canonical User Nostr Identity. It does not accept or
  resolve an email address for this option.
- Supplying a requester is valid only for Organization Vault creation. Personal
  Vault creation rejects it because Personal Agent bootstrap has its own
  account-bound authority model.
- When a requester is supplied, it must identify a valid Member Identity that
  differs from the signing creator. Invalid or identical input fails before
  durable Vault state is written.
- Organization Vault bootstrap produces both the signing creator and requester
  as initial members and admins. Every Vault admin remains a Vault member.
- Required initial Folder Key Grants include both identities for every default
  Organization Vault Folder they must open. Client-generated encrypted grants
  are validated for completeness before storage.
- Vault metadata, memberships, admin roles, initial folders, and all required
  grants are committed as one store operation. Any validation, encryption,
  grant, or persistence failure leaves no new Vault or partial relationships.
- Brain enforces the atomic two-principal result when a requester is supplied.
  The managed skill is responsible for selecting the requester from the
  authenticated message's sender metadata because Brain remains agnostic to
  whether a Member Identity is controlled by a human or agent.
- The managed FiniteBrain skill passes authenticated sender metadata unchanged
  for direct human requests. It never substitutes typed text, an email address,
  profile data, the Agent Principal, or another inferred identity.
- With no authenticated sender metadata, the managed skill does not call the
  agent-created Organization Vault flow and briefly asks the user to retry from
  an authenticated chat.
- A successful natural-language request requires no second confirmation. The
  agent reports the Vault name and that the requester and agent are admins,
  then continues the user's original task when applicable.
- Product Client Organization Vault creation omits the requester and preserves
  its current single-signing-admin bootstrap. It does not automatically enroll
  the user's Personal Agent.
- This decision implements ADR-0025 and remains consistent with ADR-0016: the
  authorization model stores and evaluates Member Identities, not controller
  kinds.

## Testing Decisions

- The primary behavior test is the existing signed Brain HTTP integration seam.
  A request signed by an Agent Principal and carrying a distinct Organization
  Vault Requester must return a Vault in which both identities are members and
  admins and both can exercise their initial Folder access.
- The integration test observes externally visible authorization and access,
  not internal helper calls or SQL layout.
- The same seam verifies atomic failure by submitting an invalid or incomplete
  requester bootstrap and then proving the Vault does not exist.
- The signed HTTP suite verifies that requester input is rejected for Personal
  Vault creation and that ordinary Product Client-style Organization Vault
  creation without requester input retains its current behavior.
- Existing core and store tests may provide focused coverage for bootstrap
  invariants and rollback, but they support rather than replace the signed HTTP
  acceptance seam.
- Existing CLI request-capture tests verify that
  `--requesting-user-npub` produces the correct signed creation request and
  complete encrypted grant set.
- Existing managed-skill static checks verify that the canonical component skill
  and packaged managed skill remain synchronized, use authenticated sender
  metadata, avoid identity guessing, and stop when that metadata is unavailable.
- A good test proves what each principal can observe and do after success, and
  proves the absence of durable Vault state after failure. It does not assert
  private function names or an exact natural-language sentence.

## Out of Scope

- Changing Personal Vault ownership, Personal Agent bootstrap, or the one
  Personal Agent limit.
- Automatically adding a Personal Agent when a human creates an Organization
  Vault in the Product Client.
- Adding other humans, room participants, agents, or account members beyond the
  authenticated requester and acting creator.
- Resolving the requester from email, NIP-05, quoted text, profiles, WorkOS
  account guesses, or conversational content.
- Giving agents an inbox or changing their Managed Agent Email behavior.
- Changing Organization Vault member/admin permissions after bootstrap.
- Adding new confirmation controls, slash commands, setup tickets, buttons, or
  modals.
- Supporting unauthenticated, cron, background, or system-triggered creation on
  behalf of an inferred human.
- Changing Organization Vault deletion, invitation, sharing, export, or
  recovery semantics.
- Changing the Product Client Organization Vault creation experience.

## Further Notes

- The bug is not that Organization Vaults lack multi-admin support. That support
  already exists. The gap is that agent-created Vault bootstrap currently
  establishes only the signing agent, while human membership and promotion are
  later mutations.
- The Finite Sites managed skill already demonstrates the same identity-source
  pattern by passing the authenticated chat sender's public account identifier
  into a creation operation. Brain should reuse that product convention.
- Email remains the readable identity in user-facing selection and display
  surfaces, but the authoritative requester for this bootstrap is the
  authenticated User Nostr Identity.
