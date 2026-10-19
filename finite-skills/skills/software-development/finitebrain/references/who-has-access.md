# Who Has Access

Use this branch to answer who has access to a Brain, who a key belongs to,
or what the dashboard Brain roster shows, and before removing or demoting
anyone.

## Read The Report

Answer from one fresh report for one exact Brain ID:

```sh
fbrain access list --brain <brain-id> --json
```

The acting key must itself be that Brain's admin, owner or Personal Agent;
there is no account fallback. The dashboard roster is this same report, run
by the Agent, so answer roster questions from it too.

Call the result the named report only when its JSON has
`"version": "finite-brain-access-report-v1"`. An `fbrain` older than 0.7
prints a metadata summary with no `version`; never call it complete or quote
`currentAccessComplete` from it. Say a newer `fbrain` is needed for the named
report and that an operator can run it from an updated CLI; do not roll the
Agent fleet for this task. If the server answers `unsupported`, report that.
`fbrain access summary` names no identities and proves no coverage, so it
never stands in for the report.

Report `coverage` and `currentAccessComplete` first. Coverage has `members`,
`guests`, `mounts`, `currentGrants` and `accessHistory` scopes, each
`complete`, `unverified` or `unsupported`. While any scope is `unverified`,
the list is incomplete: Folders mounted in from another Brain, for example,
have other routes in that only that Brain's admins can report.

## Name Each Key

Each row in `identities` is one exact key. Distinct keys never merge, even
with the same name or email. A row's `description` says who the key belongs
to when that is permitted; it never grants or proves access.

| `description.state` | Say |
| --- | --- |
| `resolved`, `kind: human` | the shared `accountEmail` (or `displayName`): the account's current contact, not proof of mailbox control |
| `resolved`, `kind: agent` | its `displayName`, `lifecycle` and `responsibleAccount.email`: responsibility, not legal ownership |
| `notShared`, `reason: noParticipation` | this key has not acted in this Brain, so it was never looked up |
| `notShared` | the details are not shared with this Brain |
| `ambiguous` | conflicting identity records |
| `unknown` | the identity is not described |
| `unavailable` | the identity source was unavailable |

Give the exact npub with every key that is not `resolved`: it is that key's
only identity. `notShared` deliberately reveals nothing about whether an
account exists, so never guess who a key is from a name, an email, timing or
who invited it. `storedNip05` is a stored public name with its stored time,
not rechecked and not a mailbox.

Core describes only keys that have themselves acted in this Brain: joined,
approved, or wrote a Brain record. Folder Key wraps delivered during sync do
not count. An acting Agent key is described from Core's record of that Agent
and its current owner. A human key is described only once Core links it to a
Finite account, which happens when the person approves a Brain request in
Finite Chat. If this Brain's Core is older and answers only v1 descriptions,
the account must also have shared contact with this Brain through that
approval. Nothing backfills older keys, so many stay `unknown` or `notShared`
(shown as "Unknown" on the roster); a new Personal Brain can too.

Never change access, join, write or approve only to make a name appear. Join
a Brain only when your owner asks you to.

## Folder States

| Report `folders[].state` | Roster shows | Meaning |
| --- | --- | --- |
| `ready` | the Folder name | entitled and holds the current Folder Key |
| `grantMissing` | key pending | entitled, but the current key is not yet wrapped for it; see Membership And Readable Folders in [sharing.md](sharing.md) |
| `revocationIncomplete` | removal pending | holds a current Folder Key without entitlement |

Each row also lists `missingCurrentGrants` and `revocationIncomplete` Folder
IDs. A present grant does not prove the key decrypted anything, and removal
cannot recall plaintext it already read.

## Dashboard Roster

Brain admins see a read-only roster on the dashboard Brain page. It shows
each key's name or email only when the report resolved it, otherwise
"Unknown" with the npub. Its roles map from `brainRole`: `personalAgent` is
"Personal agent", `mountParticipant` is "Shared folder", and
`retainedMountAccess` and `noCurrentRole` are "Removed".

"Identities unavailable" means the Agent could not produce the report: it is
not an admin of that Brain, its Runtime predates the roster, the Brain server
lacks the report, or the roster is too large. Run `access list` yourself to
see which.

## Removing Or Demoting Someone

The report is read-only. Change access only on the user's explicit request,
after confirming the exact npub with them. Never remove, restore, re-grant or
act on another Brain because of a description or its absence.

From the Brain's Working Tree, `fbrain admin member remove --target <npub>`
removes Membership and rotates every Folder whose current key the key holds:
the Folders it reads and any Folder it still shows as `revocationIncomplete`,
for example from an earlier demotion. The server names those Folders and
refuses a removal that skips one, and `fbrain` prepares the new grants and
re-encrypted Folder objects itself. Report each `rotatedFolders` entry (its
`reason` is `readable` or `staleCurrentGrant`). If preparation fails, stop on
the reported blocker and never create or edit a raw
rotation body. `fbrain admin role revoke admin --target <npub>` removes only
the Admin role; in this release it rotates no keys, so Folders the key no
longer qualifies for show `revocationIncomplete` afterwards.

## Clearing Removal Pending

A `revocationIncomplete` Folder needs a Folder Key rotation. Run it from a key
that can open that Folder's current key, one Folder at a time:

```sh
fbrain admin folder-access revoke --folder <folder-id> --target <npub> --json
fbrain admin folder-access rotate --folder <folder-id> --json
```

Both generate a fresh Folder Key at the next version, wrap it for exactly the
Folder's entitled keys and re-encrypt its live pages, so every key outside that
set is cut off, not only a named one. `revoke` also removes the named key's
explicit Folder access, and accepts a key that holds the current key without
entitlement. It refuses a key that stays entitled without that access, such as
an admin with an old explicit grant (`through its Brain role`: change the role
instead), and a key with nothing to remove (`does not currently have access`).
Use `rotate` to clear every `revocationIncomplete` key at once; only its
receipt lists the keys it cut off (`cutOffNpubs`). After a `revoke`, run
`access list` to see which keys lost the Folder. Report `state: complete` and
the new key version from either receipt.

A rotation revokes pending email invitations that would have delivered the
Folder. Pending Mount Offers and Folder share links for it keep a grant for the
old key version and can no longer be accepted: revoke and recreate them.

Mount participants of a Folder that is the source of an active Mount stay
entitled, so both commands wrap the new key for them. `revoke` refuses a key
that still receives the Folder through an active Mount; remove it from that
Mount first. An HTTP 409 saying the Folder Key version changed while the
rotation was prepared means another admin changed the Folder first: rerun the
command. An older Brain server answers `rotate` with `unsupported: ... does not
support Folder Key rotation yet` (nothing changed) and refuses `revoke` for a
key without explicit access.

One rotation request carries every re-encrypted page of the Folder and is
capped at 64 MiB. If the CLI refuses with `needs a <size>-byte rotation
request, over the ... limit` (nothing was sent), the Folder's pages are too
large to re-encrypt at once: report the Folder and size and stop.

Member removal rotates, in one request, every Folder the Member can read or
still holds the current key for. A `rotate` does not shrink that request for
a Folder the Member reads: the Member stays entitled and the removal still
re-encrypts the Folder. It does shrink it for a Folder the Member only holds
as `revocationIncomplete`: after `admin folder-access rotate --folder
<folder-id>` that Folder drops out of the removal. Only where the Member reads
a large Folder through an explicit Folder grant can you first run
`admin folder-access revoke --folder <folder-id> --target <npub>` for that
Folder (it ends that access), then retry the removal. When the Member reads
the large Folders through its role, such as `all_members` Folders, nothing
splits the request: report the removal as blocked by the 64 MiB limit.

Rotation requests take one of a few server slots, in arrival order, after
the server checks the signed request comes from an admin of that Brain or
Mount. A slot is held while the request body arrives (at most 60 seconds) and
while the server applies it; the CLI waits up to four minutes for its turn,
its upload and the result. A 408 means your own upload did not arrive within
60 seconds: nothing changed and the slot was released, so rerun it. A 503
means the server stayed busy and nothing changed. It comes either after the
whole two-minute admission wait (slots stayed taken), or at once when the
admission queue was already full (no waiting place at all). Both count
towards the report below.

Arrival order serves a waiting admin ahead of anyone who arrives later, and
while the slots are merely busy it is admitted within about one 60-second
upload. It does not stop a flood: anyone who can create a Brain on this
server (any key can) and keep many requests open can keep the queue full and
delay rotations for as long as they sustain that load. Report repeated 503s
of either kind to the server operator instead of retrying in a loop.

Member removal clears removal pending for the removed key itself. Run it as
an admin that can open the current key of every Folder it rotates; if
preparation stops on a Folder this key cannot open, run the removal from
another admin that holds that key. If it fails with HTTP 413 `exceeds ...
limit`, rotate the removed key's `revocationIncomplete` Folders first with
`folder-access rotate`, then remove the Member.

Every refusal below changes nothing. An older Brain server answers with
`unsupported: ... cannot say which Folders member removal must rotate`:
report that the Brain server needs an upgrade before this `fbrain` can remove
Members. HTTP 400 naming Folders where the Member still holds the current key
means the removal skipped them: upgrade `fbrain` if it is an older version,
otherwise rerun the removal so it fetches a fresh plan. HTTP 400 saying
member removal requires exactly one rotation per Folder, or a Folder that
changed while the removal was prepared, also means another admin changed
access in between: rerun the removal.

Run `access list` again afterwards. Done when the key holds the intended role
and no Folder shows `revocationIncomplete` for it. Report any that remain as
removal pending: the key still holds those Folders' current keys. Earlier keys
and downloaded copies cannot be recalled.
