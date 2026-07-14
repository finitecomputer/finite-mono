# ADR 0004: Products own bounded identity adapters; Finite Identity supplies key primitives

Status: accepted, 2026-07-13.

Implementation note: the hosted phase uses the versioned
`finite-brain-identity-provider-v1` contract. The Product Client calls the
dashboard bridge at `POST /api/brain/identity-provider`; the server-sandboxed
Brain frame receives a signed, expiring capability after WorkOS verification,
and each operation also carries a short-lived proof for its exact request,
minted by the authenticated parent dashboard. The frame keeps the capability;
the parent proves its WorkOS session is still live, so neither alone can invoke
custody. The bridge forwards only the named Brain operations to the Hosted Device's
`POST /v1/brain/identity-provider` executor with the verified WorkOS user and
trusted public Brain origin. The executor loads an existing Hosted Device User
Key without generating one, applies validation owned by `finite-brain-core`,
and returns only signed events or complete, resource-bound grant results. A
grant open validates the NIP-59 wrapper, recipient, issuer, Vault, Folder, key
version, payload, and tags; grant wrapping accepts typed grant or invite
metadata rather than arbitrary NIP-44 plaintext. Missing Chat setup
returns setup-required, and arbitrary sign/decrypt operations are not routes.

## Context

Finite products need a person's Nostr identity without putting raw key material
in browser or renderer code, turning Account Auth into a Nostr signer, or
making one product's signer adapter a cross-product authority.

FiniteBrain is the first product to make this boundary explicit. Finite Sites
will need the same architectural pattern later, but it has its own product
grants, content, and authorization rules. The initial implementation runs each
product's adapter on the server; a later native implementation moves that
product's adapter to the client environment.

## Decision

- Each product owns and versions its own narrow identity-provider contract and
  adapter. Its operations are typed, validated product intents, never arbitrary
  event signing or ciphertext decryption.
- FiniteBrain owns the first such contract, the **Brain Identity Provider**.
  It retains ownership of Vault, Folder, Folder Key Grant, content-crypto, and
  authorization policy.
- Finite Identity owns reusable key-storage and key-lifecycle primitives. It
  does not own a universal product adapter, product grants, content policy, or
  a generic cross-product sign/decrypt API.
- Finite Chat's Hosted Device is the first Account-Auth-bound user-key setup
  and hosted-custody flow. It may use Finite Identity primitives, but it does
  not define Brain or Sites operations and is not their generic signer service.
- Hosted Brain assumes the User Nostr Identity was already set up through that
  Chat flow. If it is absent, Brain fails closed with a basic setup-required
  state; it does not create another user identity or bootstrap a second flow.
- Hosted Web Brain uses the existing human **User Nostr Identity** as its
  Member Identity. Electron and iOS use that same identity from protected local
  storage; the custody difference does not create another Brain identity.
- Account Auth may select that identity for a dashboard session, but it
  does not itself confer Brain membership, Folder Access, or Folder Key Grants.
- Account Auth logout or session expiry locks the Brain Product Client and
  invalidates its hosted-adapter session. This clears temporary session state;
  it does not revoke the User Nostr Identity's underlying Brain grants or stop
  an Agent Runtime using its separate Agent Principal Key and explicit Folder
  access. Stopping that agent requires Brain access revocation and the required
  Folder Key rotation.
- A product's hosted adapter is its server-side stand-in for the product's
  future native adapter. This is Greenfield work: no legacy Brain Vault or
  user-key migration/compatibility path is in scope. Future native custody and
  recovery design remain separate decisions.
- Brain's adapter may open a validated Folder Key Grant, but the Brain Product
  Client holds the resulting Session Folder Key and continues to read, write,
  encrypt, and decrypt Brain content. The adapter is not a general Brain
  plaintext service.
- Brain's hosted adapter accepts requests only from the official Brain Product
  Client. Ordinary dashboard pages, Sites content, and embedded frames never
  receive its capability.
- Brain's compatibility adapter binds HTTP authorization to the official Brain
  origin and protected routes, validates method/body tags, and accepts only
  named Brain event and Folder Key Grant intents.
- Finite Sites will adopt this pattern with its own Sites adapter while using
  Finite Identity primitives. It does not inherit Brain grants or Folder Key
  access.
- Product access pickers show a Managed Agent Email or similarly readable
  identity, resolve it through Finite Identity, and grant the resulting Native
  Principal. Brain establishes this pattern first; Sites may reuse the
  identity-resolution pattern without reusing Brain authorization policy.
- This does not make a User Nostr Identity equal to an Agent Principal Key or
  revive `finite-auth`.
- An agent uses its own Agent Principal Key and only the Folder Key Grants
  addressed to it. It never uses the user's Brain adapter to act as the user.
- An Agent Principal Key receives no Brain access merely because it belongs to
  the same Project or dashboard account. An authorized Brain Member must issue
  the agent's access and Folder Key Grants explicitly.

## Consequences

- Brain supplies the first reference contract and conformance cases, not a
  global product authorization service or shared adapter implementation.
- A hosted adapter must state its trust boundary and keep raw keys out of
  browser and renderer JavaScript.
- A native adapter can take over the same product responsibility later without
  widening the product client API or changing product authorization behavior.
- Sites can reuse Finite Identity primitives without coupling its authorization
  or content model to Brain.

## Rejected shapes

- A global `window.nostr`-style capability that lets product clients sign or
  decrypt arbitrary inputs.
- One generic Finite Identity or Finite Chat signer adapter for every product.
- A shared human-agent secret or revived `finite-auth` service.
- Moving Brain's Folder Key Grant, content-crypto, or access policy into
  Finite Identity merely because an adapter uses identity keys.
- Making Finite Sites a Brain client in order to reuse identity behavior.
