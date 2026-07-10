# FiniteBrain Portability Specification

Status: hard-cut draft implementation spec
Source snapshot: Rust hard-cut branch `feature/rust-portable-v1-core`; see
the source map at the end of this document.

This document describes FiniteBrain Portable v1 at the level needed to
reimplement its data model, cryptographic records, authorization checks, and
sync behavior in another programming language.

It is intentionally implementation-neutral, but it is grounded in the Rust
Portable v1 implementation. When this spec says "MUST", it describes behavior a
compatible implementation needs in order to interoperate with the current
FiniteBrain client and server. When it says "prototype boundary", it describes
behavior that exists now but should be revisited before production hardening.

## 1. Product Boundary

FiniteBrain is a personal and organizational knowledge system built from
Markdown-like file content. The top-level namespace container is a Vault. A
Vault contains Folders. A Folder is the access boundary and the default LLM wiki
scope. A Folder contains Folder Objects. A Folder Object decrypts into a Page,
attachment, asset, generated file, or future content type.

FiniteBrain is a hard cut from the earlier SilverBullet-based prototype. The
active Rust implementation and first-party Product Client use FiniteBrain
product language: Vault, Folder, and Page rather than SilverBullet Space.

The server stores and syncs encrypted Vault state. It does not need to decrypt
Page paths, Page titles, links, backlinks, wiki indexes, wiki logs, or Page
contents. The trusted client or Agent Runtime opens Folder Key Grants, decrypts
accessible Folder Objects, and materializes readable content into a Vault
Working Tree.

A FiniteBrain Vault is not one wiki with folders. It is one Vault namespace
containing many Folder-scoped LLM wikis. Indexes and logs live at the same
Folder scope as the knowledge they describe.

## 2. Trust Model

There are three relevant trust zones.

Trusted local client:

- Holds the active User's Nostr signing capability through a NIP-07 provider.
- Uses NIP-44 encryption/decryption exposed by that provider.
- Opens Folder Key Grants into an in-memory session keyring.
- Encrypts plaintext Page content before uploading it.
- Decrypts accessible ciphertext after sync.
- Builds local indexes, link graphs, search, and LLM Wiki artifacts over
  decrypted content.

Trusted Agent Runtime:

- May read and write the Vault Working Tree as ordinary files when operating
  with a Member Identity that has Folder Access.
- Receives no authority merely from being an agent; Vault Membership, Folder
  Access, grants, and attribution belong to the signing npub.
- FiniteBrain does not classify whether a human, agent, shared client, or
  several clients control that npub.

FiniteBrain server:

- Authenticates HTTP writes and secure reads with Nostr HTTP authorization.
- Stores Vault metadata, access state, Folder Key Grant envelopes, Share Links,
  Invitation Links, Folder Mounts, encrypted Folder Objects, and sync records.
- Gates access by Vault Membership, Vault Admin status, and Folder Access.
- MUST NOT require Page plaintext for authorization or sync.
- Cannot recover a User's Folder Keys after Nostr key loss.

That server limitation is not a complete product recovery posture. A durable
Finite Product Release MUST establish and test an independent user-held or
Finite-assisted Recovery Principal/backup path for every Folder before relying
on that Folder as the sole copy of user data.

## 3. Identity And Auth

### 3.1 User Identity

A User is identified by a Nostr public key encoded as a NIP-19 `npub`; this is
the protocol's Member Identity, not a claim that its controller is human. A
human, agent, shared client, or several clients may control it. Multiple
devices may use the same identity, while separate keypairs are separate Users
and Members from FiniteBrain's access-control perspective.

Changing Nostr identity creates a new User from the access-control perspective.
Nostr key loss means existing Folder Key Grants cannot be decrypted by the new
identity.

A product-scoped Email Access Delegation may authorize a distinct Agent
Principal to exercise one verified email Principal's FiniteBrain grants, but it
does not change either identity and never substitutes for cryptographic access.
Every readable Folder still requires a current Folder Key Grant addressed to
the agent npub. Sites delegations have no effect in FiniteBrain, and revocation
must be independently enforceable here.

### 3.2 Browser Signing Interface

The browser client expects a NIP-07 provider with:

- `getPublicKey()`
- `signEvent(event)`
- `nip44.encrypt(pubkey, plaintext)`
- `nip44.decrypt(pubkey, ciphertext)`

The client wraps this as a `VaultCryptoProvider`.

### 3.3 HTTP Authorization

Secure Vault API calls use a Nostr authorization header:

```http
Authorization: Nostr <base64-json-event>
X-Nostr-Authorization: Nostr <base64-json-event>
X-FiniteBrain-Nostr: Nostr <base64-json-event>
```

The client also mirrors the same authorization value into a short-lived cookie
whose name is derived from the HTTP method, absolute URL, and body:

```text
finitebrain_nostr_<fnv1a32(method + "\n" + absoluteURL + "\n" + body)>
```

The signed authorization event has:

```json
{
  "kind": 27235,
  "created_at": 1780000000,
  "tags": [
    ["u", "https://host/_admin/vaults/..."],
    ["method", "POST"],
    ["payload", "<sha256 hex of request body, only when body is non-empty>"]
  ],
  "content": ""
}
```

Server validation rules:

- Event kind MUST be `27235`.
- Event content MUST be empty.
- Event timestamp MUST be within 60 seconds of server time.
- The `u` tag MUST match the absolute request URL after forwarded-host/proto
  handling.
- The `method` tag MUST match the HTTP method.
- If the request body is non-empty, the `payload` tag MUST be the SHA-256 hex
  digest of the exact request body bytes.
- The Nostr event id and Schnorr signature MUST verify.
- The actor npub is derived from the event pubkey.

Rust hard-cut behavior: protected API routes derive the actor from Nostr HTTP
authorization. Metadata, Vault creation, secure object, sync, folder grant,
access, sharing, and invitation routes do not accept `X-Actor-User-Id` as an
authorization bridge.

Creating a Vault with `POST /_admin/vaults` requires Nostr authorization. The
created Vault owner/admin is the signer npub, not a caller-supplied `userId`.

## 4. Core Domain Model

### 4.1 Vault

A Vault is the top-level privacy container.

```json
{
  "id": "acme",
  "kind": "personal | organization",
  "name": "Acme",
  "ownerUserId": "npub...", 
  "folders": [],
  "members": [],
  "admins": [],
  "invitations": []
}
```

Personal Vault:

- Has exactly one owner identity in `ownerUserId`.
- Starts with Folder-scoped wiki roots:
  - `getting-started`: role `personal_home`, access `owner`
  - `restricted`: role `folder`, access `restricted`
- Does not use ordinary organization membership/admin lists for the owner.
- May contain limited members only when sharing a source Folder.

Organization Vault:

- Has Vault Members.
- Has Vault Admins.
- Every Vault Admin MUST also be a Vault Member.
- A new organization Vault starts with:
  - `getting-started`: role `general`, access `all_members`
  - `restricted`: role `folder`, access `restricted`
- Additional team or domain knowledge SHOULD be created as explicit Folders.
- Sensitive or limited-audience org knowledge SHOULD be created as restricted
  Folders rather than hidden local directories inside an all-member Folder.
- Organization Vaults MUST keep at least one Vault Admin.

### 4.2 Vault Member

```json
{
  "userId": "npub...",
  "folderAccess": [
    { "folderId": "strategy" }
  ]
}
```

Folder access is binary. There are no viewer/editor/read-only roles in the
current model.

### 4.3 Folder

```json
{
  "id": "strategy",
  "name": "Strategy",
  "role": "personal_home | vault_ops | general | folder",
  "access": "owner | admin_only | all_members | restricted",
  "parentFolderId": "parent-folder",
  "path": "Parent/Strategy",
  "sharedFolderSource": true
}
```

Folder rules:

- Every Folder has its own independent Folder Key.
- There is no Vault Root Key.
- A Child Folder is a real Folder, not a subdirectory access shortcut.
- Parent Folder access does not imply Child Folder access.
- Child Folder access does not imply Parent Folder access.
- Folder names need only be unique among sibling folders from the product point
  of view. Current route IDs must still be unique per Vault.
- Moving or renaming a Folder changes metadata only. It does not change Folder
  Keys, Folder Object ids, or Folder Access.
- Creating a local directory inside a Folder does not create a Child Folder.
- A local directory inside a Folder is part of that same Folder-scoped wiki and
  inherits that Folder's access boundary.

Access modes:

- `owner`: personal Vault owner only.
- `admin_only`: Vault Admins only.
- `all_members`: all organization Vault Members and Vault Admins.
- `restricted`: personal Vault owner, organization Vault Admins, plus members
  listed in `folderAccess`.

### 4.3.1 Folder-scoped LLM Wiki Profile

Every readable Folder SHOULD be treated as an independent LLM Wiki root unless
its local instructions say otherwise.

The default wiki profile for a knowledge Folder is:

```text
config.md
_index.md
log.md
inbox/
raw/
  assets/
wiki/
inventory/
datasets/
output/
archive/
```

Scope rules:

- `_index.md` MUST describe only the Folder it lives in.
- `log.md` MUST record only meaningful writes and maintenance work inside that
  Folder.
- A Vault-wide or root-level index MUST NOT reveal titles, summaries, activity,
  or source hints from Folders the active User cannot access.
- Trusted clients and Agent Runtimes MUST filter by Folder access before
  reading, querying, compiling, indexing, or answering with content.
- Non-Markdown source material SHOULD be captured as an Asset under
  `raw/assets/` and paired with a Markdown Source Note in the same Folder.
- Agents SHOULD cite and reason over Source Notes and synthesized wiki Pages,
  not over opaque asset bytes directly.
- Content from a more-restricted Folder MUST NOT be synthesized into a
  less-restricted Folder, index, log, output, or public summary unless the User
  explicitly chooses a destination Folder whose audience is allowed to see the
  source material.
- Cross-Folder outputs SHOULD be written to the most restrictive common Folder
  that is appropriate for every source used.
- Folder names and server-visible Folder ids are metadata. Sensitive project or
  people names SHOULD live inside encrypted Pages when the Folder audience is
  narrow.

This profile is FiniteBrain's access-aware adaptation of the LLM Wiki topic
model: the LLM Wiki spec's "topic wiki" maps to a FiniteBrain Folder, because
Folder Keys and Folder Access are the enforceable privacy boundary.

### 4.4 Folder Object

A Folder Object is an encrypted item addressed by an opaque object id.

```json
{
  "objectId": "obj_0123456789abcdef",
  "folderId": "strategy",
  "keyVersion": 1,
  "cipher": "AES-256-GCM",
  "ciphertext": "{\"version\":\"finite-folder-object-v1\",...}",
  "ciphertextSize": 240,
  "revision": 1,
  "authorNpub": "npub...",
  "updatedAt": "2026-06-23T00:00:00Z",
  "deleted": false,
  "revisionEvent": { "kind": 30078 },
  "tombstoneEvent": { "kind": 30078 }
}
```

Server-visible Folder Object identity is `vaultId + folderId + objectId`.
Server-visible metadata includes object ids, Folder ids, key versions, cipher,
ciphertext size, revisions, authors, update times, and deletion state. It does
not include Page path, Page title, links, backlinks, or content.

Object ids MUST match:

```text
^[A-Za-z0-9_-]{16,128}$
```

Object ids MUST be path-safe base names with no file extension.

### 4.5 Folder Object Plaintext

After decryption, trusted clients and Agent Runtimes normalize Folder Object
plaintext into a typed local object model. The server stores opaque ciphertext
and MUST NOT parse this plaintext.

Canonical Page plaintext has this shape:

```json
{
  "type": "page",
  "path": "wiki/concepts/example.md",
  "title": "Example",
  "links": ["Other Page"],
  "backlinks": [],
  "content": "# Example\n",
  "contentType": "text/markdown"
}
```

Asset plaintext has this shape:

```json
{
  "type": "asset",
  "path": "raw/assets/source.pdf",
  "filename": "source.pdf",
  "contentType": "application/pdf",
  "size": 12345,
  "contentHash": "<sha256 hex of plaintext bytes>",
  "bytesBase64": "<base64 plaintext asset bytes>"
}
```

Rules:

- Canonical Page plaintext uses `type: "page"` and
  `contentType: "text/markdown"`.
- The current hard-cut client also recognizes the versioned Markdown Page
  envelope `{ "version": "finite-folder-object-page-v1", "path": "...",
  "markdown": "..." }` and normalizes it to a Page.
- Asset plaintext MUST use `type: "asset"` and a non-Markdown `contentType`.
- Asset plaintext paths SHOULD live under `raw/assets/` inside the containing
  Folder.
- Every non-Markdown source Asset SHOULD have a Markdown Source Note Page in the
  same Folder. The Source Note records provenance, content type, hash or
  extraction status when known, and links to any synthesized wiki Pages.
- Search, graph, and LLM Wiki synthesis SHOULD index Source Notes and Markdown
  Pages first. Asset bytes are preserved evidence, not the primary knowledge
  surface.
- Compatible clients MUST tolerate future plaintext types they do not
  understand by preserving encrypted records and avoiding lossy rewrites.

### 4.6 Vault Metadata Response

The metadata API exposes server-visible metadata:

```json
{
  "vaultId": "acme",
  "folders": [
    {
      "id": "strategy",
      "name": "Strategy",
      "role": "folder",
      "access": "restricted",
      "parentFolderId": "general",
      "path": "General/Strategy",
      "sharedFolderSource": false,
      "accessUserIds": ["npub..."],
      "currentKeyVersion": 1,
      "setupIncomplete": false
    }
  ],
  "objects": [],
  "pages": []
}
```

Page metadata in the legacy `pages` array is visible only for accessible
Folders. The secure object path should not depend on server-side Page metadata.

### 4.7 Path And Naming Rules

FiniteBrain has two path layers:

- Folder hierarchy paths are server-visible metadata.
- Page/object plaintext paths are encrypted inside Folder Objects.

Rules:

- All paths MUST be UTF-8 strings normalized to Unicode NFC before comparison,
  storage, link extraction, and collision checks.
- Path comparison is case-sensitive. Portable implementations MUST NOT fold
  case unless a local filesystem adapter does so only as an unresolved local
  conflict.
- Folder ids and object ids are opaque stable ids. They are not derived from
  display names and do not change on rename.
- Folder display names are user-facing labels. They MUST NOT contain `/`, NUL,
  or control characters.
- Page plaintext paths MUST be relative safe paths. They MUST NOT be absolute,
  contain `..`, contain NUL, or resolve outside the Folder root.
- The reserved top-level names `.finitebrain`, `_admin`, and `.git` MUST NOT be
  used as Folder root paths or decrypted Page path first segments.
- A Page path collision inside one Folder is a write conflict. A Folder root
  collision with another Folder root or Page path is also a conflict.
- Renaming a Folder changes only Folder metadata. It does not rewrite Page
  plaintext paths, object ids, Folder Keys, or grants.
- Moving a Folder changes only parent metadata and the decorated Folder path.
  It does not move encrypted objects between Folders.
- Moving a Page within the same Folder is an object update with operation
  `move`, a new encrypted plaintext path, and the same object id.
- Moving a Page between Folders is a delete/tombstone in the source Folder plus
  a create in the destination Folder. It requires authority to write both
  Folders and a Folder Key for the destination Folder.
- Deleting a Page creates a tombstone. Deleting a Folder requires either no live
  objects or an explicit recursive delete operation; the recursive operation is
  outside Portable v1 unless a server route defines it.

Slug/display-name guidance:

- UI-created Folder ids SHOULD use a path-safe slug plus collision suffix, but
  compatibility MUST NOT depend on a particular slug algorithm.
- UI-created object ids SHOULD be random path-safe identifiers, not a hash of
  the Page path or title.

### 4.8 Vault Bootstrap

Vault bootstrap is the first atomic access-control boundary.

Personal Vault bootstrap:

- Create one Vault with `kind: "personal"` and `ownerUserId` equal to the
  acting User.
- Create the default personal wiki scope Folders from Section 4.1 with current
  key version `1` and Folder Key Grants for the owner.
- Seed ordinary encrypted Folder Objects for default Pages:
  - `AGENTS.md`, `HUMANS.md`, `README.md`, and orientation Pages in
    `getting-started`
  - `config.md`, `_index.md`, and `log.md` in each default personal Folder
  - a restricted example Page in `restricted`

Organization Vault bootstrap:

- Create one Vault with `kind: "organization"`.
- Add the acting User as both Vault Member and Vault Admin.
- Create the default organization wiki scope Folders from Section 4.1 with
  current key version `1` and grants for all initial members/admins.
- Seed ordinary encrypted Folder Objects for default Pages:
  - `AGENTS.md`, `HUMANS.md`, `README.md`, and orientation Pages in
    `getting-started`
  - `config.md`, `_index.md`, and `log.md` in each default organization Folder
  - a restricted example Page in `restricted`

Default starter Pages MUST explain the Asset Source Note convention:
non-Markdown source files are Assets under `raw/assets/`, and every Asset
SHOULD have a Markdown Source Note in the same Folder before agents cite it
from synthesized `wiki/` pages.

Smoke/demo bootstrap:

- Demo seeds may create additional Folders and Pages, but MUST create current
  Folder Keys and Folder Key Grants for every seeded Folder.
- Demo seeds MUST be reproducible by script or fixture and MUST avoid relying
  on hidden local browser keyring state.
- Test-only Folders and Pages MUST be clearly marked or omitted from reusable
  smoke fixtures.

## 5. Cryptographic Primitives

### 5.1 Folder Key

A Folder Key is a 256-bit AES-GCM key.

Compatible implementations MUST support:

- Generate a random 256-bit AES-GCM key.
- Export the raw key bytes as base64 for inclusion in a Folder Key Grant
  plaintext.
- Import a base64 raw key as an AES-GCM encrypt/decrypt key.

Prototype boundary: the browser session keyring is in-memory. Opened Folder
Keys are not durable across browser sessions.

### 5.2 Folder Object Encryption

Folder content encryption uses AES-256-GCM.

Input:

- Folder Key for `(vaultId, folderId, keyVersion)`.
- A 12-byte random nonce.
- Folder Object plaintext JSON.
- Additional authenticated data.

AAD is the UTF-8 bytes of this JSON object:

```json
{
  "version": "finite-folder-object-v1",
  "vaultId": "acme",
  "folderId": "strategy",
  "objectId": "obj_0123456789abcdef",
  "keyVersion": 1
}
```

The encrypted envelope is:

```json
{
  "version": "finite-folder-object-v1",
  "cipher": "AES-256-GCM",
  "keyVersion": 1,
  "nonce": "<base64 12 bytes>",
  "ciphertext": "<base64 AES-GCM ciphertext plus tag>"
}
```

The request-level `cipher` field MUST be `AES-256-GCM`. The serialized
envelope's `cipher` MUST also be `AES-256-GCM`, and its `keyVersion` MUST match
the request key version.

The revision event's `ciphertextHash` is:

```text
sha256_hex(exact serialized encrypted envelope string)
```

Portable implementations MUST keep the exact serialized encrypted envelope
string they submit and hash that same byte string in the revision event.

### 5.3 Folder Key Grant

A Folder Key Grant gives one recipient npub the raw Folder Key for one
`vaultId + folderId + keyVersion`.

The plaintext grant is:

```json
{
  "version": "finite-folder-key-grant-v1",
  "vaultId": "acme",
  "folderId": "strategy",
  "keyVersion": 1,
  "issuerNpub": "npub-admin",
  "recipientNpub": "npub-member",
  "folderKey": "<base64 raw AES-256 key>",
  "issuedAt": "2026-06-23T00:00:00.000Z"
}
```

The stored grant envelope is:

```json
{
  "id": "folder-key-grant-1",
  "vaultId": "acme",
  "folderId": "strategy",
  "keyVersion": 1,
  "issuerNpub": "npub-admin",
  "recipientNpub": "npub-member",
  "format": "NIP-59",
  "wrappedEvent": { "kind": 1059 },
  "accessChangeEvent": { "kind": 30078 },
  "createdAt": "2026-06-23T00:00:00Z"
}
```

The current implementation stores NIP-59-shaped envelopes on the FiniteBrain
server rather than publishing them to public relays.

Grant creation:

1. Convert `recipientNpub` to recipient pubkey hex.
2. Create an unsigned rumor:

```json
{
  "kind": 30078,
  "created_at": 1780000000,
  "tags": [
    ["d", "finite-folder-key-grant:<vaultId>:<folderId>"],
    ["vault", "<vaultId>"],
    ["folder", "<folderId>"],
    ["keyVersion", "1"],
    ["p", "<recipient pubkey hex>"]
  ],
  "content": "<JSON FolderKeyGrantPlaintext>",
  "pubkey": "<issuer pubkey hex>",
  "id": "<nostr event hash of unsigned rumor>"
}
```

3. Create a seal event signed by the issuer:

```json
{
  "kind": 13,
  "created_at": 1780000000,
  "tags": [],
  "content": "<NIP-44 encrypt JSON(rumor) from issuer to recipient>"
}
```

4. Generate a one-time Nostr secret key.
5. Create a gift wrap event signed by the one-time key:

```json
{
  "kind": 1059,
  "created_at": 1780000000,
  "tags": [["p", "<recipient pubkey hex>"]],
  "content": "<NIP-44 encrypt JSON(seal) from one-time key to recipient>"
}
```

6. Store the envelope with `format: "NIP-59"` and the context fields.

Grant opening:

1. Check the envelope context matches expected `vaultId`, `folderId`,
   `keyVersion`, `issuerNpub`, and `recipientNpub`.
2. Check `format` is `NIP-59`.
3. Verify wrapped event kind `1059`, signature, and recipient `p` tag.
4. Recipient decrypts wrapped content with NIP-44 using the wrapped event
   pubkey.
5. Parse the seal, require kind `13`, no tags, and valid issuer signature.
6. Recipient decrypts seal content with NIP-44 using issuer pubkey.
7. Parse the rumor, verify rumor pubkey equals issuer pubkey and rumor id equals
   the Nostr hash of the rumor.
8. Parse the Folder Key Grant plaintext and verify the same context.
9. Import `folderKey` as the AES-GCM Folder Key.

Server validation of grants is intentionally envelope-level. The server checks
metadata, issuer, recipient, kind, signature, and recipient tag. It does not
decrypt Folder Keys.

### 5.4 Signed Folder Object Revision

Every create/update/move write carries a signed Nostr event of kind `30078`.

Payload:

```json
{
  "version": "finite-folder-object-revision-v1",
  "vaultId": "acme",
  "folderId": "strategy",
  "objectId": "obj_0123456789abcdef",
  "operation": "create | update | move",
  "revision": 1,
  "baseRevision": null,
  "keyVersion": 1,
  "cipher": "AES-256-GCM",
  "ciphertextHash": "<sha256 hex of serialized encrypted envelope>",
  "authorNpub": "npub-author",
  "createdAt": "2026-06-23T00:00:00.000Z"
}
```

Tags:

```json
[
  ["d", "finite-folder-object-revision:<vaultId>:<folderId>:<objectId>:<revision>"],
  ["vault", "<vaultId>"],
  ["folder", "<folderId>"],
  ["object", "<objectId>"],
  ["operation", "create"],
  ["keyVersion", "1"]
]
```

Validation rules:

- Event kind MUST be `30078`.
- Event signature MUST verify.
- Event signer npub MUST equal `authorNpub` and request actor.
- Payload fields MUST match the request and server-computed revision.
- `ciphertextHash` MUST match the exact ciphertext string submitted.
- Tags MUST match the payload.

### 5.5 Signed Folder Object Tombstone

Deletes create tombstones rather than deleting history semantically.

Payload:

```json
{
  "version": "finite-folder-object-tombstone-v1",
  "vaultId": "acme",
  "folderId": "strategy",
  "objectId": "obj_0123456789abcdef",
  "operation": "delete",
  "revision": 2,
  "baseRevision": 1,
  "authorNpub": "npub-author",
  "deletedAt": "2026-06-23T00:00:00.000Z"
}
```

Tags:

```json
[
  ["d", "finite-folder-object-tombstone:<vaultId>:<folderId>:<objectId>:<revision>"],
  ["vault", "<vaultId>"],
  ["folder", "<folderId>"],
  ["object", "<objectId>"],
  ["operation", "delete"]
]
```

Validation rules mirror revision events, but the operation is always `delete`.

### 5.6 Vault Admin Access Change

Admin access changes are signed Nostr event kind `30078`.

Actions:

- `add-member`
- `remove-member`
- `add-admin`
- `remove-admin`
- `grant-folder-access`
- `remove-folder-access`
- `rotate-folder-key`
- `set-folder-access-mode`

Payload:

```json
{
  "version": "finite-vault-admin-access-change-v1",
  "vaultId": "acme",
  "changeId": "grant-strategy-bob-1",
  "action": "grant-folder-access",
  "adminNpub": "npub-admin",
  "folderId": "strategy",
  "targetNpub": "npub-member",
  "keyVersion": 1,
  "note": "optional",
  "createdAt": "2026-06-23T00:00:00.000Z"
}
```

Tags:

```json
[
  ["d", "finite-vault-admin-access-change:<vaultId>:<changeId>"],
  ["vault", "<vaultId>"],
  ["action", "<action>"],
  ["folder", "<folderId>"],
  ["p", "<target pubkey hex>"],
  ["keyVersion", "1"]
]
```

Rules:

- Event kind MUST be `30078`.
- Signature MUST verify.
- Signer npub MUST equal `adminNpub`.
- Payload and tags MUST match the requested server mutation.
- `createdAt` MUST parse as RFC3339/RFC3339Nano and match the event
  `created_at` to second precision.

### 5.7 Canonical Serialization And Test Vectors

Portable v1 treats serialization as part of the cryptographic boundary.

Canonical JSON rules for FiniteBrain-owned payloads:

- JSON is UTF-8.
- Object property order is the order shown in this specification for signed
  payloads, encrypted AAD, encrypted envelopes, grants, exports, and sync
  request bodies.
- No insignificant whitespace is emitted in cryptographic hash inputs.
- Strings are emitted with JSON escaping only where required by JSON.
- Numbers are base-10 JSON numbers. `keyVersion`, `revision`, `baseRevision`,
  `created_at`, and `sequence` are integers.
- Timestamps in JSON payloads use UTC RFC3339/RFC3339Nano with a trailing `Z`.
- Nostr event `created_at` is Unix seconds.
- Hex digests are lowercase.
- Base64 is RFC4648 standard base64 with padding.
- Arrays preserve order. Tags preserve exact order.

Nostr event ids use the Nostr canonical event serialization:

```json
[0,"<pubkey hex>",1780000000,30078,[["d","..."]],"<content>"]
```

The event id is the SHA-256 hash of the exact UTF-8 bytes of that serialized
array. The signature is over that id. Implementations MUST NOT sort Nostr tags
after the event id has been computed.

Hash inputs:

- HTTP auth `payload` tag hashes the exact request body bytes.
- Folder Object revision `ciphertextHash` hashes the exact serialized
  encrypted envelope string submitted in the request.
- Deterministic ids that use `sha256(...)` hash the exact UTF-8 string shown in
  the id formula, including newline separators.

Non-signature test vectors:

```text
request body:
{"recordType":"folder_object_revision","folderId":"strategy","objectId":"obj_0123456789abcdef"}

sha256:
beb370cd8804a3a4e7b4764f1f7fdf4bac95895004513a19abee515a2b9c55e4
```

```text
serialized encrypted envelope:
{"version":"finite-folder-object-v1","cipher":"AES-256-GCM","keyVersion":1,"nonce":"AAAAAAAAAAAAAAAA","ciphertext":"AQIDBAUGBwgJCgsMDQ4PEA=="}

ciphertextHash:
9083fa9666f921de7da1d0b435903e98045b27a1065030dc6d4c841d2374b5bb
```

A complete compatibility fixture suite SHOULD include:

- One Nostr authorization event for each HTTP method used by secure routes.
- One valid Folder Object encryption vector with key, nonce, AAD, plaintext,
  envelope, and ciphertext hash.
- One valid Folder Key Grant envelope for a known issuer/recipient keypair.
- One valid create revision, update revision, move revision, and tombstone.
- One duplicate sync submit case where the same event id returns the existing
  sequence.
- One stale `baseRevision` case that returns `409`.
- One bootstrap plus incremental pull sequence with visible and inaccessible
  records.

## 6. Access Rules

### 6.1 Folder Access

Access is binary:

- If a member has Folder Access, they can read and write encrypted content in
  that Folder.
- If a member lacks Folder Access, they can see visible Folder metadata but
  cannot open content.
- Vault Admins have full access to all Folders in an Organization Vault.
- A Personal Vault owner has access to owner Folders.

### 6.2 Required Folder Key Recipients

For any current Folder Key version, recipients are:

- `all_members`: all Vault Members plus all Vault Admins.
- `restricted`: members listed for that Folder plus all Vault Admins.
- `owner`: the Personal Vault owner.
- `admin_only`: all Vault Admins. In the current helper this is satisfied by
  always adding admins after mode-specific recipients.

Recipients are unique and sorted when computed by the server/client helpers.

### 6.3 Folder Creation

Organization Folder creation is an atomic cryptographic action:

1. Vault Admin creates a new Folder Key.
2. Client computes required recipients.
3. Client creates a Folder Key Grant for each required recipient at
   `keyVersion: 1`.
4. Client submits Folder metadata and all Folder Key Grants together.
5. Server validates the actor is a Vault Admin, the hierarchy is valid, and the
   grants cover every required recipient.
6. Server stores Folder metadata and grants together.

A Folder without current grants is `setupIncomplete`. Empty legacy Folders may
be repaired by Vault Admins with Finish Setup. Finish Setup generates current
grants for required recipients, stores them, and leaves content unchanged.

### 6.4 Granting Access

Adding Folder Access:

- Does not require Folder Key Rotation.
- Requires Folder Key Grants for each newly added recipient.
- Requires a signed Vault Admin Access Change with action
  `grant-folder-access` or the appropriate set/access-mode action.

Removing Folder Access:

- Requires Folder Key Rotation.
- Must re-encrypt every live Folder Object under a new Folder Key version.
- Must issue Folder Key Grants for every remaining required recipient.
- Must carry a signed Vault Admin Access Change.

### 6.5 Folder Key Rotation

Rotation request:

```json
{
  "newKeyVersion": 2,
  "objects": [
    {
      "objectId": "obj_0123456789abcdef",
      "baseRevision": 1,
      "keyVersion": 2,
      "cipher": "AES-256-GCM",
      "ciphertext": "<serialized finite-folder-object-v1 envelope>",
      "revisionEvent": { "kind": 30078 }
    }
  ],
  "folderKeyGrants": []
}
```

Server validation:

- `newKeyVersion` MUST be greater than the current version for removal/admin
  removal flows.
- Every live object in the Folder MUST be included exactly once.
- Tombstoned objects MUST NOT be re-encrypted.
- Each rotation object's `baseRevision` MUST match the current object revision.
- Each rotation object MUST validate as a normal update revision signed by the
  actor.
- Rotation grants MUST target every remaining required recipient.
- Rotation grants MUST use the new key version.

After validation, the server applies every object update and appends all new
Folder Key Grants.

## 7. Vault Working Tree

A Vault Directory is the portable local directory shape:

```text
<root>/
  .finitebrain/
    vault-directory.json
    working-tree-state.json
    encrypted-sync/
      folders/<folderId>/<objectId>.json
  <folder roots and plaintext files for accessible folders>
```

`vault-directory.json`:

```json
{
  "version": "finite-vault-directory-v1",
  "vault": {
    "id": "acme",
    "kind": "organization",
    "name": "Acme",
    "ownerNpub": "npub..."
  },
  "workingTree": { "path": "." },
  "encryptedSync": { "path": ".finitebrain/encrypted-sync" },
  "portability": {
    "ownedByAgentRuntime": false,
    "ownedByAppSurface": false
  },
  "createdAt": "2026-06-23T00:00:00.000Z",
  "updatedAt": "2026-06-23T00:00:00.000Z"
}
```

`working-tree-state.json`:

```json
{
  "version": "finite-vault-working-tree-state-v1",
  "folderRoots": [
    {
      "folderId": "strategy",
      "path": "General/Strategy",
      "canRead": true,
      "metadataOnly": false
    }
  ],
  "objects": [
    {
      "folderId": "strategy",
      "path": "wiki/concepts/example.md",
      "objectId": "obj_0123456789abcdef",
      "revision": 1,
      "keyVersion": 1,
      "contentType": "text/markdown",
      "contentHash": "<sha256 hex>"
    },
    {
      "folderId": "strategy",
      "path": "raw/assets/source.pdf",
      "objectId": "obj_asset_0123456789",
      "revision": 1,
      "keyVersion": 1,
      "contentType": "application/pdf",
      "contentHash": "<sha256 hex>"
    }
  ],
  "sync": {
    "latestSequence": 42
  }
}
```

Materialization rules:

- Accessible Folders are materialized as normal directories.
- Accessible Pages are materialized as UTF-8 Markdown files. Accessible Assets
  are materialized as ordinary files at their decrypted object path.
- Non-Markdown files under `raw/assets/` SHOULD have a sibling or nearby
  Markdown Source Note that explains provenance and extraction status.
- Inaccessible ancestor Folders may be materialized as metadata-only containers
  to make accessible Child Folders reachable.
- Inaccessible ciphertext may be stored under `.finitebrain/encrypted-sync`.
- Folder root paths and object paths MUST be relative safe paths.
- A decrypted object path MUST NOT collide with another child Folder root.
- Working-tree creates/edits/moves/deletes are detected from
  `working-tree-state.json`.
- Cross-Folder moves require Vault Admin authority. Otherwise they are
  unresolved changes.

## 8. Server Storage

Current Rust hard-cut storage:

```text
finite-brain.sqlite3
finite-brain.sqlite3-wal   # when SQLite WAL mode leaves a live WAL file
finite-brain.sqlite3-shm   # when SQLite WAL mode leaves a live SHM file
```

The Rust implementation uses SQLite as the authoritative store for Vault
metadata, Folder hierarchy and access state, Folder Key Grants, sync append log,
current encrypted object projection, Invitations, Share Links, Shared Folder
Connections, and Mounts.

Legacy JSON metadata files are not part of the Rust Portable v1 hard-cut route
surface. They may appear only as explicit import or migration inputs outside the
default secure flow.

### 8.1 Backup, Migration, And Retention

A complete server backup MUST include:

- The configured SQLite database file, default `finite-brain.sqlite3`.
- SQLite WAL/SHM files if WAL mode is active and the database has not been
  checkpointed.
- Any separate local Product Client or Vault Working Tree projections only when
  the backup is meant to preserve local decrypted/agent workspace state.

Backup consistency:

- The backup process MUST either pause writes or take a database-consistent
  snapshot.
- If writes are not paused, SQLite MUST be backed up through a safe online
  backup/checkpoint mechanism, not by copying only the main database file.
- A restored server MUST NOT advertise metadata grants that do not exist in sync
  records when those grants are required for current content.

Restore order:

1. Restore the database into an isolated server data path.
2. Run schema migrations before serving traffic.
3. Validate every Vault id, Folder id, member npub, mount source, invitation,
   Share Link, and connection reference.
4. Validate sync sequences are monotonic per Vault.
5. Rebuild or verify `current_encrypted_vault_objects` from
   `vault_record_index` when the current projection is missing or suspect.
6. Start serving only after metadata and current encrypted state agree.

Migration policy:

- Migrations MUST be idempotent.
- Migrations MUST preserve signed event payloads byte-for-byte.
- Migrations MAY add derived indexes and projections.
- Migrations MUST NOT rewrite ciphertext, revision events, grant envelopes, or
  access-change events unless the migration is an explicit cryptographic
  rotation with user-visible consequences.

Retention:

- `vault_record_index` is the authoritative append log until a configured
  retention floor.
- `current_encrypted_vault_objects` is the latest-state projection and MUST be
  enough to bootstrap clients after old records are compacted.
- If a client cursor is older than the retained floor, the server returns
  `410 rebootstrap_required`.
- Retention MUST NOT remove current Folder Key Grants, current object state,
  live Share Links, live Invitations, live Mounts, or active access state.

## 9. Vault Record Index And Sync

The Vault Record Index is an ordered, Vault-scoped index of accepted encrypted
records.

Record types:

- `folder_object_revision`
- `folder_object_tombstone`
- `folder_key_grant`
- `vault_admin_access_change`

Each accepted record receives a monotonic server `sequence` per Vault.

SQLite tables are conceptually:

```text
vault_record_index(
  vault_id,
  sequence,
  record_event_id,
  record_type,
  folder_id,
  object_id,
  revision,
  actor_npub,
  client_created_at,
  payload_json,
  accepted_at,
  record_event_kind
)

current_encrypted_vault_objects(
  vault_id,
  folder_id,
  object_id,
  payload_json,
  revision,
  updated_at,
  deleted
)
```

Acceptance transaction:

1. Validate Nostr HTTP auth.
2. Validate actor can write/delete the target Folder.
3. Validate encrypted object envelope and signed revision/tombstone event.
4. If event id was already accepted for this Vault, return duplicate=true and
   the existing sequence.
5. Compute the next sequence.
6. Append payload to `vault_record_index`.
7. Project latest object state into `current_encrypted_vault_objects`.
8. Commit atomically.

Bootstrap:

```http
GET /_admin/vaults/{vaultId}/sync/bootstrap
```

Response:

```json
{
  "vaultId": "acme",
  "latestSequence": 42,
  "objects": [],
  "objectCount": 0,
  "controlRecords": [],
  "currentStateKind": "current_encrypted_vault_state"
}
```

Incremental pull:

```http
GET /_admin/vaults/{vaultId}/sync/records?after=42&limit=100
```

Response:

```json
{
  "vaultId": "acme",
  "afterSequence": 42,
  "latestSequence": 50,
  "records": [],
  "count": 8,
  "hasMore": false,
  "nextSequence": 50
}
```

Submit:

```http
POST /_admin/vaults/{vaultId}/sync/records
```

Revision body:

```json
{
  "recordType": "folder_object_revision",
  "folderId": "strategy",
  "objectId": "obj_0123456789abcdef",
  "baseRevision": 1,
  "keyVersion": 1,
  "cipher": "AES-256-GCM",
  "ciphertext": "<serialized encrypted envelope>",
  "revisionEvent": { "kind": 30078 }
}
```

Tombstone body:

```json
{
  "recordType": "folder_object_tombstone",
  "folderId": "strategy",
  "objectId": "obj_0123456789abcdef",
  "baseRevision": 1,
  "tombstoneEvent": { "kind": 30078 }
}
```

Visibility filtering:

- Bootstrap and incremental pulls return content records only for Folders the
  actor can read.
- Folder Key Grant control records are visible to the recipient and Vault
  Admins.
- Vault Admin Access Change control records are visible to Vault Admins.

Cursor behavior:

- Clients persist `latestSequence` in `working-tree-state.json`.
- If the server retention floor makes a cursor too old, the server returns
  `410 Gone` with `rebootstrap_required` and a bootstrap path.

Prototype boundary: current sync is pull-based. Realtime push, encrypted merge,
and full conflict resolution are not defined yet.

### 9.1 Sync Conflict Policy

Portable v1 uses optimistic concurrency, not CRDT merge.

Create:

- A create revision MUST use `baseRevision: null`.
- Creating an object id that already has a live or tombstoned current-state
  record is a conflict unless the submitted event id was already accepted.
- A create request with a non-null `baseRevision` returns `400`.

Update/move/delete:

- Update, move, and delete requests MUST include the client's observed current
  `baseRevision`.
- If `baseRevision` does not equal the server current revision for that object,
  the server returns `409`.
- If two clients edit the same object from the same base revision, the first
  accepted event advances the object. The second distinct event receives `409`.
- The server does not decrypt or merge content to resolve conflicts.

Duplicate events:

- If the same record event id has already been accepted for the same Vault, the
  server returns the existing sequence and marks the response duplicate.
- Duplicate handling is by event id, not by object id, hash, or body equality.
- Replaying an accepted event against another Vault is invalid because the
  signed payload and tags bind `vaultId`.

Client behavior:

- On `409`, the client MUST keep the local unsent change as unresolved, pull
  current state, and ask the user or Agent Runtime to reconcile.
- Reconciliation is a new update from the latest server revision.
- A client MUST NOT silently overwrite the server current revision after `409`.

Pagination:

- `limit` controls the maximum records returned in one incremental pull.
- Servers SHOULD cap large limits to an implementation-defined maximum.
- If `hasMore` is true, the client MUST continue pulling from `nextSequence`
  before considering the local mirror caught up.
- Clients MUST persist only sequences that have been fully applied locally.

Cursor expiry and rebootstrap:

- If `after` is below the retention floor, the server returns
  `410 rebootstrap_required`.
- The client MUST discard its incremental cursor, run bootstrap, rebuild the
  current encrypted state projection locally, and then resume incremental pull
  from the bootstrap `latestSequence`.
- Rebootstrap MUST preserve unresolved local edits until they are either
  reconciled or explicitly discarded.

Current-state projection:

- For each `(vaultId, folderId, objectId)`, the highest accepted non-deleted
  revision is current.
- A tombstone with the highest accepted revision marks the object deleted.
- Current-state projection MUST be derived from accepted records only.
- Visibility filtering is applied at read time. The projection may contain
  inaccessible records; responses must not reveal their encrypted object bodies
  to unauthorized actors.

## 10. Secure Object Routes

Primary secure object routes:

```http
PUT    /_admin/vaults/{vaultId}/folders/{folderId}/objects/{objectId}
GET    /_admin/vaults/{vaultId}/folders/{folderId}/objects/{objectId}
DELETE /_admin/vaults/{vaultId}/folders/{folderId}/objects/{objectId}
POST   /_admin/vaults/{vaultId}/folders/{folderId}/objects/{objectId}/move
```

These routes require Nostr auth and encrypted Folder Object payloads. The
server stores ciphertext only.

Rust hard-cut behavior: legacy plaintext file routes are not part of the
default server route surface. Readable Page content enters through trusted
Product Client, OKF import, or Vault Working Tree flows and is uploaded through
encrypted object routes.

## 11. Vault Invitations

A Vault Invitation lets one targeted npub become a Vault Member. It does not by
itself grant Folder Keys.

Invitation shape:

```json
{
  "id": "invitation-npub...",
  "userId": "npub...",
  "status": "pending | accepted | revoked",
  "initialFolderAccess": [{ "folderId": "strategy" }],
  "inviteCode": "invite-<32 hex chars>",
  "acceptPath": "/_admin/vault-invitation-links/invite-.../accept",
  "expiresAt": "2026-06-30T00:00:00Z"
}
```

Rules:

- Only Vault Admins invite organization members.
- Invitation Links are singleton handles for one npub. They are not reusable
  public join links.
- The target User must authorize with Nostr before viewing or accepting.
- Opening a link as the wrong User returns unavailable, not organization
  details.
- Accepting the link adds the target as a Vault Member and applies initial
  Folder Access metadata.
- Folder Key Grants for restricted Folders still need to be issued through the
  normal grant flow.

### 11.1 Invitation Lifecycle

Invitation Links are npub-bound, singleton, and single-use.

States:

- `pending`: link can be viewed only by the targeted npub and accepted before
  `expiresAt`.
- `accepted`: target npub has accepted; the link MUST NOT be accepted again.
- `revoked`: a Vault Admin has invalidated the link; the link MUST NOT reveal
  organization details to the target after revocation.
- `expired`: derived state when `expiresAt` is in the past; expired links MUST
  behave like unavailable links.

Lifecycle rules:

- A link is baked for one target npub at creation time.
- The accepting signer npub MUST equal the invitation `userId`.
- Accept is idempotent only for server retries of the same acceptance request.
  A completed accepted link cannot onboard a second session, second npub, or
  second membership action.
- Revocation before acceptance prevents membership creation.
- Revocation after acceptance disables the delivery handle only. Removing the
  accepted member is a separate membership/access change.
- Initial Folder Access metadata does not by itself prove the member can open a
  Folder. The required Folder Key Grants must exist and be openable.
- If grant issuance fails during accept, the server MUST surface setup
  incomplete instead of pretending the Folder is usable.

## 12. Sharing And Mounts

### 12.1 Shared Folder Source

Live sharing is source-first. The original Folder is converted in place by
setting:

```json
{ "sharedFolderSource": true }
```

The source Folder remains canonical:

- One source Vault Record Index.
- One current Folder Key.
- One access history.
- Writes to mounted content go to the source Vault, not the destination Vault.

### 12.2 Personal Folder Mount

A Personal Folder Mount is a visible reference for one User.

```json
{
  "id": "personal-mount-<hash>",
  "ownerNpub": "npub...",
  "sourceVaultId": "shared-vault",
  "sourceFolderId": "strategy",
  "displayName": "Strategy",
  "displayParentFolderId": "optional",
  "createdAt": "2026-06-23T00:00:00Z",
  "updatedAt": "2026-06-23T00:00:00Z"
}
```

Mount ids are deterministic:

```text
personal-mount-<first 8 bytes sha256(ownerNpub + "\n" + sourceVaultId + "\n" + sourceFolderId)>
```

A Personal Folder Mount does not grant Folder Access. The User must still be a
member of the source Vault for that Shared Folder and have a valid Folder Key
Grant.

Prototype boundary: the current server-side Personal Folder Mount validator
accepts organization source Vaults only. ADR 0043 and the domain model allow
vault-native Shared Folder Sources, including Personal Vault sources, but that
route still carries older organization-source assumptions.

### 12.3 Share Link

A Share Link is a singleton delivery handle for one recipient npub and one
Folder Key Grant.

```json
{
  "id": "share-link-<hash>",
  "vaultId": "acme",
  "folderId": "strategy",
  "recipientNpub": "npub-recipient",
  "createdByNpub": "npub-admin",
  "status": "pending | accepted | revoked",
  "acceptPath": "/_admin/share-links/share-link-.../accept",
  "expiresAt": "2026-06-30T00:00:00Z",
  "createdAt": "2026-06-23T00:00:00Z",
  "updatedAt": "2026-06-23T00:00:00Z"
}
```

The private server-side record also stores:

- `folderKeyGrant`
- `accessChangeEvent`

Rules:

- Only Vault Admins create Share Links.
- Current server validation supports Share Links from organization Vaults only.
- Link is scoped to `recipientNpub`.
- Recipient must authorize with the matching Nostr key.
- Accepting may optionally create a Personal Folder Mount.
- Accepting adds the recipient as a limited Vault Member if needed, grants
  restricted Folder Access, stores the Folder Key Grant, and marks the link
  accepted.
- Expired or revoked links cannot be accepted.
- Accepted links cannot be accepted again. Retries of the same acceptance
  request may be idempotent, but the delivery handle is consumed once the
  recipient's access and optional mount are created.
- Revoking a pending link prevents future acceptance.
- Revoking an accepted link disables the delivery handle only. Removing access
  requires Folder Access removal and Folder Key Rotation.

Hard cut: earlier prototype route behavior allowed accepted, unexpired Share
Links to be re-accepted by the same recipient. Portable v1 treats that behavior
as legacy and requires single-use npub-bound Share Links.

### 12.4 Shared Folder Invitation

A Shared Folder Invitation connects a source Shared Folder to a destination
Organization Vault.

```json
{
  "id": "shared-folder-invitation-<hash>",
  "sourceVaultId": "source",
  "sourceFolderId": "strategy",
  "destinationVaultId": "destination-org",
  "destinationAdminNpub": "npub-dest-admin",
  "createdByNpub": "npub-source-admin",
  "status": "pending | accepted | revoked",
  "currentKeyVersion": 1,
  "acceptPath": "/_admin/shared-folder-invitations/.../accept",
  "createdAt": "2026-06-23T00:00:00Z",
  "updatedAt": "2026-06-23T00:00:00Z"
}
```

Private server-side invitation record also stores:

- `folderKeyGrant` for the destination admin.
- `accessChangeEvent` signed by the source admin.

Creation rules:

- Source actor MUST be able to manage source Vault Folders.
- Source Folder MUST exist, be `sharedFolderSource: true`, and be
  `restricted`.
- Source Folder setup MUST be complete.
- Destination Vault MUST be an organization Vault.
- `destinationAdminNpub` MUST be an admin of the destination Vault.
- A current Folder Key Grant for the destination admin is required.

Acceptance rules:

- Only `destinationAdminNpub` can accept.
- Invitation must be pending.
- Server creates or reuses a Shared Folder Connection.
- Server creates or reuses an Organization Folder Mount.
- Server adds destination admin as a limited member of the source Vault if
  needed.
- Server grants source Folder Access and stores the Folder Key Grant.

### 12.5 Shared Folder Connection

```json
{
  "id": "shared-folder-connection-<hash>",
  "sourceVaultId": "source",
  "sourceFolderId": "strategy",
  "destinationVaultId": "destination-org",
  "destinationAdminNpub": "npub-dest-admin",
  "status": "active | revoked",
  "createdAt": "2026-06-23T00:00:00Z",
  "updatedAt": "2026-06-23T00:00:00Z"
}
```

Connection ids are deterministic:

```text
shared-folder-connection-<first 8 bytes sha256(sourceVaultId + "\n" + sourceFolderId + "\n" + destinationVaultId)>
```

Destination admins may update participating destination members for their
connection, but they can only manage source Folder Access for members of their
own destination Organization Vault. They must keep their own access because
issuing Folder Key Grants requires the current Folder Key.

### 12.6 Organization Folder Mount

```json
{
  "id": "organization-mount-<hash>",
  "organizationVaultId": "destination-org",
  "sourceVaultId": "source",
  "sourceFolderId": "strategy",
  "connectionId": "shared-folder-connection-...",
  "displayName": "Strategy",
  "displayParentFolderId": "optional",
  "createdByNpub": "npub-dest-admin",
  "createdAt": "2026-06-23T00:00:00Z",
  "updatedAt": "2026-06-23T00:00:00Z"
}
```

Mount ids are deterministic:

```text
organization-mount-<first 8 bytes sha256(organizationVaultId + "\n" + sourceVaultId + "\n" + sourceFolderId)>
```

Removing a Shared Folder Connection or Organization Mount revokes the
destination organization's source access and requires Folder Key Rotation when
access is removed.

### 12.7 Mounted Shared Folder Client Projection

A mounted Shared Folder is a client-visible entry that points at source Vault
content. It is not a copy in the destination Vault.

Projection rules:

- Personal Folder Mounts appear in the owner's sidebar/tree as a Folder-like
  entry at `displayName` and optional `displayParentFolderId`.
- Organization Folder Mounts appear in the destination Organization Vault's
  sidebar/tree as a Folder-like entry for destination members who have source
  Folder Access and an openable Folder Key Grant.
- Clients SHOULD visually distinguish mounted/shared Folders from native
  destination Folders.
- Opening a mounted Folder reads from the source Vault Record Index and source
  Folder Object state.
- Creating, editing, moving, or deleting a Page in a mounted Folder writes to
  the source Vault and source Folder. The destination Vault receives no copied
  Folder Object.
- Destination Organization Admins may manage which members of their own
  Organization participate in a Shared Folder Connection, but the resulting
  access is still source Folder Access plus source Folder Key Grants.
- If the source owner/admin revokes the connection, the mount becomes
  unavailable for all destination members and the source Folder requires Folder
  Key Rotation for future secrecy.
- If a destination member is removed from the connection, only that member
  loses access. Other destination members keep their mount if their source
  Folder Access and grants remain valid.
- If a mounted Folder cannot be opened because Folder Access or a grant is
  missing, the client should show a locked/setup-needed state, not an empty
  copied Folder.

## 13. Export

Encrypted Vault Export:

```json
{
  "version": "finite-vault-export-v1",
  "vault": {
    "id": "acme",
    "kind": "organization",
    "name": "Acme",
    "ownerUserId": "npub..."
  },
  "folders": [],
  "objects": [],
  "keyGrants": [],
  "accessState": {
    "admins": [],
    "members": [],
    "folders": [],
    "accessLog": []
  }
}
```

Export includes accessible encrypted Folder Objects and key grants visible to
the actor. Vault Admins may see broader access state and grants. Non-admins see
only accessible material and their own grants.

Opening an export:

1. Use recipient npub to filter Folder Key Grants.
2. Open each usable grant.
3. Verify access-change events where present.
4. Verify each revision/tombstone event.
5. Decrypt objects whose Folder Key is open.
6. Leave other objects opaque.

### 13.1 Readable OKF Export

Readable OKF Export is separate from Encrypted Vault Export. It contains
decrypted Markdown Pages and readable asset files for accessible Folders and
intentionally excludes Folder Keys, Folder Key Grants, encrypted sync state, and
inaccessible ciphertext.

An OKF bundle has this shape:

```text
okf-export/
  okf-vault.json
  content/
    <folder display path>/
      <page path>.md
  attachments/
    <folder display path>/
      <attachment paths>
  _wiki/
    index.md
    backlinks.md
    orphans.md
    stale.md
    tags.md
```

`okf-vault.json`:

```json
{
  "version": "finite-okf-vault-export-v1",
  "exportedAt": "2026-06-23T00:00:00.000Z",
  "exportedByNpub": "npub...",
  "sourceVault": {
    "id": "acme",
    "kind": "organization",
    "name": "Acme"
  },
  "folders": [
    {
      "folderId": "strategy",
      "displayPath": "General/Strategy",
      "access": "restricted",
      "omitted": false
    }
  ],
  "objects": [
    {
      "folderId": "strategy",
      "objectId": "obj_0123456789abcdef",
      "path": "content/General/Strategy/wiki/concepts/example.md",
      "contentType": "text/markdown",
      "contentHash": "<sha256 hex>"
    }
  ],
  "omissions": [
    {
      "folderId": "board",
      "displayPath": "Board",
      "reason": "inaccessible"
    }
  ]
}
```

Export rules:

- The bundle includes only content the actor can decrypt at export time.
- Inaccessible Folders MAY appear as omission entries, but their Page paths,
  Page titles, links, backlinks, and content MUST NOT appear.
- `contentHash` is the SHA-256 hash of the exported plaintext file bytes.
- Exported file paths are safe relative bundle paths.
- Encrypted-only metadata such as object ids may be included in the manifest to
  support round-trip diagnostics, but importing an OKF bundle MUST NOT require
  preserving object ids.

Markdown link conventions:

- Portable OKF Markdown SHOULD use ordinary relative Markdown links:
  `[Title](../concepts/title.md)`.
- Wiki links such as `[[Title]]` MAY be preserved in page bodies, but the OKF
  manifest MUST NOT rely on them as the only link representation.
- Folder-internal links SHOULD be rewritten to relative links when the target
  is present in the export.
- Links to omitted or inaccessible content SHOULD remain as plain text or
  unresolved links, not as leaked target paths discovered from the server.

Round-trip expectations:

- Encrypted Vault Export is the lossless encrypted portability format.
- OKF Export is a readable portability format for accessible plaintext.
- OKF export followed by OKF import into a new Vault SHOULD preserve Page
  content, relative paths, Markdown links, generated wiki reports, and
  attachment files for accessible content.
- OKF round-trip is not expected to preserve Folder ids, object ids, revision
  history, ciphertext, Folder Keys, grants, invitations, mounts, or inaccessible
  omissions.

### 13.2 OKF Import

OKF import turns readable Markdown back into encrypted Folder Objects in a
destination Vault.

Import rules:

- The importer MUST authenticate as a User who can write the destination
  Folders.
- The importer MUST encrypt imported content client-side before upload.
- The importer MUST create missing destination Folders only when the acting User
  has authority to create them and can create required Folder Key Grants.
- Default import behavior MUST NOT overwrite existing Pages silently.
- If an imported Page path collides with an existing Page path, the importer
  MUST either skip, create a copy with a clear suffix, or require an explicit
  overwrite decision.
- Imported content receives new object ids unless the import is an explicitly
  trusted restore flow using Encrypted Vault Export.
- Imported links are resolved best-effort against imported Page paths and
  existing accessible destination Page paths.
- Omission entries are advisory and MUST NOT create inaccessible placeholder
  Pages.

Conflict modes:

- `skip`: leave destination content unchanged and report skipped paths.
- `copy`: create unique suffixed paths such as `name imported.md`.
- `overwrite`: create normal encrypted update revisions over existing Pages;
  this mode requires explicit user/admin confirmation.

### 13.3 LLM Wiki And Agent Layer

The LLM Wiki is an agent-facing readable layer built from accessible decrypted
content. It is not a server-side plaintext index.

Conventions:

```text
AGENTS.md
_index.md
raw/
  assets/
wiki/
inventory/
datasets/
output/
```

- `AGENTS.md` gives local agent instructions for the Vault or Folder subtree.
- `_index.md` is a human/agent navigation page for a Folder or bundle.
- `raw/` contains source captures or immutable imported references.
- `raw/assets/` contains non-Markdown source Assets.
- `wiki/` contains curated synthesized wiki pages.
- `inventory/` contains source candidates, open questions, watch items, and
  next actions.
- `datasets/` contains manifests, schemas, samples, and query recipes.
- `output/` contains generated artifacts, reports, exports, or task outputs.

Agent discovery rules:

- Agents discover the nearest `AGENTS.md` by walking from the target Page path
  toward the Folder root, then the Vault root.
- Agents may read and write only materialized accessible Folders.
- Agent writes are User writes. They are signed, encrypted, synced, and audited
  as the acting User.
- Agents MUST NOT write decrypted content into `.finitebrain/encrypted-sync`.
- Agents SHOULD store non-Markdown source files under `raw/assets/` and create
  Markdown Source Notes that record provenance, content type, hash or extraction
  status when known, and links to synthesized Pages.
- Agents SHOULD query and cite Source Notes before treating an Asset blob as
  knowledge.
- Generated reports in `output/` SHOULD state when they were generated, by which
  acting npub, and from which accessible Folder scope.
- Reports MUST NOT include Page titles, Page paths, excerpts, backlinks, or
  tags from inaccessible Folders.
- `raw/`, `raw/assets/`, `wiki/`, `inventory/`, `datasets/`, and `output/` are
  ordinary paths inside accessible Folders. They do not imply special Folder
  Access semantics.

## 14. Server Route Surface

Admin and metadata:

```http
POST   /_admin/vaults
GET    /_admin/vaults/{vaultId}/metadata
GET    /_admin/vaults/{vaultId}/export
GET    /_admin/vaults/{vaultId}/search
```

Vault membership and invitations:

```http
POST   /_admin/vaults/{vaultId}/members
DELETE /_admin/vaults/{vaultId}/members/{memberNpub}
POST   /_admin/vaults/{vaultId}/admins
DELETE /_admin/vaults/{vaultId}/admins/{adminNpub}
POST   /_admin/vaults/{vaultId}/invitations
DELETE /_admin/vaults/{vaultId}/invitations/{invitationId}
POST   /_admin/vaults/{vaultId}/invitations/{invitationId}/accept
GET    /_admin/vault-invitation-links/{inviteCode}
POST   /_admin/vault-invitation-links/{inviteCode}/accept
```

Folders and access:

```http
POST   /_admin/vaults/{vaultId}/folders
POST   /_admin/vaults/{vaultId}/folders/{folderId}/finish-setup
POST   /_admin/vaults/{vaultId}/folders/{folderId}/access
DELETE /_admin/vaults/{vaultId}/folders/{folderId}/access/{targetNpub}
POST   /_admin/vaults/{vaultId}/folders/{folderId}/share-links
POST   /_admin/vaults/{vaultId}/folders/{folderId}/share-source
POST   /_admin/vaults/{vaultId}/folders/{folderId}/shared-folder-invitations
GET    /_admin/vaults/{vaultId}/organization-folder-mounts
```

Secure content and sync:

```http
PUT    /_admin/vaults/{vaultId}/folders/{folderId}/objects/{objectId}
GET    /_admin/vaults/{vaultId}/folders/{folderId}/objects/{objectId}
DELETE /_admin/vaults/{vaultId}/folders/{folderId}/objects/{objectId}
POST   /_admin/vaults/{vaultId}/folders/{folderId}/objects/{objectId}/move
GET    /_admin/vaults/{vaultId}/sync/bootstrap
GET    /_admin/vaults/{vaultId}/sync/records?after=0&limit=100
POST   /_admin/vaults/{vaultId}/sync/records
```

Sharing:

```http
GET    /_admin/share-links/{shareLinkId}
POST   /_admin/share-links/{shareLinkId}/accept
DELETE /_admin/share-links/{shareLinkId}
GET    /_admin/shared-folder-invitations/{invitationId}
POST   /_admin/shared-folder-invitations/{invitationId}/accept
DELETE /_admin/shared-folder-invitations/{invitationId}
PATCH  /_admin/shared-folder-connections/{connectionId}/members
DELETE /_admin/shared-folder-connections/{connectionId}
```

## 15. Error Semantics And Recovery

Important current errors:

- Missing/invalid Nostr auth: `403 valid Nostr authorization is required`.
- Missing Folder Key Grant in client: `No Folder Key grant found for <folder>`.
- Missing open Folder Key in client: `Folder Key is not open in the current session`.
- Existing object update with wrong base revision: `409 baseRevision does not match current folder object revision`.
- Creating with base revision: `400 baseRevision is not allowed for new folder object revisions`.
- Removing access without rotation: `400 folder key rotation is required when removing folder access`.
- Old sync cursor beyond retention: `410 rebootstrap_required`.

User-facing framing:

- "No access" means Folder Access or a usable Folder Key Grant is missing.
- "Finish setup" means Folder metadata exists but current Folder Key Grants are
  missing for an empty Folder.
- "Reconnect signer" means no NIP-07/NIP-44 capable signer is available, or the
  signer key does not match the acting User.

## 16. Security And Privacy Invariants

Compatible implementations MUST preserve these invariants:

- Page plaintext is produced only inside a trusted Vault Client or Agent
  Runtime.
- The server never needs a Folder Key to authorize, sync, store, or export
  encrypted state.
- Every Folder has an independent Folder Key.
- Folder Key Grants are addressed to User npubs, not devices.
- Folder Access is binary.
- Removing access requires Folder Key Rotation for future access control.
- Folder Key Rotation does not promise retroactive erasure of content already
  decrypted, copied, exported, or retained as old ciphertext.
- Server-visible metadata may include Vault names, Folder names, Folder
  hierarchy, membership, access lists, current key versions, object ids,
  ciphertext sizes, revisions, timestamps, and author npubs.
- Server-visible metadata MUST NOT include Page paths, Page titles, links,
  backlinks, or content for secure object records.
- Share Links and Vault Invitation Links are delivery handles, not independent
  cryptographic permissions.
- Folder Mounts are display references, not access grants and not sync copies.

### 16.1 Security Hardening Requirements

Replay resistance:

- Nostr HTTP authorization events MUST expire after the configured clock-skew
  window. Portable v1 uses 60 seconds.
- The `u`, `method`, and `payload` tags bind the authorization event to one
  request target and body.
- Servers SHOULD reject already-seen authorization event ids within the skew
  window for state-changing requests.
- Signed revision, tombstone, access-change, grant, invite, and share payloads
  MUST bind Vault id and relevant Folder/object/user ids.

Clock skew:

- Servers MAY allow at most 60 seconds of skew for HTTP authorization.
- If skew validation fails, the error SHOULD tell the client to retry signing
  with current time.

NIP-07 boundary:

- The browser extension/provider is trusted to protect the User's Nostr secret
  key and perform correct NIP-44 operations.
- FiniteBrain clients MUST treat provider output as authority from the active
  User, but SHOULD display the active npub whenever access-sensitive operations
  are performed.
- A signer key mismatch is an auth failure, not a recoverable Folder Key issue.

Local plaintext/XSS:

- Decrypted Page content, opened Folder Keys, local search indexes, and graph
  indexes exist inside the trusted client or Agent Runtime.
- Browser clients MUST treat XSS as a plaintext compromise.
- Opened Folder Keys are Session Folder Keys. Trusted clients and Agent
  Runtimes MUST NOT persist raw Folder Keys in localStorage, browser databases,
  working-tree control state, logs, diagnostics, OS credential stores, or any
  other durable client state.
- A new process MUST reopen encrypted Folder Key Grants through the acting
  Member Identity's signer and fail closed as locked when that signer or a
  valid grant is unavailable.
- Legacy Agent State containing raw Folder Keys is a hard cut: upgraded clients
  MUST scrub the raw keys, clear stale unlocked status, and MUST NOT retain a
  legacy fallback that uses them.
- Scrubbing opened raw keys MUST preserve encrypted Folder Key Grants and any
  Recovery Principal or Recovery Set material needed to restore access; this
  hard cut removes redundant plaintext, not recovery authority.

Nonce uniqueness:

- AES-GCM nonces MUST be random 12-byte values generated for each encryption.
- A client MUST NOT reuse the same `(Folder Key, nonce)` pair.
- Folder Key Rotation creates a new key version and resets the nonce uniqueness
  domain.

CSRF/CORS/cookies:

- The Nostr authorization header is the primary auth mechanism.
- The mirrored short-lived cookie is a browser compatibility helper and MUST be
  scoped to the specific method, URL, and body hash.
- Server CORS policy SHOULD allow only trusted client origins.
- State-changing routes MUST require Nostr authorization, not cookie presence
  alone.

Payload and rate limits:

- Servers SHOULD enforce maximum request body sizes for encrypted object
  writes, grant batches, imports, exports, and sync pulls.
- Servers SHOULD rate-limit auth failures, invitation/share creation, grant
  generation, and large sync/export operations per actor and per Vault.
- Error responses SHOULD avoid revealing whether inaccessible Page paths or
  Page titles exist.

Key loss and recovery:

- Server-side recovery of lost Folder Keys is not possible.
- Recovery requires another authorized recipient with an openable current
  Folder Key Grant, a user-held key backup, or a new Folder Key created through
  an explicit recovery/rotation process for content that can still be read.
- Finish Setup repairs missing grants for empty/setup-incomplete Folders; it
  does not decrypt or recover existing encrypted content.
- A first-slice durable Folder MUST have an independently recoverable current
  Folder Key Grant or encrypted user-held key backup before accepting content.
  Restoring server ciphertext alone does not satisfy recovery.
- The recovery test MUST delete the primary signer/key state and prove that the
  Recovery Principal can reopen the Folder on an empty replacement client;
  otherwise the Folder is a non-durable preview.

### 16.2 Search And Index Privacy

Search, backlinks, graph indexes, and LLM Wiki reports are derived from
decrypted content.

Privacy rules:

- The server MUST NOT build plaintext search over secure Folder Objects.
- Browser/client search indexes MUST include only content the active User can
  decrypt.
- Graph/search visibility filtering MUST be applied before rendering, export,
  agent reports, or suggestions.
- Inaccessible content MAY contribute server-visible metadata such as Folder
  name, Folder id, object id, ciphertext size, revision, author, and timestamp,
  but MUST NOT contribute Page title, Page path, links, backlinks, tags, or
  snippets.
- Local search indexes are plaintext-sensitive client state. Clearing a session
  SHOULD clear or invalidate decrypted search/index caches for Folders whose
  keys are no longer open.
- Mounted Shared Folders are indexed from the source content only after the
  active User has source Folder Access and an open Folder Key.

## 17. Prototype Boundaries

These behaviors are part of the current prototype and should be understood by a
porting implementation, but they are not final product guarantees:

- Session Folder Keys are in-memory only.
- Sync is pull-based, not realtime.
- Conflict resolution is limited to base-revision checks.
- Public Nostr relay storage is deferred.
- Server-side key recovery is not implemented.
- The system is not claiming production-grade cryptographic hardening yet.

## 18. Clean-Room Reimplementation Checklist

A compatible implementation in another language should implement, in order:

1. Nostr primitives:
   - NIP-19 npub encode/decode.
   - Nostr event id serialization and Schnorr signature validation.
   - Event signing for kind `27235` and `30078`.
   - NIP-44 encrypt/decrypt for provider-compatible grant wrapping.
2. Folder crypto:
   - AES-256-GCM raw key import/export/generation.
   - `finite-folder-object-v1` encrypt/decrypt with the specified AAD.
   - Exact ciphertext string hashing for revision events.
3. Folder Key Grants:
   - NIP-59-shaped rumor/seal/gift-wrap creation.
   - Grant opening and context validation.
4. Domain model:
   - Vault, Folder, Member, Admin, Invitation, Share Link, Mount, and Access
     Log data shapes.
   - Folder hierarchy validation and path decoration.
   - Required recipient computation.
5. Server auth:
   - Kind `27235` authorization validation.
   - Actor npub derivation.
   - Access checks for personal, organization, all-member, restricted, owner,
     and admin-only Folders.
6. Secure object APIs:
   - Create/update/delete/move validation.
   - Revision and tombstone signing checks.
   - Base revision conflict checks.
7. Sync:
   - Append accepted records with monotonic per-Vault sequence.
   - Idempotency by event id.
   - Current encrypted state projection.
   - Bootstrap and incremental filtering by actor access.
8. Client working tree:
   - Open current Folder Key Grants on Vault load.
   - Materialize accessible Folders.
   - Store opaque inaccessible ciphertext.
   - Detect and prepare local file changes.
9. Admin flows:
   - Atomic Folder creation with grants.
   - Finish Setup for empty legacy Folders.
   - Grant access without rotation.
   - Remove access with rotation.
   - Add/remove admins and members with required grants/rotations.
10. Sharing:
    - Singleton Vault Invitation Links.
    - Singleton npub-scoped Share Links.
    - Shared Folder Source conversion.
    - Personal Folder Mounts.
    - Shared Folder Invitations, Connections, delegated member management, and
      Organization Folder Mounts.
11. OKF portability:
    - Readable OKF Export bundle layout.
    - `okf-vault.json` manifest.
    - Markdown link rewriting and omission handling.
    - OKF Import conflict modes.
12. LLM Wiki and agent layer:
    - `AGENTS.md` discovery.
    - `_index.md`, `raw/`, `raw/assets/`, `wiki/`, `inventory/`, `datasets/`,
      and `output/` conventions.
    - Generated report visibility filtering.
13. Operations and compatibility:
    - Backup/restore of SQLite metadata, sync, grants, sharing, and mounts.
    - Retention and `rebootstrap_required`.
    - Canonical serialization and fixture test vectors.
    - Portable v1 hard-cut versioning and unknown-field behavior.

## 19. Versioning And Compatibility

Portable v1 is a hard-cut compatibility line.

Hard-cut rule:

- New compatible implementations target this document, not earlier loose
  prototype behavior.
- Earlier prototype behaviors such as plaintext file routes, unauthenticated
  metadata shortcuts, reusable accepted Share Links, and JSON-only metadata
  storage are outside the Rust Portable v1 hard-cut surface.
- Implementations MAY build explicit import or migration tooling for old data,
  but MUST NOT make those paths the default secure flow.

Version strings:

- Version strings embedded in encrypted AAD, encrypted envelopes, signed
  payloads, exports, working-tree state, and manifests are part of the wire
  contract.
- A breaking change to any signed or encrypted payload shape requires a new
  version string.
- Portable v1 version strings use the existing `finite-...-v1` and
  `finite-okf-...-v1` forms shown in this spec.
- Implementations MUST reject unknown major versions in signed/encrypted
  payloads unless an explicit migration path exists.

Additive compatibility:

- Readers SHOULD ignore unknown top-level fields in metadata responses,
  manifests, export records, mount records, and invitation/share records when
  the known required fields are valid.
- Readers MUST preserve unknown fields when round-tripping metadata records
  they do not fully understand.
- Writers MUST NOT add fields inside signed event `content` unless those fields
  are included in the canonical serialization and all verifiers know how to
  validate them.
- Writers MUST NOT change JSON property order for cryptographic hash inputs.

Route compatibility:

- Secure object, grant, invitation, share, and sync routes are the normative
  Portable v1 route surface.
- Metadata and Vault creation routes are protected API routes and derive actor
  identity from Nostr authorization.
- Plaintext file compatibility routes are not part of the Rust Portable v1
  default route surface.

Migration compatibility:

- Migrations MAY add indexes, projections, and manifests.
- Migrations MUST NOT rewrite signed events, ciphertext strings, grant
  envelopes, or historical sync payloads.
- If a future version needs different crypto, it should be introduced as a new
  Folder Key version and object/envelope version, not by mutating existing v1
  objects in place.

Feature negotiation:

- A server SHOULD expose the Portable v1 feature set in metadata or health
  output before clients rely on optional flows such as OKF Import, Share Links,
  or Organization Folder Mounts.
- Clients SHOULD fail closed when a server does not advertise support for a
  security-sensitive feature.

## 20. Source Map

The current implementation source of truth is spread across:

- `CONTEXT.md`: domain language and invariants.
- `docs/adr/0001-*.md` through `docs/adr/0006-*.md`: accepted workspace,
  storage, crypto-boundary, Product Client, graph/replay, and OKF import
  decisions.
- `crates/finite-brain-core/src/lib.rs`: domain model, validation, signed
  payload checks, encrypted Folder Object helpers, and core bootstrap rules.
- `crates/finite-brain-core/src/portability.rs`: OKF export/import planning,
  local search/agent discovery helpers, and Vault Working Tree intent shaping.
- `crates/finite-brain-store/src/lib.rs`: SQLite schema, migrations,
  transaction boundary, grants, sync append log, current projection,
  invitations, shares, mounts, backup/rebuild tests, and retention behavior.
- `crates/finite-brain-server/src/lib.rs`: HTTP route catalog, request/response
  types, server-side policy orchestration, and Product Client asset serving.
- `crates/finite-brain-server/src/protected_routes.rs`: Nostr HTTP auth
  validation, replay rejection, protected-route rate limits, and CORS allowlist
  behavior.
- `crates/finite-brain-server/src/product-client.js`: first-party Product
  Client workflow, NIP-07/NIP-44 bridge, local Folder Key opening, decrypt/edit
  loop, Graph View, Graph Replay, OKF import execution, and local sync merge.
