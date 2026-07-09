# Product CLIs wrap Identity Client Flows

Finite Identity owns reusable Identity Client Flows for local key handling, email proof, NIP-98 signing, and binding requests. Product CLIs such as `fsite` and `fbrain` should expose product-shaped wrappers around those flows for v1 onboarding, while a standalone identity CLI remains allowed for debugging, operations, and future direct identity management.
