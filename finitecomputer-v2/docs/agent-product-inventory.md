# Selected-agent Brain and Sites inventory

The dashboard's Brain and Sites pages read the selected Agent Runtime's product
access, not the human account's memberships. Skyler's table/list designs remain
the presentation baseline. These are metadata reads, without inference, except
the Brain roster changes below, which the Agent signs.

## Authority and ownership

Every entry/manual Refresh obtains a fresh account-authorized native Hermes grant
through the existing `/api/agents/{runtime}/hermes-access` flow. The browser calls
these fixed native GET routes:

| Product owner | Native route | Local supported read |
| --- | --- | --- |
| FiniteBrain | `/api/plugins/finite-brain/overview` | `fbrain brain list --json --existing-identity`, then `fbrain brain metadata --brain=ID --json --existing-identity` |
| FiniteBrain | `/api/plugins/finite-brain/identities/{brain_id}` | `fbrain brain list --json --existing-identity` (the Agent must already belong to the Brain), then `fbrain access list --brain ID --json` |
| Finite Sites | `/api/plugins/finite-sites/overview` | `fsite project list --output json --existing-identity` |

Native Hermes authenticates before plugin routing and respects disabled plugins.
The product plugins live under each product's `integrations/hermes/` directory.
The shared bounded subprocess reader is sealed into Hermes' Python environment;
it contains no product inventory or authorization policy. Both full and minimal
Hermes packages include the product plugins. No mutable plugin installation,
Core product API/storage, Management Pipe feature reports, or shell endpoint is
introduced. Requests accept no profile, identity, server, command or query input;
the identities route takes only a Brain ID path segment.

## Brain roster changes

When the selected Agent is an Admin of an Organization Brain, the roster lets
the person invite someone, revoke a pending invitation, and remove a Member or
Admin. The Agent signs each change with its own key; the person is not a Brain
member through the dashboard. The identities response advertises them in an
optional `changes` field (`actingNpub`, `actions`); an older Agent omits it, and
the dashboard then shows a short "update" line instead of buttons.

The plugin adds `invitations/{brain_id}` (list, create, revoke) and
`removals/{brain_id}` (start, status) under `/api/plugins/finite-brain/`.

Invitations are key-addressed Member invitations with the default expiry: the
target is an npub or a public NIP-05 name, resolved once by Brain, and the
invitee accepts with the invited key. Email bearer links, roles, Folders and
expiry are not offered here.

Before any command that could mint an identity, the plugin proves with
`brain list --existing-identity` that the Agent is an Admin of that
Organization Brain; a removal also needs a fresh `access list` showing the
target as a Member or Admin and not the Agent's own key. Brain still enforces
every rule. These commands read flags only as `--flag value` (an equals form is
silently ignored and `invite brain list` would then read the Agent's own
inbox), so every value must match a pattern that cannot start with `-`. Bodies
are small JSON objects with exactly the named field.

Changes are never retried by the browser or the plugin, and one change runs at
a time per Agent (others answer `409 busy`). Invite and revoke finish within the
read deadline. Removal re-encrypts the removed key's Folders (up to 64 MiB per
request), so it runs as a background job the browser polls; a job lost to a
Hermes restart reads as `unknown`, and the refreshed roster is the truth.
A nonzero CLI exit or lost response also leaves the outcome `unknown`, even
after a confirmed Admin demotion: removal may have committed before a later
read failed. Invite and revoke use the same rule. Refresh before deciding
whether another change is needed.
Answers carry only a fixed error code, never CLI output or receipts.

The CLI signer remains in the Runtime's existing Finite Home. The optional
`--existing-identity` flag on these reads refuses to mint a missing identity;
ordinary CLI first-use behavior is unchanged. Product services still decide the
signer's Brain/Sites access. Dashboard ownership alone grants neither product
membership nor website viewer access.

## Version 1 data and limits

Brain projects only ID, name, kind, role and folder IDs/names. Pending invitations
have `role: invited`, no folder lookup, and do not count as accessible Brains.
Metadata failure returns `folders: null`, displayed as “Folder details
unavailable.” An empty folder array means zero metadata folders. Mounted/linked
folders, invite codes, members, key grants, content, local sync and decryptability
are excluded. At most 100 list rows and 32 metadata requests are allowed; a larger
inventory fails explicitly instead of silently truncating.

The identities route is for Brain admins (Brain enforces this). It projects each
key's npub, role and Folder IDs/states and, only when Core resolved the key, its
kind, name, email and responsible account email. Unresolved keys carry no contact
fields. See the [access report contract](../../finite-brain/docs/brain-access-report-v1.md).

Sites projects Project Repositories that actually have a Site. Source-only
repositories are counted separately. Name, URL, site visibility, publication
state, edit role and repository URL come from `fsite project list`. Repository
visibility is not website visibility. There are no modification dates or thumbnail
fields in this read. Unpublished/disabled sites cannot be opened or shared from
the list; editing requires owner/editor role. Share copies a URL and grants
nothing. New/Edit create a reviewable Chat draft using the existing topic intent;
they do not send a message or recover an original conversation.

Reads allow 10 seconds total, two concurrent requests and four concurrent CLI
processes across both products. Each CLI output and the final response are capped
at 512 KiB. Sites caps the complete project list at 500; Brain caps each folder
list at 500. Subprocesses are killed/reaped on deadlines, cancellation and output
limits. Responses use `Cache-Control: no-store` and never include CLI stderr.

## Failure and compatibility

The browser stores inventory only in the mounted account/organization/agent
scope. It fetches on entry and manual Refresh, with no background polling. A
network/timeout/service failure preserves the previous list and timestamp with a
stale notice. Authorization loss, disabled/not-ready access, unsupported endpoints
and incompatible responses clear it. Switching scope unmounts the old state,
aborts the old read and ignores late responses; folder expansion and Site dialogs
cannot carry over.

For these `--existing-identity` JSON reads, nonzero CLI exits emit a bounded,
versioned error classification on stderr without raw diagnostics. Identity failures
and authoritative HTTP 401/403/404 clear prior inventory (native 403); transport
and service failures are retryable (native 503). Unknown/older error contracts fail
closed. A failed per-Brain metadata read clears that row's folders while retaining
the freshly authorized list row. No classification parses human-readable errors.

| Dashboard | Runtime | Result |
| --- | --- | --- |
| New | Existing image without these plugins | Native 404; honest unavailable state, no fake empty list |
| Existing | New image | Existing reads unchanged; new routes unused |
| New | New image with both CLI flags/plugins | Version 1 inventory |
| New | Plugin explicitly disabled/access revoked | Native auth/plugin gate denies; prior inventory cleared |

This changes no Chat persistence, protocol, Device identity, history reader/writer,
SimpleX adapter, enrollment or runtime lifecycle state. Read endpoints write no
product or identity data; only the roster change routes above change Brain
access, as the Agent. New/Edit retain the existing Chat intent writer and
composer behavior. Rollback is the prior dashboard/runtime artifact; there is no
inventory migration or new recovery state. A new immutable runtime candidate,
canary proof and separately authorized deployment are still required before
claiming live production availability. The previously released Skills/SimpleX
runtime does not contain these adapters.

## Verification

Dashboard unit/browser tests cover projection, folder/pending semantics, mobile
layout, keyboard expansion, stale refresh, access loss, unsupported older runtimes,
late agent responses, preview isolation, and draft-only Chat actions.
`infra/images/test_hermes_product_inventory.py` exercises the real native Hermes
router/auth middleware with disposable CLIs and checks resource bounds and cleanup.
CI runs it against packaged modules/plugins. CI builds both CLIs and runs the real signer proof. To run it locally, set `FBRAIN_TEST_BINARY` and `FSITE_TEST_BINARY` to the built binaries; it creates
two disposable agent identities and a distinct human identity and checks signed
requests against a local synthetic product server. With
`FBRAIN_SERVER_TEST_BINARY` set to a built `finite-brain` server as well, it runs
the roster changes against a real local Brain server. `FINITE_INVENTORY_SOURCE_TEST=1`
is only a local source-test convenience and is not packaging evidence.
