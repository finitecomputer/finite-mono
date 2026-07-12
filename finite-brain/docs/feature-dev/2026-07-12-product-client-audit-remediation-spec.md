# Spec: FiniteBrain Product Client Audit Remediation

GitHub issue: #17

## Problem Statement

The Product Client's primary trusted-member workflow has several broken or
misleading controls discovered through a full local browser audit. A Member
Identity can edit a Page but cannot persist its signed revision, cannot write a
signed tombstone through the visible delete action, can be left with an
unlocked-looking session after authorization is revoked, and is not reliably
shown errors. The invitation surface also exposes actions while the Session is
locked even though its protected operation must fail closed. Finally, one
Folder action contradicts the product's hierarchy model and the Graph View has
an undiscoverable pseudo-search affordance.

## Solution

Repair those workflows while preserving the Product Client's existing trusted
client, NIP-07, Session Lock, Folder Key Grant, and encrypted-object interfaces.
Every user-visible failure should have safe, actionable feedback; operations
that need an unlocked session should be gated before they run; and labels should
match the actual Vault, Folder, Page, and Graph behavior.

## User Stories

1. As a Member Identity with an unlocked Session, I want Save to create a
   signed encrypted Page revision so that my edit persists securely.
2. As a Member Identity, I want Delete Page to create a signed tombstone so
   that deletion preserves the encrypted-object history model.
3. As a user, I want a clear in-product error when an action fails so that I
   know what happened and what to do next without opening developer tools.
4. As a user whose Vault access was revoked, I want the client to fail closed
   and show a locked Session rather than claim that encrypted content remains
   open.
5. As a locked user, I want Invitations to direct me to Unlock before inspect
   or accept actions are available so that the interface matches Session Lock
   semantics.
6. As a Vault administrator, I want to revoke an invitation I created without
   trying to inspect a recipient-only invitation code.
7. As a user, I want a pasted invitation code to update the available actions
   immediately and remain present until I deliberately change it.
8. As a Vault administrator, I want New Folder Inside to create a real Child
   Folder with the chosen parent while retaining its own Folder Key and access
   boundary.
9. As a Graph View user, I want controls to be visible and honestly labelled,
   so that I do not mistake a hidden filter for an unexplained search icon.

## Implementation Decisions

- Reuse the existing session-aware NIP-07 signing adapter for Page revision and
  Page tombstone requests. Do not add durable browser state or bypass the
  signed secure-object route.
- Surface safe generic client failures through one visible, accessible status
  region. It must not reveal Invite Secrets, Folder Keys, or decrypted content
  outside the existing Session boundary.
- Treat a confirmed authorization failure for the active Vault's protected
  metadata/content workflow as a Session Lock event. Preserve a safe notice
  after clearing Session Folder Keys and Ephemeral Client Plaintext. Do not
  relock for an ordinary unavailable-network failure or an unrelated
  administrator-only endpoint.
- Keep the locked Settings surface limited to safe status and Unlock/Resume.
  Do not relax the session-epoch guard for normal npub invitation inspection or
  acceptance. Email bootstrap claims remain unavailable until their required
  unlocked key-opening flow.
- Keep normal invitation-code inspection recipient-only. An administrator
  revokes through an already-known invitation identifier, such as the created
  invitation or a pending invitation row.
- A Child Folder remains an independent Folder scope. New Folder Inside passes
  the parent hierarchy metadata but keeps the established creation defaults;
  it does not silently inherit a restricted parent's access recipients.
- Remove the hidden Graph title-filter control and its event binding rather
  than preserving an inaccessible pseudo-control. Graph data remains derived
  only from the decrypted, accessible local index.

## Testing Decisions

- The public behavior seams are the existing deterministic Product Client
  contract suite and the real Rust-served `/client` flow on an isolated local
  Vault with disposable Member Identities. These are the previously accepted
  Product Client seams, and the human explicitly requested end-to-end repair.
- Contract coverage should prove signed Page write/tombstone preparation,
  parent-aware Folder creation inputs, invitation action availability, safe
  error/status rendering, and Session Lock behavior after Vault authorization
  loss without depending on private rendering details.
- Browser verification should prove Save, Delete Page, locked invitation
  guidance, admin revoke, nested Folder creation, active-Vault authorization
  loss handling, and the corrected Graph header at realistic desktop and
  narrow widths.

## Out of Scope

- New backend routes, schema changes, cryptographic formats, durable browser
  storage, Folder access inheritance, Folder deletion, or changed invitation
  authorization policy.
- Changes to Recovery Principals, recovery artifacts, or production
  configuration/deployment.
- Email proof-provider delivery integration, which requires separately
  configured external authority and mail seams.

## Further Notes

This remediation follows the existing Product Client, Graph View, Folder,
Member Identity, Session Lock, Session Folder Key, and Ephemeral Client
Plaintext terminology. It is constrained by accepted ADRs for the first-party
Product Client, client-derived graph, session-only keys, Session Lock, and
Member Identity authorization.
