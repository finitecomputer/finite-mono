# ADR 0027: Daemon-local email proofs and Sites authorization

## Status

Accepted. Email proofs and authorization are owned by Sites. Requests are
authorized against Sites state; directory identity is not permission authority.

## Mailbox proofs and keys

`finitesitesd` issues 15-minute, single-use, hash-stored email challenge tokens
in `email_login_tokens`, delivered by its configured `Mailer`.

- `fsite auth login`, `auth link-email` and `auth sites-key request` request
  challenges at `POST /api/v2/email-auth/request`.
- `fsite auth redeem` redeems at `POST /api/v2/email-auth/redeem`, recording the
  mailbox-scoped Email Key and, when explicitly requested for a registered
  native Principal, an Email Link.
- Sites Authorized Key registration/revocation carries `{email, token}` and
  consumes the local proof atomically with the mutation.

`sites_email_principals` records a durable Sites owner named by a verified
mailbox. `sites_authorized_keys` records revocable native keys, proof provenance
and revocation time. Projects and Sites retain their originating native
publisher for audit and may additionally name a mailbox publisher. An active
key may exercise that mailbox owner's Sites permissions.

Revoking one key leaves other keys, email/native shares, URLs, visibility,
project collaborators and other Principals' Git credentials intact. A revoked
key is a durable tombstone; automated evidence must never reactivate it. Only
a fresh mailbox proof can do so. Ambiguous evidence fails closed.

## Principal links and Git credentials

An Email Link asserts that an email and native key identify the same Principal.
It requires explicit proof, including `fsite auth redeem --link-native`; never
link a human mailbox to an agent merely to grant that agent access. Authorized
Sites Keys provide revocable product-scoped access without making identities
equal and without granting Chat or Brain authority.

Future email collaborator grants resolve through an active Email Link.
Linking moves active email collaborator grants to the native Principal and
revokes old email-scoped Git credentials; replay is idempotent. Git-auth checks
local Email Links, Email Keys and Sites Authorized Keys. `fsite auth git --email`
tries the Finite Home's native key first, then the mailbox-scoped Email Key on
403. Private keys never leave the client's Finite Home.

## Service dependencies

Sites sends first-publication and access-request mail through its own mailer
and durable notification outbox. The only finite-identity dependency is NIP-05
directory lookup. Directory name claiming is independent of Sites email proofs.
The account viewer exchange is defined in
[ADR 0029](0029-account-session-viewer-bridge.md); authentication never creates
a Share.
