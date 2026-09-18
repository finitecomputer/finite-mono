# Finite Identity

Finite Identity owns local key primitives and public directory bindings.
Products authorize their own requests against their own permission state.

- **Principal**: a Nostr public key (`npub`) identifying a human or agent.
- **Finite Home / Local Identity Key**: one owner's filesystem root and Nostr
  keypair. Tools within one Finite Home share that key; humans and agents do not.
- **Agent Principal Key**: the independent key owned by one Agent Runtime.
- **Mailbox Address**: a deliverable email address. Its spelling does not make
  it a NIP-05 identity or a cross-product permission.
- **NIP-05 Name**: a public name resolving to a Nostr key. A Managed Agent
  NIP-05 names an agent and must never be treated as a deliverable mailbox.
- **Identity Directory**: the service owning public name lookup, verified
  name claims, audit and binding disablement. It stores no local private keys.
- **Email Challenge**: a short-lived, single-use mailbox proof used in name
  claiming. It is not a product access grant.
- **Disabled Binding**: a retained binding excluded from public resolution.
  Disabling does not transfer ownership or recover an identity.
- **Product Signer Adapter**: a product-owned, bounded use of shared key
  primitives. It does not expose raw keys or a universal signing API.

Sites keysets and Brain grants are owned by those products. Account Auth is
separate from directory identity. See [directory operations](docs/identity-authority.md)
and [local key contract](SPEC.md).
