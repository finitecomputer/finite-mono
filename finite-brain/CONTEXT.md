# FiniteBrain

FiniteBrain owns encrypted knowledge spaces and their access policy. The
server stores ciphertext and permission metadata; authorized clients decrypt.

- **Brain**: a knowledge space containing Folders, membership and access grants.
  A Personal Brain has one human owner; an Organization Brain supports shared
  administration. Brain is the space; FiniteBrain is the product.
- **Principal**: a Nostr public key (`npub`). Human and Agent keys are distinct;
  sharing an account or Project does not grant Brain access.
- **Member / Guest**: a Member belongs to the Brain; a Guest has only explicit
  Folder access. Neither role alone supplies decryption keys.
- **Folder**: the access, encryption and sync boundary inside a Brain.
- **Folder Object**: an encrypted, versioned item in a Folder. Clients derive
  readable Pages and local indexes from decrypted objects.
- **Folder Key Grant**: a Folder key wrapped for a particular Principal.
  Reading requires both current permission and a usable grant for the key epoch.
- **Pending Grant Wrap**: a request for an authorized client to deliver a missing
  key wrap. It is a delivery hint, never permission or a reason to block sync.
- **Working Tree**: a persistent plaintext filesystem projection used by
  `fbrain`. It contains only locally accessible content and remains sensitive
  even when the client session is locked.
- **Mount**: a projection of a shared Folder into another Brain. Source access
  and encryption authority remain with the source Folder.
- **Asset Source Note**: a Markdown note under `raw/` describing non-Markdown
  bytes by canonical `resource` URI; synthesized wiki pages cite the note.
- **Brain Identity Provider**: the product-owned adapter for bounded signing and
  Folder Key Grant operations. It never exposes a raw identity secret or a
  general signing/decryption API to pages or embedded content.

Account authentication and directory lookup supply facts, not Brain grants.
Permissions, grant provenance, revocation and content crypto remain Brain-owned.
See [the CLI guide](README.md), [development](development.md), and
[restore drill](docs/runbooks/brain-restore-drill.md).
