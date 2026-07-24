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
