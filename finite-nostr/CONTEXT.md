# Finite Nostr

- **Nostr primitive**: reusable encoding, signing, verification, encryption or
  gift-wrap behavior preserving the underlying Nostr protocol semantics.
- **Product policy**: permissions, content models, sync and lifecycle owned by
  the consuming product. It does not belong in `finite-nostr`.

Typed wrappers make protocol inputs and validation explicit without importing
Brain, Sites or Chat authorization policy.
