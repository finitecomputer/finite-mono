# Opaque one-time Email Challenges

Email Challenge tokens are opaque random secrets that are stored only as hashes, expire quickly, and can be redeemed once. Nostr NIPs do not define this email-proof ceremony, and using stored opaque tokens keeps revocation, replay prevention, and rate-limited email proof straightforward while NIP-98 handles proof of Nostr key control.
