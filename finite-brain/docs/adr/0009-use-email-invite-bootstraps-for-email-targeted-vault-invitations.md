# Use Email Invite Bootstraps For Email-Targeted Vault Invitations

Email-targeted Vault Invitations extend the existing Brain invitation flow
rather than adding a separate permission system. The inviting admin's trusted
client creates NIP-59-shaped gift-wrapped Email Invite Bootstrap material from
already-open Folder Keys and addresses it to a temporary Invite Unwrap Key. The
server stores only the gift-wrapped ciphertext. The Invite Secret in the URL
fragment carries the client-only material needed to use the Invite Unwrap Key,
and Identity Authority email proof is still required before the recipient can
claim the invitation.

The Invite Unwrap Key is a bearer unwrap capability, not a User identity,
member identity, or permission principal. This reuses Nostr's NIP-59/NIP-44
envelope shape without pretending the invited email already has its final User
npub.

The Invite Secret must remain client-only. Brain carries it in the URL fragment
or an equivalent client-only channel; it must not appear in server-visible query
parameters, request bodies, stored database fields, server logs, email tracking
links, analytics redirects, server-side mailer payloads, or email bodies. If an
invite delivery mechanism cannot preserve that boundary, it must deliver only
the server-visible invite code and require the Invite Secret through a separate
client-only channel.

Accepted email invitations become durable npub-bound access through an Email
Invite Bootstrap Claim. This keeps the Brain server blind to Folder Keys,
preserves the Sites-style agent-actionable invite experience with
invite-scoped instructions, and avoids treating email targets as direct
permission mutations in v1. When the target is already a concrete User npub,
hex public key, or an active Finite VIP NIP-05 binding, Brain uses the normal
npub-bound invitation and grant path. External email-shaped identifiers always
use Email Invite Bootstraps in v1, even if the email has prior email-only proof;
third-party NIP-05 resolution and canonical external email-to-npub lookup remain
out of scope.

Claim-authorized Folder Key Grants are a first-class Brain validation path, not
normal admin-issued grants with a different signer. The server accepts a
recipient-created durable grant only when it is backed by a pending admin-signed
Email Invite Bootstrap Authorization. That authorization fixes the canonical
invited email, Vault, authorized Folder scope, Folder key versions, Invite
Unwrap public key or hash, bootstrap payload hash, expiry, and single-use claim
bounds. A later claim is valid only when it also has matching email proof
created after the invitation and no more than 24 hours before claim, an
authenticated recipient npub, authorized Vault/Folder/key-version scope,
unexpired status, Invite Secret possession, and single-use claim semantics.
Invite Secret possession is proven without revealing the secret by requiring an
Invite Unwrap Proof: a Nostr event signed by the temporary Invite Unwrap Key
that binds the claim to the invite code, Vault, invited email, claimant npub,
bootstrap payload hash, and email proof timestamp. A claimant who proves the
email but cannot sign with the Invite Unwrap Key must not be able to consume the
bootstrap or become a Vault Member from that invite.

Email Invite Bootstrap Claim is atomic. A successful claim verifies email proof,
verifies Invite Unwrap Proof, consumes the pending bootstrap, binds the
recipient npub, creates the membership/access metadata, and inserts every
durable Folder Key Grant required for the invited scope. If any required grant
cannot be created or validated, the claim fails without partially accepting the
invitation.

Email-targeted Vault Invitations include current all-members Folders in the
bootstrap scope automatically, because a successful claim makes the recipient a
Vault Member and existing Brain semantics require all Vault Members and admins
to receive all-members Folder access. The Email Invite Bootstrap Authorization
records the exact all-members Folder ids and key versions present at invite
creation. New all-members Folders created later are not silently added to an
already pending bootstrap; they follow the normal late-member/current-grant
workflow after claim or require a fresh invitation if access at claim time must
include them.

Pending Email Invite Bootstraps are invalidated by any relevant Folder Key
Rotation before claim. Claim checks every authorized Folder's current key
version against the bootstrap payload; a mismatch fails closed and requires a
new invitation/bootstrap payload.

Brain intentionally does not copy Sites' public `llms.txt` behavior wholesale.
Sites can expose project-editing metadata publicly because that metadata does
not unlock the Git repository; Brain invite metadata can reveal private Vault
structure and access intent. Brain therefore serves two levels of Invite
Instructions rather than encrypting private instructions inside public
`llms.txt`: Public Invite Instructions are unauthenticated and generic, while
Post-Proof Invite Instructions are returned only from an authenticated surface
after Identity Authority proves control of the invited email and the requester
is acting as the claiming User npub. Public instructions may point an agent to
the email-proof flow and warn it not to expose URL-fragment Invite Secrets, but
they must not include invited email, Vault or Folder names/ids, access scope,
claim state, Folder Keys, bootstrap plaintext, or any material that would reveal
the encrypted invite's private structure. Post-Proof Invite Instructions may
include the scoped claim/open/sync workflow details needed by the human or
agent, including human-readable Vault and Folder names. Names are allowed after
proof because the requester has proven the exact invited email and is acting as
the claiming User npub. Post-Proof Invite Instructions still must not include
Folder Keys or decrypted bootstrap payloads. The gift-wrapped bootstrap remains
the only carrier for encrypted access material, and only the recipient client
uses the Invite Secret to unwrap it.

In v1, Brain owns product-specific Vault invite delivery while finite-identity
owns email proof and challenge delivery. Brain may use a Sites-like mailer
adapter for Resend, Postmark, and dev-mail, and invite creation must still
return the invite URL so local, dev, and manually delivered invites work without
a configured mailer. A future shared delivery abstraction in finite-identity is
allowed, but it should not make finite-identity responsible for Brain-specific
Vault invitation content or access policy.

Brain does not retain gift-wrapped bootstrap ciphertext after an invitation
reaches a terminal state. Accepted, revoked, expired, rotation-invalidated, and
superseded Email Invite Bootstraps delete or tombstone the encrypted bootstrap
payload. Brain may keep minimal audit metadata such as invite id, Vault id,
canonical invited email hash, status, and created/revoked/claimed/expired
timestamps, but it must not keep the gift-wrapped bootstrap ciphertext after the
payload can no longer be used.

Email Invite Bootstrap Claim is single-use and idempotent for the same claiming
User npub. If a claim commits but the client loses the response, the same User
npub may retry and receive the accepted result. A different User npub attempting
to claim an already claimed invite is rejected. This is a narrow reliability
rule for one-time acceptance, not a separate idempotency subsystem.

Current implementation note: Brain can verify current email control by calling
finite-identity Principal Resolution (`satisfies-grant`) when
`FINITE_IDENTITY_AUTHORITY` is configured. That endpoint does not yet return a
dated email-proof assertion, so Brain still validates the submitted
`emailProofCreatedAt` freshness window itself. Full authority-backed dated
proof verification remains a follow-up once finite-identity exposes that
contract.
