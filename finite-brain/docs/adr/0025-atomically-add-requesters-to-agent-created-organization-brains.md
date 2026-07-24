# Atomically Add Requesters To Agent-Created Organization Brains

Status: accepted 2026-07-17.

When an authenticated human directly asks an agent to create an Organization
Brain, the acting Agent Principal and the requesting User Nostr Identity become
initial Brain members and admins. Brain creates both memberships, both admin
roles, and the Brain in the same atomic bootstrap. Under ADR-0021, the new
Brain has no Folders, Folder Keys, or Folder Key Grants; those appear only when
an admin explicitly creates a Folder. If any bootstrap part fails, no Brain is
created.

The managed FiniteBrain skill obtains the requester from authenticated message
metadata and passes that public-key account identifier unchanged into the
Organization Brain creation operation. It never derives the requester from an
email address, quoted text, profile content, or another identity typed into the
conversation. A clear natural-language request to create the Brain is sufficient
authorization; the skill does not add another confirmation step.

Brain remains controller-kind agnostic as required by ADR-0016. The server can
validate and atomically grant the supplied requesting Member Identity, but it
does not classify the signing creator as an agent or cryptographically prove
the provenance of chat metadata. Selecting the authenticated requester is the
managed skill's responsibility. This does not expand the creator's Organization
Brain authority: the creating admin could already add another member and admin
after creation. The new operation makes that intended result atomic.

If authenticated requester metadata is unavailable, the managed skill does not
guess and does not create an agent-only Organization Brain. It briefly asks the
user to retry from an authenticated chat context. After successful creation, it
reports that both the requester and agent are admins.

This decision applies to an agent creating an Organization Brain on an
authenticated human's direct request. ADR-0026 separately governs the reverse
Product Client path, where the human may atomically include the selected agent
through a visible default-on choice.

Rejected shapes:

- Creating the Brain for the agent first and then running separate add-member
  and add-admin commands for the human, because failures can leave the human
  without access.
- Asking the human for an email address or public key when authenticated sender
  metadata already identifies the requester.
- Guessing a requester or silently creating an agent-only Brain when
  authenticated requester metadata is absent.
- Silently or unconditionally adding an agent to Organization Brains created
  directly by a human in the Product Client. ADR-0026 instead requires a
  visible choice that the human may turn off.
