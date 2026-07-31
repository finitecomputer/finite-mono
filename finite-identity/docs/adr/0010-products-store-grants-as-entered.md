# Products store Product Grants as entered

When a product user grants access to an email, NIP-05 name, or native Nostr identifier, the product stores the Product Grant in the shape the user entered rather than rewriting it immediately to the current resolved pubkey. Finite Identity's Principal Resolution decides whether a caller satisfies the grant at authorization time, preserving human-readable audit trails and avoiding product permission rows that change shape based on identity state.
