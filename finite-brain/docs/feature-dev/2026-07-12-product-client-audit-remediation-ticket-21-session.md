## Issue

- Issue: #21 — make Vault invitation actions match Session Lock
- Fixed point before session: `fe577fc`
- Worker session: `/root/ticket_21_invitations`
- Commit: `dfc8a0b` + final integration correction `4909693`
- Status: complete; final shared browser verification passed

## Inputs

- Spec issue: #17
- Ticket: #21
- Relevant glossary terms: Vault invitation, Invite Code, Invite Secret, Member
  Identity, Session Lock, Session Folder Key, Ephemeral Client Plaintext
- Relevant ADRs: 0004, 0009, 0010, 0013, 0014, 0016
- Product truth: normal invitation inspection remains recipient-bound; an admin
  revokes an already-known admin-side invitation identifier

## Implementation

- Public interface used: Vault invitation panel and pending invitation list
- Behaviors covered: locked Session shows safe unlock guidance and disables
  protected invitation actions; every protected handler, including pending-list
  Revoke, fails closed before capturing a session epoch; code/email/proof/secret
  inputs react immediately without promoting manually pasted Invite Secrets to
  state; changing a code invalidates the prior admin association and email
  proof; top-level revoke uses only explicit ID, just-created ID, or loaded
  pending admin-row ID
- Boundary preserved: normal inspect/accept still call the recipient-bound
  route, but the admin revoke resolver never calls it
- Final integration correction: accepting a normal or email Vault invitation
  now renders the reset Session immediately, so stale unlocked chrome cannot
  hide the required safe unlock notice after the active Vault changes
- `tdd` used: yes; deterministic panel/seam tests cover controls, guards,
  input lifecycle, binding, resolver paths, and secret handling
- Commands run during implementation:
  - `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
  - `git diff --check`
- Focused Rust asset test: deferred failure is the pre-existing stale
  `graph-icon-button` assertion; #23 owns the Graph asset update

## Review

- Review fixed point: `fe577fc`
- Independent focused review: pass; no P0–P3 findings
- Review specifically verified: every direct protected action guards before
  epoch capture; the revoke resolver does not inspect recipient links; manual
  Invite Secret input remains DOM/session-only
- Final browser proof: locked controls stayed disabled; an admin created an
  invite, a recipient accepted it through the visible panel and immediately
  saw the locked Session/Unlock guidance, then unlocked the invited Vault.
  The same disposable flow confirmed the admin-side revoke path remains
  available without recipient-link inspection.

## Risks

- Invitation input state intentionally remains ephemeral. A future UX request
  to retain pasted Invite Secrets beyond a Session must be separately designed
  against the client-only secret and Session Lock constraints.
