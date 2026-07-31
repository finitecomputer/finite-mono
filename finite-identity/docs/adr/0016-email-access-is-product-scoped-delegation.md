# Mailbox-backed access is product-scoped delegation, not identity linking

Status: accepted 2026-07-09; terminology clarified 2026-07-30.

A verified Mailbox Address may establish a product-owned principal that
explicitly authorizes a distinct Agent Principal inside one Finite product,
with product-owned revocation and audit. In Sites this is a Sites Email
Principal and its Authorized Sites Keys. The agent continues signing as itself;
Finite Identity never turns the authorization into a Principal Link or NIP-05
binding.

This does not transfer across products. Finite Brain must separately issue the
agent the Folder Key Grants required to decrypt content, and Finite Chat still
addresses the canonical participant `npub`.
