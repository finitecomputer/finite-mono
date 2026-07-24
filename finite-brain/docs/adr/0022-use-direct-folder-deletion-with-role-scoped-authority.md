# Use Direct Folder Deletion With Role-Scoped Authority

Status: accepted 2026-07-16.

FiniteBrain Folder deletion is a direct hard-delete operation; the product has
no intermediate Trash or restore lifecycle. In a Personal Brain, the owner may
hard-delete any Folder and a **Personal Agent** may hard-delete any Folder under
its full operational Brain access. In an Organization Brain, any Brain admin
may hard-delete a Folder, including an admin Member Identity controlled by an
agent; ordinary members may not. Direct deletion must not claim erasure from
backups, snapshots, retained client plaintext, or storage history.

Deleting a Folder atomically deletes its complete subtree, including all Pages,
Assets, nested Folders, and folder-local metadata; a failed operation leaves the
live subtree intact. Pages, Assets, and other Folder content also remain
individually deletable without deleting their containing Folder.

In an Organization Brain, direct deletion of both individual content and whole
Folders is admin-only. A non-admin member with write access may create and edit
content, but cannot permanently delete a Page, Asset, other content item, or
Folder.

In a Personal Brain, direct deletion of both individual content and whole
Folders is limited to the owner and Personal Agent. Other collaborators with
write access may create and edit content inside their granted Folder scope but
cannot permanently delete content or Folders.

Direct deletion removes the target permanently from Brain's live product state
and offers no Trash, undo, or restore workflow. Brain retains only the minimal
signed deletion marker and audit metadata required to propagate the deletion to
offline clients, prevent stale state from resurrecting it, and identify the
authorizing principal.

Folder deletion atomically closes every Folder-specific relationship in the
deleted subtree, including invitations, share links, Folder Access, Folder Key
Grants, delegated scope entries, mounts, and working-tree sync. If any required
cleanup fails, the live subtree and all of its relationships remain intact.

Adding a Personal Agent is the owner's standing full-trust authorization. A
Personal Agent's valid signed direct-deletion request therefore requires no
additional human approval ticket; removing that Personal Agent ends its future
authority. Brain still binds deletion to the exact Folder and expected current
state so a stale or ambiguous request fails without mutation.

The human Product Client presents one confirmation before direct Folder
deletion. It names the Folder, reports the number of nested Folders and content
items in the subtree, states that deletion is permanent, and offers one
**Delete permanently** action; it does not require typing the Folder name or a
second confirmation.

A valid signed deletion wins over later stale or offline edits. Sync removes
the deleted subtree from active Brain projections, rejects new revisions under
deleted Folder and object identities, and requires intentionally recreated
content to use new identities rather than resurrecting the deleted state.

## Bounded accepted-state amendment

Status: accepted 2026-07-22.

Direct deletion is also an admission promise: every state Brain accepts must
remain removable by one bounded, atomic signed operation. The shared public
capacity contract is `finite.brain.capacity.v1`, defined by
`BRAIN_CAPACITY_ENVELOPE` in `finite-brain-core`. Its current limits are:

| Dimension | Maximum |
| --- | ---: |
| live Folders per Brain | 1,000 |
| Folder nesting depth | 32 |
| live objects per Brain | 10,000 |
| retained sync records per Brain | 100,000 |
| Members per Brain | 1,000 |
| Folder Access entries per Brain | 10,000 |
| Folder Key Grants per Brain | 10,000 |
| open Folder Invitations per Brain | 1,000 |
| active Folder share links per Brain | 1,000 |
| Folder mounts per Brain | 1,000 |
| shared-Brain connections per Brain | 1,000 |
| delegated Folder scopes per Brain | 10,000 |

These are Greenfield accepted-state limits, not a claim that an oversized
production state can be silently migrated. Every database write path is inside
the same SQLite serialization boundary as the applicable capacity trigger, so
interactive writes, imports, replay, sync, and concurrent callers cannot
jointly cross a limit after independent preflight. One-over mutations return a
typed capacity failure and do not commit partial state.

Subtree discovery uses one depth- and cardinality-bounded adjacency traversal.
Deletion then performs an explicitly counted, envelope-bounded set of
statements inside the same transaction. Its content-free work report records descendants, objects,
audience, invitations scanned and deleted, mutation statements, maximum SQL
parameters, and retries. Those counters are the deterministic acceptance
boundary; wall-clock observations do not define correctness. A replay of the
same signed deletion is idempotent and performs no cleanup work.

The signed deletion marker records the exact deleted Folder identities and the
pre-deletion materialized/sync audience. This is the minimum retained evidence
needed to make affected offline Working Trees converge after their live access
rows have been removed. It is not visible to unrelated principals and does not
contain Folder names, content, keys, grants, or signed request bodies.

The Product Client's protected HTTP request also binds the exact Folder
identities and live-object count shown in its destructive confirmation. The
store compares that expected scope after acquiring the serialized mutation
boundary; a changed subtree returns a conflict and leaves live state intact.

Capacity limits govern current accepted live state. Retained deletion and audit
evidence is separately bounded by the sync-record limit because it is required
for convergence and anti-resurrection. Raising any number above requires a new
maximum-boundary proof for admission, traversal, atomic cleanup, retry, and
unrelated-request progress; a product requirement alone is not sufficient.
