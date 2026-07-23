# Destructive Operations

Use this branch before permanently deleting a Folder. A Folder deletion is not
a Page deletion: it deletes the named Folder, every descendant Folder, and all
durable objects in that subtree.

## Confirm The Exact Scope

1. Run `folder list --brain "$BRAIN" --json` and identify the named Folder and
   every descendant Folder.
2. Tell the user that the operation permanently deletes that complete subtree,
   name the affected Folders, and ask once for confirmation of that scope.
3. Execute only on a clear yes. A request to delete one Page, archive content,
   remove a member, or revoke access does not authorize Folder deletion.

The command derives an expected Folder and object inventory immediately before
submission. If that inventory changes, deletion fails closed instead of
silently expanding or shrinking the confirmed scope.

```sh
fbrain --config-dir "$FBRAIN_CONFIG" folder delete "$FOLDER" \
  --brain "$BRAIN" --server "$SERVER" --json
```

On success, the server returns `deletedFolderIds`; the CLI removes those local
Working Tree projections. Do not reconstruct them from generated reports,
derived search indexes, or stale local copies.

## Prove Completion

Run:

```sh
fbrain --config-dir "$FBRAIN_CONFIG" folder list --brain "$BRAIN" --server "$SERVER" --json
fbrain --config-dir "$FBRAIN_CONFIG" status --json
fbrain --config-dir "$FBRAIN_CONFIG" sync now --summary
fbrain --config-dir "$FBRAIN_CONFIG" conflicts --json
```

Completion: every returned `deletedFolderIds` entry is absent from authoritative
Folder metadata and the local Working Tree, sync reaches a named latest
sequence, and conflicts are empty or explicitly reported. Report the deleted
Folder IDs and never describe an unverified local disappearance as successful
server deletion.
