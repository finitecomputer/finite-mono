# FiniteBrain Context

## Glossary

### FiniteBrain Portable v1

The hard-cut implementation target for the Rust rebuild. It is defined by
`docs/specs/finitebrain-portability-spec.md` and covers Vaults, Folders, Folder
Objects, Folder Key Grants, sync, sharing, OKF import/export, and compatibility.

### FiniteBrain Policy

Application-specific behavior for Vaults, Folders, access, sync, storage,
sharing, OKF, hardening rules, the Product Client, and the Smoke UI.
FiniteBrain Policy belongs in the `finite-brain` workspace, not in
`finite-nostr`.

### Reusable Nostr Primitive

A generic Nostr operation that can be reused across Finite repos without
knowing about FiniteBrain Vaults or Folders. Examples include NIP-19 identity
encoding, event serialization and verification, NIP-44 encryption adapters,
NIP-59 gift-wrap helpers, and NIP-98-style HTTP authorization helpers.

### Smoke UI

A development-only HTML/CSS interface served by the Rust app for local
end-to-end verification. It is not the product client. It exists to inspect
Vaults, Folders, encrypted objects, sync state, grants, invitations, shares,
and mounts while the Rust core and server mature.

### Product Client

The trusted browser experience a Member Identity's controller uses to open a
Vault, connect a Brain Identity Provider, open Folder Key Grants, decrypt accessible Folder Objects,
materialize Pages, edit content, sync changes, run local search/graph indexes,
and perform OKF import/export. Unlike the Smoke UI, the Product Client owns the
normal member workflow.

### Brain Identity Provider

The versioned, product-facing capability contract through which the Product
Client uses an acting Member Identity. FiniteBrain defines the allowed typed
intents, such as identifying the Member, authorizing a Brain-bound request or
revision, and opening or wrapping an appropriately scoped Folder Key Grant.
FiniteBrain retains ownership of Vault, Folder, content-crypto, and grant
policy. It also owns its own hosted-now/native-later adapter. The contract
never exposes a raw identity secret or generic sign/decrypt operation to
Product Client code. Hosted, Electron, and iOS adapters may implement the same
contract for one User Nostr Identity despite their different custody models.
Finite Identity supplies key-storage and lifecycle primitives; it does not own
a universal product adapter. The adapter opens a validated Folder Key Grant,
but the Product Client holds the resulting Session Folder Key and continues to
read, write, encrypt, and decrypt Brain content itself.
Only the official Brain Product Client may invoke the adapter; ordinary
dashboard pages, Sites content, and embedded frames never receive that
capability.

### Dashboard-Aligned Product Theme

The Product Client's presentation language derived from the Finite dashboard:
warm neutral surfaces, blue product accents, Funnel typography, restrained
depth, rounded controls, and the dashboard's system-driven light and dark
appearances. It changes presentation without changing the Product Client's
Vault workspace layout, security state, or member workflows.

### Product Client Spine

The minimum trusted-client workflow that later client features build on:
connect the acting Member Identity's Brain Identity Provider, load Vault state, open current Folder Key
Grants, decrypt readable Pages, edit one Page, encrypt and write the Page back
as a signed revision, and pull/apply sync records without losing unresolved
local edits.

### Member Identity

A Nostr `npub` that can hold Vault Membership, receive Folder Access, and open
Folder Key Grants. FiniteBrain does not classify whether a human, agent, shared
client, or several clients control it; separate keypairs are separate Member
Identities. A product or Agent Runtime may provision and label separate
keypairs, but that client-side policy does not create a different FiniteBrain
authorization class. In particular, an Agent Principal Key receives no Brain
access merely because it belongs to the same Project or dashboard account as a
user; an authorized Member must explicitly grant it the required access and
Folder Key Grants. In a Personal Vault, a limited Member Identity may receive
only explicit restricted-Folder access; it never becomes the Vault owner or an
admin.

### User Nostr Identity

The human-controlled Nostr `npub` used across Hosted Web, Electron, and iOS.
In FiniteBrain it is a Member Identity and receives the appropriate Vault
ownership or membership, Folder Access, and Folder Key Grants. Hosted Web uses
it through a server-held Brain Identity Provider; Electron and iOS use the same
identity from protected local storage. The custody difference does not create
another Brain identity. Account Auth may authorize a Hosted Web session but does
not grant Brain access. A User Nostr Identity remains distinct from every Agent
Principal Key. In the first hosted phase, the Finite Chat Hosted Device is the
user-facing setup and custody entry point; Brain's adapter owns only
Brain-specific operations. Hosted Brain assumes that setup already exists. If
it does not, Brain fails closed with a basic setup-required state and never
creates another User Nostr Identity. This is a Greenfield boundary: Brain
carries no legacy Vault or user-key migration path into the first release.

### Personal Vault Agent Access

The explicit, revocable way a user's distinct Agent Principal Key works in that
user's Personal Vault. The User Nostr Identity remains the Vault's sole owner.
The agent is a limited Member Identity with its own Folder Key Grants, starting
with access only to the dedicated restricted **Agent Workspace Folder**. It may
read and write that Folder, but cannot see owner-only folders, change Vault
access, or use the user's Brain Identity Provider. The owner may later grant
the agent another restricted Folder deliberately; Project or dashboard
association never creates that access.

### Personal Vault Bootstrap Authorization

A short-lived, single-use Brain authorization created by the user's explicit
Chat request to set up Brain. It is bound to that User Nostr Identity, one Agent
Principal, and the initial Agent Workspace Folder only. A successful agent-first
setup consumes it atomically while creating the user-owned Personal Vault,
durable delegated access, and both parties' Folder Key Grants. It is not a
user-facing second approval and cannot later create another Vault or broaden the
agent's access.

### Agent Workspace Folder

The initial dedicated restricted Folder in a Personal Vault that both the owner
and the explicitly authorized Agent Principal can use. It is the narrow shared
starting place for an agent, not a second Vault and not a change to Personal
Vault ownership.

In Hosted Web, the selected Runtime's Managed Agent Email and display name may
prefill the owner's pairing input, but that navigation context carries no
authority. Brain resolves the email through Finite Identity and grants the
resolved Agent Principal Key only after the owner explicitly pairs it from an
unlocked Personal Vault. The raw `npub` is an advanced fallback, not the
primary user experience. After pairing, the Agent Principal discovers the
user-owned Personal Vault through the signed visible-Vault list and opens its
restricted Folder in a durable Vault Working Tree below the Runtime's
`/data/workspace` boundary.

### Local Data Security Baseline

The FiniteBrain-wide policy for how trusted clients and Agent Runtimes handle
local secret material, decrypted content, derived plaintext state, retention,
and egress. It applies regardless of which UI or editor provides the local
experience.

### Session Folder Key

A Folder Key opened for one running trusted-client session. It is not durable
local state and must be reopened from an encrypted Folder Key Grant when a new
session needs it.

### Session Lock

A trusted-client state in which Session Folder Keys and temporary plaintext
state are unavailable and automatic grant reopening is blocked until the
Member explicitly resumes the grant-opening flow. A Session Lock hides client
content but does not claim to erase a separately created Vault Working Tree.
In Hosted Web, explicitly opening Brain from the authenticated dashboard is
the Member's Resume action and may automatically reopen valid Folder Key
Grants. After a lock, the Member must explicitly open Brain again; Account Auth
selects the hosted session but remains neither Brain authority nor a signer.
The browser Product Client applies the same lock before page navigation or
back/forward-cache suspension and whenever a signed event no longer matches the
Member Identity connected for the current session.
In Hosted Web, Account Auth logout or session expiry also locks the Product
Client and invalidates the Brain hosted-adapter session. Locking never revokes
the underlying Membership, Folder Access, or Folder Key Grants. It also does
not stop an Agent Runtime using its distinct Agent Principal Key and explicit
Folder access; stopping that agent requires explicit access revocation and the
required Folder Key rotation.
A newly delivered invitation fragment is handled as a one-shot pre-session
capability: the client removes it from browser history immediately, holds it in
memory outside the locked content session, and imports it only after explicit
Resume. Explicit Lock, Vault switching, or a failed Resume discards it.

### Ephemeral Client Plaintext

Decrypted content and derived readable state held by a browser or desktop
client only while its session is unlocked. It is not retained as durable local
state after the session ends.

### Encrypted Recovery State

Durable client-side ciphertext that preserves unsent work or other restart
state without retaining readable plaintext. It becomes readable only after the
acting Member Identity unlocks the relevant Folder again.

### Plaintext Egress

Any transfer of decrypted content or content-derived readable metadata beyond
the Trusted Device Boundary. FiniteBrain's cryptographic authorization ends at
decryption; first-party clients deny automatic Plaintext Egress, while a Member
Identity's controller remains responsible for explicitly initiated exports and
for the behavior of third-party clients.

### Paused Vault Working Tree

A Vault Working Tree whose FiniteBrain sync, signing, and automatic Folder Key
opening are stopped while its existing plaintext files remain on the Trusted
Device. _Avoid_: Locked Working Tree.

### Vault Working Tree Removal

The explicit deletion of a Vault Working Tree's local plaintext projection.
It does not claim secure erasure from backups, snapshots, or storage history.

### Trusted Device Boundary

The local OS account and storage boundary trusted to hold a Member Identity's
persistent secret and authorized plaintext. Obtaining that secret is a complete
trusted-client compromise for the Member Identity, not a failure contained to
one Folder or Finite product.

### Folder-scoped LLM Wiki

The FiniteBrain knowledge model. A Vault is a namespace of many LLM wikis, and
each Folder is the enforceable wiki scope because Folder Keys and Folder Access
define who can read it. Folder-local `_index.md`, `config.md`, and `log.md`
describe only that Folder. Root/global indexes must not leak private Folder
titles, summaries, sources, or activity.

### Asset

An encrypted non-Markdown source file stored inside a Folder, such as a PDF,
image, audio file, or other blob. An Asset is evidence or source material; it
is not the primary LLM Wiki knowledge surface.

### Source Note

A Markdown Page that describes one captured source with provenance, extraction
status, and human or agent-readable notes. Source Notes are the readable handles
that LLM Wiki pages cite when synthesizing knowledge from raw material.

### Asset Source Note Pair

The expected pairing for non-Markdown source material: one Asset under
`raw/assets/` plus one Source Note that explains and cites that Asset. The
Asset preserves the original evidence, while the Source Note lets humans,
agents, search, and graph flows reason over it.

### Graph View

A Product Client view over the acting Member Identity's decrypted accessible Pages. It
renders Page nodes and Page relationships only after Folder Keys are open and
visibility filtering has been applied.

### Graph Replay

A Product Client playback of graph/index changes derived from the client's
applied sync history and decrypted Page index. It is not a server-side graph
event log.

### OKF Import Execution

A Product Client workflow that parses readable OKF, plans import conflicts,
opens destination Folder Keys, encrypts imported Pages client-side, signs
Folder Object revisions, and uploads those revisions through normal secure
object routes. The Rust server does not parse readable OKF or receive
plaintext Page content during import.

### Vault Working Tree

A local agent-facing file projection built from already-decrypted accessible
Pages. It materializes readable Folders as Folder-scoped LLM wiki roots with
local `AGENTS.md` or `HUMANS.md` when present, `_index.md`, `config.md`,
`log.md`, `raw/`, `wiki/`, `inventory/`, `datasets/`, and `output/`
conventions. It is an explicitly created persistent plaintext copy inside the
Trusted Device Boundary, remains until its controller removes it, and is
private to the controlling OS account at its root and FiniteBrain control-state
boundary. It stores only safe locked metadata for inaccessible Folders and maps
file changes back into Product Client encrypted-object write, move, and delete
intents.

### Agent CLI

The terminal control surface for a trusted Agent Runtime working inside a Vault
Working Tree. It explains and controls identity, local daemon state, automatic
sync health, blocked edits, activity, and access reasons while the controller
reads and writes ordinary files; each operation opens the Folder Key Grants it
needs without creating a durable CLI unlock state.

### Agent Sync Daemon

The resident trusted-client process that watches a Vault Working Tree, opens
available Folder Keys for the acting Member Identity, detects file changes,
syncs with the server, and records blocked states that require controller
resolution.

### Local Agent Signer

A trusted signer available to the Agent Runtime instead of a browser Brain
Identity Provider. It exposes the same conceptual abilities the Product Client
needs: identify the acting npub, sign FiniteBrain events, and perform NIP-44
encryption and decryption for Folder Key Grant handling; its npub is an
ordinary Member Identity with no agent-specific authorization semantics. It
opens only Folder Key Grants addressed to that Agent Principal Key and never
uses the user's Brain Identity Provider or User Nostr Identity.

### Recovery Principal

A distinct, narrowly authorized Principal whose Folder Key Grants provide an independent recovery path when the primary human or agent key is unavailable.

### Email Access Delegation

A revocable, Brain-owned product authorization connecting one verified email
Principal's user-approved account context to one Agent Principal. It records
the delegated relationship for audit and revocation but does not make the two
the same Principal or convey Folder Keys. In a Personal Vault it authorizes
only the explicit limited-member and Folder Key Grant setup described by
Personal Vault Agent Access; it is not itself content access. It is the durable
relationship after setup, distinct from the one-use Personal Vault Bootstrap
Authorization.

### Email Invite Bootstrap

A temporary email-address invitation state where email proof authorizes the
claim, an out-of-band invite secret unlocks NIP-59-shaped gift-wrapped
bootstrap material, and accepted access becomes durable only after it is bound
to a User npub.

### Invite Secret

The high-entropy client-only secret carried outside the server-visible invite
code, typically in the URL fragment. For Email Invite Bootstraps, this is the
secret material needed to use the Invite Unwrap Key. It unlocks bootstrap
material only after the recipient proves the invited email. It must never be
sent through server-visible channels such as query parameters, request bodies,
server logs, server-side mailer payloads, email bodies, email tracking links,
analytics redirects, or stored database fields.

### Invite Unwrap Key

A temporary Nostr/secp256k1 keypair generated for an Email Invite Bootstrap.
The public key receives the NIP-59-shaped gift-wrapped bootstrap payload; the
private key is carried client-side as an Invite Secret and must not be stored
server-side. This key is a bearer unwrap capability, not a User identity,
member identity, or permission principal.

### Invite Unwrap Proof

A Nostr event signed by the Invite Unwrap Key during Email Invite Bootstrap
Claim. It proves possession of the client-only Invite Secret without sending
the secret to the server, and binds the claim to the invite code, Vault,
invited email, claimant npub, bootstrap payload hash, and email proof
timestamp.

### Invite Instructions

Agent-readable guidance for a Vault Invitation, analogous to Sites `llms.txt`
but split by proof level for Brain's encrypted access model.

### Public Invite Instructions

Unauthenticated Invite Instructions that disclose only generic claim workflow
guidance. They exclude invited email, Vault identity, Folder identity, access
scope, claim state, Folder Keys, and bootstrap plaintext.

### Post-Proof Invite Instructions

Invite Instructions returned only after the invited email is proven through the
Identity Authority. They may disclose the scoped workflow details needed to
claim, open, and sync the Vault, including human-readable Vault and Folder
names, but never Folder Keys or bootstrap plaintext.

### Email-Targeted Vault Invitation

A Vault Invitation addressed to an email instead of a known Native Principal
npub. In v1,
external email-shaped targets use an Email Invite Bootstrap even if they have
prior email-only proof; only concrete npub/hex targets or active Finite VIP
NIP-05 bindings use the normal npub-bound path. Email targets belong to
invitation flows; direct permission mutations remain for known User npubs. Any
invited email must prove control through the Identity Authority. Invitation
proof authorizes only the invitation claim; it does not create or rebind a
Finite VIP NIP-05 Principal Link unless the claimant separately and explicitly
uses the identity-link flow as the same Principal.

### Email Invite Bootstrap Claim

The acceptance act that grants the invitation's scoped access to the claimant
Native Principal npub after email proof, using the bootstrap material to create
durable npub-addressed access without requiring the inviting admin to come back
online. This is product authorization, not global identity equivalence. Claim
is all-or-nothing: Brain must verify email proof, consume the pending bootstrap,
record the claimant npub, create membership/access metadata, and insert every
required durable Folder Key Grant in one atomic operation.

### Email Invite Bootstrap Authorization

An admin-signed authorization for a future email recipient whose User npub is
not known yet. It fixes the invited email, Vault, authorized Folder scope,
Folder key versions, Invite Unwrap Key, bootstrap payload hash, expiry, and
single-use claim bounds that a later Email Invite Bootstrap Claim must match.
For email-targeted Vault Invitations, the authorized Folder scope includes
current all-members Folders because the accepted recipient becomes a Vault
Member.

### Claim-Authorized Folder Key Grant

A durable Folder Key Grant created by an invited recipient after a valid Email
Invite Bootstrap Claim. The inviting admin authorized the access, while the
recipient's User npub finalized the encrypted grant. The grant is valid only
within the pending invitation's authorized email, Vault, Folder, key-version,
expiry, and single-use claim bounds.

### Blocked Sync State

A local condition where automatic sync cannot safely complete without
resolution. Examples include missing auth, missing Folder Key Grant, locked
Folder, stale base revision conflict, revoked access, unavailable server, or a
working-tree change that cannot be mapped to a secure object intent.

### Hard Cut

A compatibility boundary where FiniteBrain does not carry legacy route,
storage, client, or migration behavior forward. Hard-cut work may import data
through explicit new-format flows such as OKF, but it does not preserve old v1
runtime compatibility as a feature requirement.
