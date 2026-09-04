# Identity Directory Operations

The Identity Directory is the deployed Finite Identity service — the shrunken
Identity Authority. It owns exactly two jobs: serving the Finite VIP Domain's
NIP-05 names (what npub is `name@finite.vip`?) and name claiming (a human
binds `name@finite.vip` to their key, or trusted provisioning registers a
managed agent's name). Operator inspect/disable provides audit and takedown.
Grant resolution, mailbox proofs, email-only principals, WorkOS account
bindings, and the sites-notification relay were deleted: products answer
"who is asking" against their own tables and never call the Directory at
authorization time.

The Local Identity Key remains non-custodial. Product CLIs load, generate, or
import the user's local Nostr key with this crate, then prove key control to
the Identity Directory with NIP-98. The Directory stores public keys and
audit metadata only; it never stores or returns user secret key material.

## Domain Language

- **Identity Directory**: the HTTP service and identity-owned SQLite database.
  The shrunken Identity Authority; "the Directory" and "the Identity
  Authority" name the same deployment.
- **Identity Contract**: the product-facing HTTP API exposed by the Identity
  Directory.
- **Local Identity Key**: the user's local Nostr keypair stored under the
  `finite-identity` file contract.
- **Finite VIP Email**: a Finite-controlled address on the Finite VIP Domain,
  currently `localpart@finite.vip`.
- **NIP-05 Name**: the public Nostr name served by the Identity Directory. In
  v1 it is exactly the Finite VIP Email.
- **Email Challenge**: an opaque, short-lived, single-use token delivered by
  email and stored only as a hash by the Identity Directory. Its sole
  remaining purpose is proof-of-control for claiming a name; it proves
  nothing to any other product.
- **Binding Proof**: a valid Email Challenge plus a NIP-98-authenticated
  redeem request signed by the target Local Identity Key.
- **Native Principal**: a Principal backed by a Nostr public key.
- **Managed Agent Email**: a canonical Finite VIP Email assigned by Core and
  immutably registered to a hosted runtime's Native Principal by trusted
  provisioning.

The full glossary lives in [CONTEXT.md](../CONTEXT.md). The decision log lives
under [docs/adr](./adr).

## Running the Service

`finite-identityd` serves the Identity Contract and the Finite VIP Domain's
NIP-05 endpoint:

```sh
finite-identityd serve \
  --data /var/lib/finite-identity \
  --external-base-url https://identity.finite.vip \
  --finite-vip-domain finite.vip \
  --listen 127.0.0.1:8790 \
  --mailer resend \
  --mail-from "Finite Identity <identity@finite.chat>"
```

Runtime flags:

| Flag | Purpose |
| --- | --- |
| `--data DIR` | Directory for identity-owned SQLite storage. The database file is `DIR/identity.db`. |
| `--external-base-url URL` | Public base URL used when verifying NIP-98 request URLs. This must match the URL product clients sign. |
| `--finite-vip-domain DOMAIN` | Finite VIP Domain. Defaults to `finite.vip`. |
| `--listen HOST:PORT` | Local bind address. Defaults to `127.0.0.1:8790`. |
| `FINITE_IDENTITY_OPERATOR_TOKEN` | Enables v1 operator endpoints without exposing the credential in process arguments. If omitted, operator endpoints reject every request. |
| `--operator-token TOKEN` | Backward-compatible local/debug override. Production services should use `FINITE_IDENTITY_OPERATOR_TOKEN`. |
| `--mailer dev` | Development mailer. Requires `--dev-print-email-tokens yes` so token printing is explicit. |
| `--mailer resend` | Production mailer using the Resend JSON API via the shared `finite-mail` transport. Requires `--mail-from ADDR` and `RESEND_API_KEY`. |
| `--mail-from ADDR` | Sender shown on production Email Challenge messages. Never put provider API keys in argv. |
| `--dev-print-email-tokens yes` | Development-only guard that enables token printing when `--mailer dev` is selected. |

Production deploys should keep provider API keys in the service environment,
not argv. The identity semantics stay the same across mailers: token creation,
hashing, expiry, redemption, and replay rejection remain inside Finite
Identity; only delivery changes.

For local development, make the token-printer explicit:

```sh
finite-identityd serve \
  --data ./.dev/finite-identity \
  --external-base-url http://127.0.0.1:8790 \
  --listen 127.0.0.1:8790 \
  --mailer dev \
  --dev-print-email-tokens yes
```

Set `FINITE_IDENTITY_OPERATOR_TOKEN` in a protected service environment file
when operator endpoints are needed. Never place it in argv or an Agent Runtime.

## HTTP Contract

Products consume the Directory over HTTP. They must not read or mutate the
identity-owned SQLite database directly.

### Public NIP-05

```http
GET /.well-known/nostr.json?name=<localpart>
```

The response is standard NIP-05 `names` JSON:

```json
{
  "names": {
    "alice": "<lowercase-hex-pubkey>"
  }
}
```

Unknown names, invalid localparts, and Disabled Bindings return an empty
`names` map.

### Email Challenge

```http
POST /api/v1/email-challenges
Content-Type: application/json

{ "email": "alice@finite.vip" }
```

Any syntactically valid address can request an Email Challenge, but the token
only has one use left: proving control of a Finite VIP Email when claiming
its name. Only `finite.vip` addresses with a NIP-05-valid localpart can
complete a claim.

### Bind Finite VIP Email (claim a name)

```http
POST /api/v1/vip-email-bindings/redeem
Authorization: Nostr <nip98-event>
Content-Type: application/json

{ "email": "alice@finite.vip", "token": "<email-token>" }
```

The Email Challenge proves control of the Finite VIP Email. The NIP-98 header
proves control of the Local Identity Key that will own the name. The binding
is immutable in v1 except for idempotent re-proving with the same key.
Rebinding to a different key is rejected.

### Resolve a NIP-05 Name

Resolve and classify a Finite NIP-05 Name before a typed CLI uses it:

```http
POST /api/v1/nip05-resolution
Content-Type: application/json

{ "name": "cheater-a1b2c3d4e5f6g7h8@finite.vip" }
```

The response contains the canonical `name`, hex `pubkey`, `npub`, and a `kind`
of `mailbox` or `managed_agent`. A Managed Agent NIP-05 is not deliverable and
must be rejected by email-delivery flags. Unknown names return 404; names
outside the Directory's Finite VIP Domain return 400.

### Operator Endpoints

Operator endpoints are loopback-only and require:

```http
X-Finite-Operator-Token: <configured-token>
```

Register a canonical Managed Agent NIP-05 after a runtime publishes its Agent
Principal Key (Core's runner calls this at agent creation):

```http
POST /api/v1/operator/agent-email-bindings
X-Finite-Operator-Token: <configured-token>
Content-Type: application/json

{ "email": "cheater-a1b2c3d4e5f6g7h8@finite.vip", "agent_npub": "npub1..." }
```

An exact retry is idempotent. A name can never be reassigned to another key,
and a Disabled Binding is not silently re-enabled. The operator credential
belongs only to trusted provisioning; it must never enter an Agent Runtime.

Inspect public identity state:

```http
POST /api/v1/operator/inspect
Content-Type: application/json

{ "identifier": "alice@finite.vip" }
```

`identifier` may be an email address, raw hex pubkey, or `npub1...`. The
response reports the Finite VIP binding (or every binding for the key) plus
recent Email Challenge audit metadata. Responses expose public binding state
and audit metadata only; no secret key material exists server-side.

Disable a Finite VIP Email binding:

```http
POST /api/v1/operator/disable-binding
Content-Type: application/json

{ "email": "alice@finite.vip" }
```

Disabling preserves audit history but suppresses NIP-05 serving and name
resolution. Operators cannot reassign a name, rotate a key, recover an
account, or migrate product data in v1.

## Storage Ownership

The SQLite database under `--data` is identity-owned state. Products must not
read it directly, write it directly, or couple behavior to table layout. The
only production contract for Sites, Brain, and other products is the HTTP
Identity Contract.

The Directory schema is four tables: `native_principals` (registry of known
pubkeys), `vip_email_bindings` (the names), `managed_agent_nip05_bindings`
(managed-agent name markers), and `email_challenges` (hashed, single-use
claim tokens). Tables are created with plain `CREATE TABLE IF NOT EXISTS`
execs; there is no migration mechanism.

Databases created before the directory shrink still contain the retired
`principal_links`, `workos_account_principals`, `email_only_principals`,
`mailbox_proofs`, and `notification_deliveries` tables. The shrink
deliberately adds no drop migration: those tables are never created, written,
or read by current code, and the rows they hold are no longer load-bearing
for any product. Operators may drop them by hand during routine maintenance
if desired; nothing requires it.

Identity-owned storage contains public identity state and challenge audit
metadata, including hashed challenge tokens. It does not contain user secret
keys. Local secret keys live only in each user's Local Identity Key file.

## Backup And Restore

Back up the full `--data` directory, including `identity.db`, with normal
SQLite-safe backup procedures. A backup must preserve:

- Finite VIP Email bindings
- NIP-05 serving state
- Managed Agent NIP-05 registrations
- Disabled Bindings
- Email Challenge audit metadata

Restoring identity-owned storage restores the Directory's public state. It
does not restore user Local Identity Keys because those are never stored by
the Directory. V1 does not reassign Finite VIP Emails to replacement keys. It
is therefore insufficient by itself for the SaaS Recoverability Contract: launch
must add tested same-key recovery material or an explicit Identity Recovery
flow that moves product grants and encrypted key access to a replacement key.

## Product Integration Rules

Products should:

- use `finite_identity::client` helpers to load or generate the Local Identity
  Key, request Email Challenges, and build NIP-98-authenticated name-claim
  requests
- answer every authorization question against their own tables; the Directory
  is a name lookup, never an authorization oracle
- fail closed when a name does not resolve
- own product permissions, product data, and product-specific audit trails

Products should not:

- copy the Local Identity Key secret into product-specific config
- build their own NIP-05 JSON from product tables
- hash, store, or redeem Email Challenge tokens outside Finite Identity
- mutate identity-owned SQLite storage
- treat third-party email-shaped identifiers as trusted Third-Party NIP-05
  Names
- ask the Directory whether a caller satisfies a grant, whether an email
  controls a key, or which WorkOS account a key belongs to — those services
  were deleted; keep membership and account bindings in product-local tables

## Explicit V1 Limits

The following are intentionally out of scope:

- key-loss Identity Recovery
- rebinding a Finite VIP Email or NIP-05 Name to a different pubkey
- product data migration from an old key to a replacement key
- server-side custody of user secret keys
- Third-Party NIP-05 resolution and trust policy
- NIP-05 relay metadata
- arbitrary alternate handles, display names, or non-`finite.vip` Finite VIP
  Domains

Deleted with the directory shrink (do not rebuild on this service):

- Principal Resolution / `satisfies-grant` (products check their own grants)
- Mailbox Proofs and Email-Only Principals (email proof-of-control proves
  nothing beyond a name claim)
- Principal Links (email↔key equivalence claims)
- WorkOS account→principal bindings (moves to Core, which already speaks
  WorkOS)
- the sites-notification relay (Sites sends its own first-publication and
  access-request mail)

These are v1 protocol limits, not permission to strand first-slice user data.
Any product depending on a listed capability must remain non-durable preview or
add a separately versioned and tested recovery contract before launch.

These limits are product-facing behavior. Do not work around them in Sites or
Brain without a new ADR.
