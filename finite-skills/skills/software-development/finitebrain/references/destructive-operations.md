# Destructive Operations

Use this branch before permanently deleting a Folder or an Organization Brain.
A Folder deletion is not a Page deletion: it deletes the named Folder, every
descendant Folder, and all durable objects in that subtree. Brain deletion is
covered under [Delete An Organization Brain](#delete-an-organization-brain).

Run every command in this branch from the Brain's Working Tree, so each one
uses the server that tree was opened against.

## Confirm The Exact Scope

1. Run `fbrain folder list --brain <brain-id> --json` and identify the named
   Folder and every descendant Folder.
2. Tell the user that the operation permanently deletes that complete subtree,
   name the affected Folders, and ask once for confirmation of that scope.
3. Execute only on a clear yes. A request to delete one Page, archive content,
   remove a member, or revoke access does not authorize Folder deletion.

The command derives an expected Folder and object inventory immediately before
submission. If that inventory changes, deletion fails closed instead of
silently expanding or shrinking the confirmed scope.

```sh
fbrain folder delete <folder-id> --brain <brain-id> --json
```

On success, the server returns `deletedFolderIds`; the CLI removes those local
Working Tree projections. Do not reconstruct them from generated reports,
derived search indexes, or stale local copies.

## Prove Completion

```sh
fbrain folder list --brain <brain-id> --json
fbrain status --json
fbrain sync now --summary
fbrain conflicts --json
```

Completion: every returned `deletedFolderIds` entry is absent from authoritative
Folder metadata and the local Working Tree, sync reaches a named latest
sequence, and conflicts are empty or explicitly reported. Report the deleted
Folder IDs and never describe an unverified local disappearance as successful
server deletion.

## Delete An Organization Brain

Brain deletion removes the Brain for everyone: its Folders, Pages and
server-side history, every member's and guest's access, and its pending
invitations and Invite Tokens. Only a current admin of an Organization Brain
can delete it; Personal Brains cannot be deleted. Only an operator restore from
a backup can bring it back, and local Working Trees and exports stay where they
are.

1. Preview: `fbrain brain delete <brain-id> --json`. A preview deletes nothing.
2. If `state` is `blocked`, report each `preview.blockingMounts` entry (Mount
   ID, direction, source and destination Brain) and stop. Revoking a Mount
   changes another Brain's sharing, so revoke one with
   `fbrain mount revoke <mount-id>` only when the user asks, then preview
   again.
3. Show the user the exact scope: `server`, Brain `name` and `brainId`, Folder
   and Page counts, members, guests, pending invitations and open Invite
   Tokens. Ask once whether to delete exactly that Brain. Only a clear yes
   naming that Brain authorizes deletion; a request to delete Pages or
   Folders, leave a Brain, or archive content does not.
4. On that yes, run the preview's `confirmCommand` within ten minutes:

   ```sh
   fbrain brain delete <brain-id> --confirm <stateDigest> --json
   ```

   If anything in the Brain changed since the preview, the command fails and
   nothing is deleted: preview again, show the new scope, and ask again.

Completion: the command returned `state: deleted` with a `receipt` naming the
Brain, and `fbrain brain list --json` no longer lists it. Report the receipt's
`deletionEventId` and `deletedAt`.

A Working Tree of a deleted Brain stops syncing for good: `sync now` and the
daemon report the deletion and list the unsynced local changes they kept in
place. Ask the user before moving those files into another Brain.
