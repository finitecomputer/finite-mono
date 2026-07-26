# Atomically Add Requesters To Agent-Created Organization Brains

Status: accepted 2026-07-17.

When an authenticated human directly asks an agent to create an Organization
Brain, the acting Agent Principal and the requesting User Nostr Identity become
initial Brain members and admins. Brain creates both memberships, both admin
roles, and the Brain in the same atomic bootstrap. Under ADR-0021, the new
Brain has no Folders, Folder Keys, or Folder Key Grants; those appear only when
an admin explicitly creates a Folder. If any bootstrap part fails, no Brain is
created.

The Agent Runtime obtains the requester from authenticated message metadata and
writes a short-lived, task-local lease around the CLI call. The CLI uses that
lease as a transport hint for the requester carried in the signed Organization
Brain creation operation. The lease and child-process environment are not an
authorization boundary: the Brain server independently classifies the signed
creator through the configured Finite Identity and Core operator authorities.
When the signer is a Managed Agent Principal, the server accepts a requester
only when it exactly matches that Agent's server-resolved account owner.

The normal CLI command does not require the agent to transcribe a raw requester
identity and exposes no advanced requester-identity override. It never derives
the requester from an email address, quoted text, profile content, or another
identity typed into the conversation. A clear natural-language request to
create the Brain is sufficient authorization; the skill does not add another
confirmation step.

Brain's durable model remains controller-kind agnostic as required by ADR-0016:
it stores Member Identities and roles rather than a special Agent membership
kind. At the creation trust boundary, however, the server must distinguish an
active Managed Agent Principal from a direct human signer so that a local
caller cannot omit or forge the initial requesting human. This classification
is an admission check backed by operator-only Finite Identity and Core
resolution; it does not add controller kinds to Brain state.

For a Managed Agent signer, missing authority resolution, missing requester
context, or a requester that differs from the server-resolved account owner
fails closed before the Brain transaction. A signer that does not resolve as a
Managed Agent follows the direct-human path and cannot supply a separate
requester. A server without the operator authority configuration rejects every
requester-bearing create instead of trusting the request body. This does not
expand the creator's Organization Brain authority: an admin could already add
another member and admin after creation. The new operation makes the intended
initial result atomic while authenticating who may occupy that second position.

If authenticated requester metadata is unavailable in an Agent Runtime, the
managed skill does not guess and does not create an agent-only Organization
Brain. It briefly asks the user to retry from an authenticated chat context.
When a human runs the CLI directly, the signing human remains the sole initial
admin. After successful agent creation, the skill reports that both the
requester and agent are admins.

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
- Retaining a raw requester-identity CLI flag, because it preserves the
  transcription burden and permits the agent to substitute conversational
  identity input for authenticated Runtime context.
- Guessing a requester or silently creating an agent-only Brain when
  authenticated requester metadata is absent.
- Silently or unconditionally adding an agent to Organization Brains created
  directly by a human in the Product Client. ADR-0026 instead requires a
  visible choice that the human may turn off.
