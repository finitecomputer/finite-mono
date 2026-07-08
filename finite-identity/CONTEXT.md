# Finite Identity

Finite Identity owns the shared identity language for Finite tools and products. It defines how Finite-controlled email names, Nostr keys, NIP-05 names, and product principals relate to each other.

## Language

**Finite VIP Email**:
A Finite-controlled email address on `finite.vip` that is also the canonical user-facing identity string. Its full form is `localpart@finite.vip`.
_Avoid_: finite-vip email, account email, VIP address

**Finite VIP Domain**:
The `finite.vip` domain that hosts Finite VIP Emails and their matching NIP-05 names.
_Avoid_: finite-vip, VIP host

**NIP-05 Name**:
The public Nostr name for a Finite identity. In v1 it is always identical to the user's Finite VIP Email, such as `localpart@finite.vip`.
_Avoid_: handle, username, nostr email

**Third-Party NIP-05 Name**:
A NIP-05 identifier on a domain not owned by Finite. Third-Party NIP-05 Names are future work and are not trusted as product grantees in v1.
_Avoid_: external handle, external nostr address

**NIP-05 Endpoint**:
The public `.well-known/nostr.json` HTTP endpoint for the Finite VIP Domain. In v1, the Identity Authority owns the response for this endpoint.
_Avoid_: static nostr file, nostr profile endpoint

**Recovery**:
The unresolved future process for regaining or moving control of a Finite identity after key loss. Recovery is not defined in v1 because it also decides how product data follows or does not follow a replacement key.
_Avoid_: reset, relink, account restore

**Disabled Binding**:
A Finite VIP Email or NIP-05 Name binding that the Identity Authority keeps for audit history but no longer serves or resolves. Disabling a binding is an operator safety action, not Recovery or reassignment.
_Avoid_: deleted binding, reset binding, transferred binding

**Principal**:
The identity subject that Finite products attach permissions to. A Principal is either a Native Principal or an Email-Only Principal.
_Avoid_: account, user, member

**Native Principal**:
A Principal backed by a Nostr public key controlled by a Finite identity keypair.
_Avoid_: native account, npub user

**Email-Only Principal**:
A Principal backed by verified control of an email address before the person has linked that email to a Native Principal.
_Avoid_: guest user, invited user, external account

**Invited Email**:
Any email address that a Finite product can grant access to before the recipient has a Native Principal. An Invited Email can become an Email-Only Principal, but only an address on the Finite VIP Domain can become a Finite VIP Email and NIP-05 Name in v1.
_Avoid_: external email, collaborator email

**Principal Link**:
A verified relationship from an email address to a Native Principal. Products can use Principal Links during authorization without immediately rewriting their product-owned permission records.
_Avoid_: alias, migration, account merge

**Principal Resolution**:
The Finite Identity answer to "who does this email, NIP-05 name, npub, or caller prove as right now?" Principal Resolution lets products attach permissions to stable product concepts while delegating identity proof and email-to-native links to Finite Identity.
_Avoid_: user lookup, account lookup, auth mapping

**Product Grant**:
A product-owned permission record that names a Principal or Invited Email exactly as the product user granted it. Finite Identity does not own Product Grants; it only resolves whether a caller satisfies them.
_Avoid_: identity grant, membership row, access mapping

**Identity Authority**:
The deployed Finite Identity service and its identity-owned storage. It is the source of truth for Principal Resolution, Finite VIP Email bindings, and NIP-05 Names.
_Avoid_: auth server, account service

**Identity Contract**:
The product-facing HTTP contract exposed by the Identity Authority. Finite products consume identity through this contract rather than by owning or directly mutating identity storage.
_Avoid_: internal API, shared database, crate API

**Identity Client Flow**:
A reusable client-side identity workflow implemented by Finite Identity and exposed through product CLIs. A standalone identity CLI may expose the same flows, but product users should not need to leave the product workflow for routine identity setup.
_Avoid_: fsite auth flow, fbrain auth flow

**Resolution Cache**:
A short-lived product-held cache of Principal Resolution answers. A Resolution Cache is never the source of truth and must fail closed when an answer is missing, expired, or uncertain.
_Avoid_: identity replica, local identity store

**Local Identity Key**:
The user's Nostr keypair generated, imported, and stored by the local Finite Identity client contract. The Identity Authority stores public identity state, not Local Identity Key secret material.
_Avoid_: server key, account key, hosted key

**Binding Proof**:
The combined proof required to bind a Finite VIP Email to a Native Principal in v1: a valid email challenge token for the Finite VIP Email and a NIP-98-authenticated request signed by the target Local Identity Key.
_Avoid_: signup proof, verification proof, login proof

**Email Challenge**:
A short-lived, single-use proof request sent to an email address. The challenge token is opaque random secret material, stored only as a hash by the Identity Authority.
_Avoid_: magic token, signed token, email login

**Mailer Adapter**:
The deployment-specific implementation that delivers Email Challenges. Finite Identity owns the challenge flow, while a Mailer Adapter performs delivery through dev outbox, Resend, Postmark, or another provider.
_Avoid_: email service, notification service
