# Atomically Add Requesters To Agent-Created Organization Vaults

Status: accepted 2026-07-17.

When an authenticated human directly asks an agent to create an Organization
Vault, the acting Agent Principal and the requesting User Nostr Identity become
initial Vault members and admins. Brain creates both memberships, both admin
roles, and the Vault in the same atomic bootstrap. Under ADR-0021, the new
Vault has no Folders, Folder Keys, or Folder Key Grants; those appear only when
an admin explicitly creates a Folder. If any bootstrap part fails, no Vault is
created.

The managed FiniteBrain skill obtains the requester from authenticated message
metadata and passes that public-key account identifier unchanged into the
Organization Vault creation operation. It never derives the requester from an
email address, quoted text, profile content, or another identity typed into the
conversation. A clear natural-language request to create the Vault is sufficient
authorization; the skill does not add another confirmation step.

Brain remains controller-kind agnostic as required by ADR-0016. The server can
validate and atomically grant the supplied requesting Member Identity, but it
does not classify the signing creator as an agent or cryptographically prove
the provenance of chat metadata. Selecting the authenticated requester is the
managed skill's responsibility. This does not expand the creator's Organization
Vault authority: the creating admin could already add another member and admin
after creation. The new operation makes that intended result atomic.

If authenticated requester metadata is unavailable, the managed skill does not
guess and does not create an agent-only Organization Vault. It briefly asks the
user to retry from an authenticated chat context. After successful creation, it
reports that both the requester and agent are admins.

This decision applies only to an agent creating an Organization Vault on an
authenticated human's direct request. A human creating an Organization Vault
in the Product Client remains the signing initial admin and does not
automatically enroll a Personal Agent or another agent.

Rejected shapes:

- Creating the Vault for the agent first and then running separate add-member
  and add-admin commands for the human, because failures can leave the human
  without access.
- Asking the human for an email address or public key when authenticated sender
  metadata already identifies the requester.
- Guessing a requester or silently creating an agent-only Vault when
  authenticated requester metadata is absent.
- Automatically adding a Personal Agent to Organization Vaults created directly
  by a human in the Product Client.
