# Immutable VIP Email bindings in v1

In v1, once a Finite VIP Email and matching NIP-05 Name are bound to a Nostr public key, that binding is immutable except for idempotent re-proving with the same key. Rebinding after key loss is intentionally out of scope because it also decides recovery, replacement-key trust, and whether product data should migrate from the old key to the new key.
