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

A key's details appear only after its responsible account shares contact with
this Brain. That happens when the person next accepts an invitation or
approves a Brain request in Finite, which tells them this Brain's admins will
see their account email and their responsibility for their agents. Nothing an
admin or Agent runs changes a description, so leave access unchanged when the
goal is only to make a name appear.

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
removes Membership and rotates the affected Folder Keys, preparing the new
grants and re-encrypted Folder objects itself. If preparation fails, stop on
the reported blocker and never create or edit a raw rotation body.
`fbrain admin role revoke admin --target <npub>` removes only the Admin role;
in this release it rotates no keys, so Folders the key no longer qualifies
for can show `revocationIncomplete` afterwards.

Run `access list` again afterwards. Done when the key holds the intended role
and no Folder shows `revocationIncomplete` for it. Report any that remain as
removal pending: the key still holds those Folders' current keys. Earlier keys
and downloaded copies cannot be recalled.
