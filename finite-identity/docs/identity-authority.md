# Identity Authority Operations

The Identity Authority is the deployed Finite Identity service. It owns public
identity state for Finite products: Finite VIP Email bindings, NIP-05 Names,
Email Challenges, Native Principals, Email-Only Principals, Principal Links,
Principal Resolution, and Disabled Bindings.

The Local Identity Key remains non-custodial. Product CLIs load, generate, or
import the user's local Nostr key with this crate, then prove key control to
the Identity Authority with NIP-98. The Identity Authority stores public keys
and audit metadata only; it never stores or returns user secret key material.

## Domain Language

- **Identity Authority**: the HTTP service and identity-owned SQLite database.
- **Identity Contract**: the product-facing HTTP API exposed by the Identity
  Authority.
- **Local Identity Key**: the user's local Nostr keypair stored under the
  `finite-identity` file contract.
- **Finite VIP Email**: a Finite-controlled address on the Finite VIP Domain,
  currently `localpart@finite.vip`.
- **NIP-05 Name**: the public Nostr name served by the Identity Authority. In
  v1 it is exactly the Finite VIP Email.
- **Email Challenge**: an opaque, short-lived, single-use token delivered by
  email and stored only as a hash by the Identity Authority.
- **Binding Proof**: a valid Email Challenge plus a NIP-98-authenticated
  redeem request signed by the target Local Identity Key.
- **Native Principal**: a Principal backed by a Nostr public key.
- **Email-Only Principal**: a Principal backed by verified control of an
  Invited Email before that address is linked to a Native Principal.
- **Principal Link**: a verified relationship from an email address to a
  Native Principal.
- **Product Grant**: a product-owned permission row. Products store grants as
  entered and ask Principal Resolution whether the current caller satisfies
  them.

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
  --operator-token "$FINITE_IDENTITY_OPERATOR_TOKEN" \
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
| `--operator-token TOKEN` | Enables v1 operator endpoints. If omitted, operator endpoints reject every request. |
| `--mailer dev` | Development mailer. Requires `--dev-print-email-tokens yes` so token printing is explicit. |
| `--mailer resend` | Production mailer using the Resend JSON API. Requires `--mail-from ADDR` and `RESEND_API_KEY`. |
| `--mailer postmark` | Production mailer using the Postmark JSON API. Requires `--mail-from ADDR` and `POSTMARK_SERVER_TOKEN`. |
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
  --operator-token dev-operator-token \
  --mailer dev \
  --dev-print-email-tokens yes
```

## HTTP Contract

Products consume identity over HTTP. They must not read or mutate the
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

Any valid Invited Email can request an Email Challenge. Addresses outside
`finite.vip` can become Email-Only Principals but never create Finite-owned
NIP-05 Names in v1.

### Bind Finite VIP Email

```http
POST /api/v1/vip-email-bindings/redeem
Authorization: Nostr <nip98-event>
Content-Type: application/json

{ "email": "alice@finite.vip", "token": "<email-token>" }
```

The Email Challenge proves control of the Finite VIP Email. The NIP-98 header
proves control of the Local Identity Key that will own the Native Principal.
The binding is immutable in v1 except for idempotent re-proving with the same
key. Rebinding to a different key is rejected.

### Redeem Email-Only Principal

```http
POST /api/v1/email-only-principals/redeem
Authorization: Nostr <nip98-event>
Content-Type: application/json

{ "email": "editor@example.com", "token": "<email-token>" }
```

This creates or refreshes an Email-Only Principal for the signed public key.
If a Finite VIP Email later binds to a Native Principal, the native binding
becomes authoritative for Product Grant satisfaction and the previous
email-only rows for that Finite VIP Email are revoked for authorization.

### Principal Resolution

```http
POST /api/v1/principal-resolution/satisfies-grant
Content-Type: application/json

{
  "grant": "alice@finite.vip",
  "actor_pubkey": "<lowercase-hex-pubkey>"
}
```

Products store Product Grants as entered, then call Principal Resolution at
authorization time. Supported v1 grantees are:

- raw lowercase or uppercase hex public keys
- `npub1...` identifiers
- email-shaped Invited Emails
- Finite VIP NIP-05 Names, which are exactly `localpart@finite.vip`

Third-party email-shaped identifiers are treated as Invited Emails only.
Finite Identity does not perform or trust Third-Party NIP-05 resolution in v1.

Products may keep short-lived Resolution Caches for latency, but the cache is
never the source of truth. Missing, expired, or uncertain answers must fail
closed.

### Operator Endpoints

Operator endpoints require:

```http
X-Finite-Operator-Token: <configured-token>
```

Inspect public identity state:

```http
POST /api/v1/operator/inspect
Content-Type: application/json

{ "identifier": "alice@finite.vip" }
```

`identifier` may be a Finite VIP Email, Invited Email, raw hex pubkey, or
`npub1...`. Responses expose public binding state and audit metadata only; no
secret key material exists server-side.

Disable a Finite VIP Email binding:

```http
POST /api/v1/operator/disable-binding
Content-Type: application/json

{ "email": "alice@finite.vip" }
```

Disabling preserves audit history but suppresses NIP-05 serving and Principal
Resolution. Operators cannot reassign a name, rotate a key, recover an
account, or migrate product data in v1.

## Storage Ownership

The SQLite database under `--data` is identity-owned state. Products must not
read it directly, write it directly, or couple authorization behavior to table
layout. The only production contract for Sites, Brain, and other products is
the HTTP Identity Contract.

Identity-owned storage contains public identity state and challenge audit
metadata, including hashed challenge tokens. It does not contain user secret
keys. Local secret keys live only in each user's Local Identity Key file.

## Backup And Restore

Back up the full `--data` directory, including `identity.db`, with normal
SQLite-safe backup procedures. A backup must preserve:

- Finite VIP Email bindings
- NIP-05 serving state
- Email-Only Principal rows and revocation metadata
- Principal Links
- Disabled Bindings
- Email Challenge audit metadata

Restoring identity-owned storage restores the Authority's public state. It
does not restore user Local Identity Keys because those are never stored by the
Authority. Users who lose their Local Identity Key need a future Recovery
design; v1 does not reassign Finite VIP Emails to replacement keys.

## Product Integration Rules

Products should:

- use `finite_identity::client` helpers to load or generate the Local Identity
  Key, request Email Challenges, and build NIP-98-authenticated redeem
  requests
- store Product Grants exactly as entered by the product user
- call Principal Resolution before permission-changing or sensitive access
  decisions
- fail closed when identity resolution is missing, stale, or rejected
- keep only short-lived Resolution Caches
- own product permissions, product data, and product-specific audit trails

Products should not:

- copy the Local Identity Key secret into product-specific config
- build their own NIP-05 JSON from product tables
- hash, store, or redeem Email Challenge tokens outside Finite Identity
- mutate identity-owned SQLite storage
- treat third-party email-shaped grants as trusted Third-Party NIP-05 Names

## Explicit V1 Limits

The following are intentionally out of scope:

- key-loss Recovery
- rebinding a Finite VIP Email or NIP-05 Name to a different pubkey
- product data migration from an old key to a replacement key
- server-side custody of user secret keys
- Third-Party NIP-05 resolution and trust policy
- NIP-05 relay metadata
- arbitrary alternate handles, display names, or non-`finite.vip` Finite VIP
  Domains
- required product permission rewrites after linking

These limits are product-facing behavior. Do not work around them in Sites or
Brain without a new ADR.
