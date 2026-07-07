# Harden Product Client Folder Key Grant wrapping and opening

## Parent

Parent PRD: #47

## What To Build

Make the Product Client treat NIP-07/NIP-44 grant wrapping as the normal smoke
path. A connected signer should be able to decrypt real gift-wrapped Folder Key
Grants addressed to the active npub, reject wrong-recipient or malformed
wrappers, and create encrypted grant envelopes for access/share flows.

## Acceptance Criteria

- [x] Product Client grant opening uses a NIP-07 `nip44.decrypt` adapter when
  available and validates the visible gift-wrap recipient before decrypting.
- [x] Product Client grant opening validates seal and rumor shape before parsing
  Folder Key Grant plaintext.
- [x] Plaintext development grants are not accepted by default in the Product
  Client smoke path.
- [x] Product Client grant creation uses a NIP-07 `nip44.encrypt` adapter when
  available and produces wrapped events the existing server routes can store.
- [x] Focused JS tests cover encrypted open, wrong-recipient rejection,
  malformed-wrapper rejection, and explicit development fallback behavior.

## Blocked By

None - can start immediately.
