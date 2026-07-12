## Issue

- Issue: #27 — make clipboard and invitation handoff trustworthy
- Fixed point before session: `8a58f70`
- Worker session: `/root/ticket_27_clipboard_invitation_feedback`
- Status: complete

## Scope

- Give Product Client clipboard actions truthful, generic success/failure
  feedback without echoing copied values.
- Make a successful email-invite URL locally visible and explicitly copyable
  only while the trusted session is unlocked.
- Keep manually entered Invite Secrets masked; clear all client-only invite
  material on Session Lock.

## Constraints

- Do not write copied values, Invite Secrets, raw Folder Keys, or decrypted
  content to browser storage, logs, history, or documentation.
- Do not touch `finite-brain/docs/research/`.
- Invitation lifecycle actions such as accept and revoke remain explicit;
  keyboard submission mapping belongs to ticket #28.

## Implementation

- Routed Page ID, Folder ID, and generated invite-link copy actions through one
  asynchronous client helper with generic, accessible success and failure
  feedback. The helper neither logs nor displays copied material or raw
  clipboard failures.
- Made a generated email-invite URL readable and copyable only during an
  unlocked trusted session. The output and its copy action are hidden or
  disabled after Session Lock, which also clears the local URL and secret.
- Kept manually entered Invite Secrets in the existing masked password field.
- Added deterministic Product Client seams covering clipboard success,
  rejection, unavailable clipboard support, context-menu ID copies, generated
  invite visibility/copy, and Session Lock clearing.

## Verification

- Passed deterministic Product Client tests.
- Passed JavaScript syntax checks for the Product Client and static verifier.
- Passed `cargo fmt --check` and the targeted server asset-serving test.
- Passed the locked finite-brain app build.

## Limits

- The full static smoke verifier requires a disposable smoke Folder-key
  manifest that was not present in this workspace, so it was not run to
  completion.
- Isolated browser proof was intentionally deferred to the coordinating
  session after the static-smoke prerequisite was found missing. No copied
  values, invite material, or user data were recorded during this work.
