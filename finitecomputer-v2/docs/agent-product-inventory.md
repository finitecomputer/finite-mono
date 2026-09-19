# Selected-agent Brain and Sites inventory

The dashboard's Brain and Sites pages read the selected Agent Runtime's product
access, not the human account's memberships. Skyler's table/list designs remain
the presentation baseline. These are metadata reads, without inference.

## Authority and ownership

Every entry/manual Refresh obtains a fresh account-authorized native Hermes grant
through the existing `/api/agents/{runtime}/hermes-access` flow. The browser calls
one fixed native GET route:

| Product owner | Native route | Local supported read |
| --- | --- | --- |
| FiniteBrain | `/api/plugins/finite-brain/overview` | `fbrain brain list --json --existing-identity`, then `fbrain brain metadata --brain=ID --json --existing-identity` |
| Finite Sites | `/api/plugins/finite-sites/overview` | `fsite project list --output json --existing-identity` |

Native Hermes authenticates before plugin routing and respects disabled plugins.
The product plugins live under each product's `integrations/hermes/` directory.
The shared bounded subprocess reader is sealed into Hermes' Python environment;
it contains no product inventory or authorization policy. Both full and minimal
Hermes packages include the product plugins. No mutable plugin installation,
Core product API/storage, Management Pipe feature reports, or shell endpoint is
introduced. Requests accept no profile, identity, server, command or query input.

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
product or identity data. New/Edit retain the existing Chat intent writer and
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
requests against a local synthetic product server. `FINITE_INVENTORY_SOURCE_TEST=1`
is only a local source-test convenience and is not packaging evidence.
