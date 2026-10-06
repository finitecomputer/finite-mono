# Sharing And Invitations

Use this branch to invite someone, find or accept an invitation, share an
Organization Brain with another Agent, or unlock a member's Folders.

## Choose The Invitation

Creating and revoking invitations require Brain admin standing. Choose the
path by what identifies the invitee, and confirm the person by the email or
NIP-05 name the user knows.

**Key-addressed invitation**, for an npub, a 64-character hex public key, or
a name that resolves through public NIP-05 (for example `name@finite.vip`):

```sh
fbrain invite brain create --brain <brain-id> --target <npub|hex|NIP-05> [--folder <folder-id>] [--expires-in 7d]
```

`fbrain` resolves a NIP-05 name once and binds the invitation to that one
key; only that key can accept it. The receipt reports `deliveryStatus: in_app`:
the invitee finds it with `fbrain invite brain list` outside any Brain Working
Tree, with the intended server selected, or in the Product Client. No email is
sent, by design.

**Email capability invitation**, for an email address with no public NIP-05:

```sh
fbrain invite-token create --brain <brain-id> --email <address> [--role member|admin] [--expires-in 7d]
```

The printed link is a single-use bearer capability. The first key that
redeems it with `fbrain invite-accept <url>` joins the Brain with the token's
role; the email address is only where the link was sent. The raw token is
shown once: share the link only with the intended person, and keep it out of
logs, Brain Pages and reports. The receipt's `deliveryStatus` is one of:

- `sent`: the mailer accepted the email.
- `not_configured`: the server has no mailer; share the link yourself.
- `failed`: the email failed; the link still works, so share it yourself or
  revoke it.
- `manual`: no `--email` was given; share the link yourself.

Revoke an unredeemed link with
`fbrain invite-token revoke --brain <brain-id> --token-id <token-id>`.

A failed NIP-05 lookup does not authorize switching to a bearer link: explain
the email capability alternative and get the user's choice first, and respect
any user restriction on bearer invitations.

`--expires-in` takes whole hours or days from `1h` through `30d`; the default
is `7d`. An older Brain server can reject `1h` when the request arrives a few
seconds late, and a leading client clock can push `30d` past the server's
strict ceiling. Report the error, keep the requested expiry, and let the user
choose a longer duration or wait for a server update.

Unsupported, so never offer them:

- Inviting by Finite account, login email, or Core account lookup. An email
  that is not a public NIP-05 name fails as a `--target` for every `invite`,
  `admin`, `collaborator` and `mount` command.
- Inviting every agent or device that belongs to one person. Each invitation
  reaches exactly one key; invite each key separately.
- Guest email bootstrap. The Folder claim flow and invite secrets are retired;
  Folder Invitations target one key.

## Membership And Readable Folders

Membership and readable Folders are separate states. A Brain Invitation grants
Membership and entitlement to every `all_members` Folder plus each selected
`restricted` Folder. Naming an `owner` or `admin_only` Folder with `--folder`
does not grant its required standing. A member-role token grants Membership
and `all_members` entitlement; an Organization Brain admin-role token grants
admin standing and entitlement to every Folder. Folder Invitations grant
bounded Guest access without Brain Membership.

An entitled Folder stays locked until an admin holding the current Folder Key
wraps it for the member. That admin's ordinary sync attempts pending wraps on
a best-effort basis. If necessary, that admin runs
`fbrain admin ensure-access --brain <brain-id> --target <npub|hex|NIP-05>`.
It also completes missing Brain Membership, so use it only for an intended
member, never to repair a Folder Guest's access. A Folder whose `repair` is
`needsKeyHolder` needs an admin who holds that Folder's current key.

The invitee checks with `fbrain sync now --summary`, then
`fbrain access explain <folder-id> --json`. Its local `state` is one of:

- `readable`: usable once sync succeeded and the expected revisions arrived.
- `locked`: Folder Access or an open Folder Key is missing. Verify entitlement
  before diagnosing a delayed key.
- `unavailable`: present but unreadable.
- `unknown`: absent from the local state.

## Find And Accept Invitations

To answer "have I been invited to anything?", run `fbrain invite brain list`
with no `--brain`, from outside any Brain Working Tree, with the intended
server selected. Inside a Working Tree the same command lists the invitations
that Brain issued, like `--brain <id>`, and needs admin standing.
`brain list --json` rows with `role: "invited"` are the same incoming
invitations. Also point the user to pending approval and invitation cards in
chat.

Accept by invitation id (`invitation-...`), not invite code (`invite-...`).
An invite code's public instructions at
`https://<brain-server>/v1/brain-invitation-links/<invite-code>/llms.txt`
print the id. An `expired` invitation cannot be accepted; ask the admin to
invite again.

## Share An Organization Brain With Another Agent

For a normal request to share an Organization Brain with another managed
Agent, use the recipient's canonical Managed Agent Email and one convergent
operation from the Brain's Working Tree:

```sh
fbrain collaborator ensure-admin \
  --target agent@example.finite.vip \
  --json
```

`fbrain` resolves the email itself, once, through public NIP-05, so pass the
email straight to the command without resolving or probing it first. It
prepares every Folder grant whose current
key this Finite Home can open and returns a typed receipt. Inspect `state`
before reporting the result:

- `complete`: authoritative postcondition inspection proved the Admin Brain
  Role and a current Folder Key Grant for every Folder in this operation's
  snapshot. Report the ready Folder count. Folders created or rotated later
  are not covered.
- `partial`: useful role and grant progress was preserved, but collaboration
  is not complete. Name each safe Folder path and reason from `folders`. When
  the receipt supplies a holder email, tell the user to
  retry the exact same command from that current key holder's Finite Home.
  Otherwise ask another current Folder reader who can open the listed Folder
  to retry; never invent or expose a holder identity. Admin role alone is not
  successful sharing.
- `indeterminate`: the mutation may have committed, but the client could not
  prove its postcondition. Do not claim success or clean failure. Retry the
  exact same idempotent command, then inspect the new typed receipt.

Reports may include the canonical Agent Email, Folder paths, readiness counts,
safe reason codes, and named holder emails supplied by the receipt. Never paste
raw response payloads, Member Identity keys, wrapped grant events, auth
material, Folder Keys, or grant plaintext.

Low-level `admin` commands are advanced primitives. `admin member add` and
`admin role grant admin` change Brain Role, while `admin folder-access grant`
grants one specific Folder version. Separately or together they
do not prove complete Organization Brain Collaboration, so the normal "share
this Org Brain with Agent B" request uses `collaborator ensure-admin`.
