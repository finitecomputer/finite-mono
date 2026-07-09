# Identity owns Principal resolution

Finite Identity owns reusable Principal resolution across products: a Principal is either a Native Principal backed by a Nostr public key or an Email-Only Principal backed by verified email control. Products such as Finite Sites and Finite Brain own their product permissions, but they should ask Finite Identity who an email, NIP-05 name, npub, or caller resolves to instead of reimplementing email proof, Nostr identity, or email-to-native migration logic.
