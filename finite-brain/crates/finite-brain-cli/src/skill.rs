//! `fbrain --skill`: a self-contained agent guide to the fbrain CLI, printed
//! to stdout so agents can ingest capabilities directly.

use std::io::Write;

use crate::CliError;

pub(crate) const SKILL_GUIDE: &str = r#"# fbrain skill guide

`fbrain` is the command-line control plane for FiniteBrain. A Finite Brain is
an end-to-end encrypted knowledge base: a set of Folders whose Markdown pages
sync through the FiniteBrain server as encrypted objects that only Folder Key
holders can read. You work in a Brain Working Tree, a local decrypted mirror
of every Folder your identity can read, and `fbrain` encrypts and syncs your
changes. Folder Keys are opened into memory per operation; there is no durable
unlock state.

## Install and auth

`fbrain` ships as a single binary (from the finite-mono repo:
`cargo run -p finite-brain-cli --bin fbrain -- <args>`). Server and identity
defaults are built in; environment variables are overrides, never
requirements:

- `FINITE_BRAIN_SERVER_URL` defaults to `https://brain.finite.computer`.
- `FINITE_IDENTITY_AUTHORITY` defaults to `https://identity.finite.vip`.

Auth uses the shared Finite identity in the current Finite Home
(`$FINITE_HOME/identity/`, else `~/.finite/identity/`). It is minted on first
signing use; adopt an existing secret only when asked:

```sh
fbrain auth status --json        # who am I acting as
fbrain auth import --file <path> # adopt an existing secret (explicit request only)
fbrain auth login <email>        # request an email challenge
fbrain auth redeem <email> <token> # bind a finite.vip email to this key
fbrain doctor                    # end-to-end health check
```

## Everyday flow

```sh
fbrain brain list --json         # Brains visible to this identity
fbrain open personal             # or: fbrain open <brain-id> [path]
cd <printed working tree path>
fbrain sync now --summary        # pull remote changes, push local edits
fbrain status --json             # tree, server, daemon, sync state
```

Inside the tree, each top-level directory is one Folder and each Markdown
file inside it is one synced page. Write a note by creating or editing a
`.md` file under a Folder (`raw/` for immutable captured sources and Asset
Source Notes, `wiki/` for durable synthesized pages), then sync again:

```sh
fbrain folder list               # Folders, access modes, key versions
$EDITOR wiki/my-note.md          # ordinary file tools are the editor
fbrain wiki check                # internal [[links]] resolve?
fbrain sync now --summary        # publish
fbrain conflicts                 # must stay empty; resolve <id> if not
fbrain search "<query>" --json   # ranked search across readable Folders
```

Read the tree's root `AGENTS.md` first; it carries the Brain id, your acting
identity and role, and the same orientation in short form. Never edit
`.finitebrain/`, generated `_index.md` / `_wiki/` files, or locked
metadata-only Folders.

## Rename a Brain

```sh
fbrain brain rename "New display name" --brain <brain-id> --json
# Inside the intended open Working Tree, --brain may be omitted.
fbrain sync now --summary
```

Organization admins, Personal Brain owners, and active Personal Agents may
rename a Brain. Resolve an ambiguous target with `brain list --json` first.
Rename preserves the stable Brain ID, Working Tree path, content, membership,
and key grants. The signed rename appears in administrative sync history;
clients learn the new display name on refresh or sync. The server must support
rename before this CLI command is used.

## Sharing: inviting someone (admin)

Creating and revoking invitations require Brain admin standing. Invitees
inspect and accept key-addressed invitations as the addressed key; a bearer
holder can redeem a token. Grants name keys or capability tokens. Choose the
path by what identifies the invitee.

Key-addressed invitation: for an npub, a 64-character hex public key, or a
name that resolves through public NIP-05 (for example `name@finite.vip`):

```sh
fbrain invite brain create --brain <brain-id> --target <npub|hex|NIP-05> [--folder <folder-id>] [--expires-in 7d]
```

`fbrain` resolves a NIP-05 name once and binds the invitation to that one
key; only that key can accept it. The receipt reports
`deliveryStatus: in_app`: the invitee finds it with `fbrain invite brain list`
outside any Brain Working Tree, with the intended server selected, or in the
Product Client. No email is sent, by design.

Email capability invitation: for an email address with no public NIP-05:

```sh
fbrain invite-token create --brain <brain-id> --email <address> [--role member|admin] [--expires-in 7d]
fbrain invite-token list --brain <brain-id>
fbrain invite-token revoke --brain <brain-id> --token-id <token-id>
```

The printed link is a single-use bearer capability. The first key that
redeems it with `fbrain invite-accept <url-or-token>` joins the Brain with the
token's role; the email address is only where the link was sent. The raw
`fbit-...` token is shown once: share the link only with the intended person
and never paste it into logs, Brain pages, or reports. `deliveryStatus` is
`sent` (the mailer accepted it), `not_configured` (no mailer; share the link
yourself), `failed` (the link still works; share it yourself or revoke it),
or `manual` (no `--email`; share the link yourself).

The CLI accepts `--expires-in` in whole hours or days from `1h` through `30d`;
the default is `7d`. An older Brain server can reject `1h` when the request
arrives a few seconds late. Report the error and preserve the requested
expiry; let the user choose a longer duration or wait for a server update. A
leading client clock can also make `30d` exceed the server's strict ceiling.

Membership and readable Folders are separate states. A Brain Invitation grants
Membership and entitlement to every `all_members` Folder plus each selected
`restricted` Folder. Naming an `owner` or `admin_only` Folder with `--folder`
does not grant its required standing. A member-role token grants Membership
and `all_members` entitlement; an Organization Brain admin-role token grants
admin standing and entitlement to every Folder. Folder Invitations instead
grant bounded Guest access, without Brain Membership.

For an intended Brain member, an entitled Folder stays locked until an admin
holding the current Folder Key wraps it for them. That admin's ordinary sync
attempts pending wraps on a best-effort basis. If necessary, that admin can run
`fbrain admin ensure-access --brain <brain-id> --target <npub|hex|NIP-05>`.
This also completes missing Brain Membership: use it only for an intended
member, never to repair a Folder Guest's access. A `needsKeyHolder` result
requires an admin who holds the current key.

The invitee checks with `fbrain sync now --summary`, then
`fbrain access explain <folder-id> --json`. Its local `state` is `readable`,
`locked`, `unavailable`, or `unknown`. `locked` means Folder Access or an open
Folder Key is missing; verify entitlement before diagnosing delayed keys.
`unavailable` means present but unreadable; `unknown` means absent locally.
Confirm successful sync and expected revisions alongside `readable` before
reporting that current content is usable. Lower-level primitives:
`admin member add`,
`admin role grant admin`, `admin folder-access grant --folder <id>`.

Unsupported: inviting by Finite account, login email, or Core account lookup;
one invitation reaching every agent or device of one person; guest email
bootstrap through a Folder claim flow or invite secret. If a public NIP-05
record is confirmed absent, explain the email capability alternative and
obtain the user's choice before changing identity binding. Transport or
document errors only establish that resolution could not verify a key.

## Sharing: being invited (invitee)

```sh
# Run outside any Brain Working Tree, with the intended server selected.
fbrain invite brain list                 # incoming invitations, including expired ones
fbrain invite brain accept --id <invitation-id>
fbrain invite-accept <url-or-token>      # redeem a capability Invite Token link
fbrain open <brain-id>                   # then sync as usual
```

`invite brain list` with no `--brain` is your inbox only outside a Brain
Working Tree. Inside a tree it infers that Brain and lists issued invitations,
just like `--brain <id>`; the scoped listing requires admin standing.
`brain list --json` also
hints at incoming invitations with `role: "invited"`. Accept by invitation
id (`invitation-...`), not by invite code (`invite-...`); when only a code
is at hand, read its `llms.txt` instructions URL for the id.

Every invitation carries a public instructions document at
`https://<brain-server>/v1/brain-invitation-links/<invite-code>/llms.txt`;
open it when an invite code, expiry, or claim step confuses you. An
invitation marked `expired` cannot be accepted; ask the admin to re-invite.

## Folder access

Folders have access modes: `owner`, `admin_only`, `all_members`,
`restricted` (explicit guests). Your readable Folders are materialized in the
tree; Folders you cannot read appear locked or not at all. Access changes
are signed admin events, and Folder Keys rotate on revocation; the CLI
prepares rotation material automatically; never hand-build rotation bodies.

## Who has access?

Answer "who has access?" with one fresh server report for one exact Brain ID:

```sh
fbrain access list --brain <brain-id> --json
```

The acting key must itself be an admin, owner, or Personal Agent of that
Brain; there is no account fallback. Report `coverage` and
`currentAccessComplete` first: never call the list complete while a scope is
`unverified`. Each row is one exact key; distinct keys never merge, even with
the same name or email. A row's `description` says who the key belongs to,
when that is permitted; it is never authority:

- `resolved` human: say the shared account email (or display name). It is the
  account's current contact, not proof of mailbox control.
- `resolved` agent: say its name, lifecycle and responsible account email.
  Responsibility is not legal ownership or proof of decryption.
- `notShared`: say the details are not shared. `noParticipation` means this
  key has not acted in this Brain; never guess who it is.
- `ambiguous`, `unknown`, `unavailable`: say exactly that, with the key.
- `storedNip05` is a stored public name with its stored time, not rechecked
  and not a mailbox.

`revocationIncomplete` lists Folders where a current grant remains without
entitlement; `missingCurrentGrants` lists entitled Folders without one. A
present grant does not prove the key decrypted anything. Never remove,
restore, re-grant, or act on another Brain because of a description or its
absence; confirm the exact npub with the user first. An older server
fails with `unsupported`; report that rather than substituting
`fbrain access summary`, which names no identities and proves no coverage.

## Provenance

Memberships and grants record where they came from: an invitation, a signed
approval artifact, or a direct admin action. When reporting who has access,
use `fbrain access list` rather than inferring from local files.

## Error glossary

- `unsupported: ... retired Brain protocol` / 404 on a route: upgrade fbrain.
- `email auth ...` or identity resolution failures: check connectivity to the
  identity authority; override with `FINITE_IDENTITY_AUTHORITY` only for
  development.
- `does not resolve to an npub through public NIP-05` on invite: the email
  is not a usable key target; use `fbrain invite-token create --email`.
- `approval nonce was already applied`: the signed approval was already
  executed; do not retry the same artifact.
- `deliveryStatus: in_app` on an invitation: in-band delivery by design.
  The invitee sees it in `fbrain invite brain list` outside any Brain Working
  Tree, with the intended server selected, or the Product Client.
  It does not mean email delivery is broken.
- `expired` on an invitation: it can no longer be accepted; re-invite.
- Invite-code vs invitation-id confusion: codes start with `invite-`; open
  the code's `llms.txt` instructions and use the invitation id they print.
- `no usable current grant was available`: this Finite Home cannot open that
  Folder Key; run the same command from a current key holder (see
  `admin ensure-access` output for holder hints).
- `blocked: ...` sync state: run `fbrain status --json` and
  `fbrain sync status --json`; never point the tree at a different server to
  unblock it.

When an invitation's public instructions are involved, the authoritative
reference is its `llms.txt` document:
`https://<brain-server>/v1/brain-invitation-links/<invite-code>/llms.txt`.

## Guidance precedence

This guide describes the installed CLI. Verify technical claims against its
help and errors and current server evidence. Preserve user instructions,
authorization limits, and intentional customizations, including restrictions
on bearer invitations. Report unresolved conflicts before acting on the
conflicting instruction; leave user-owned skills unchanged unless the user
asks. Saved defect notes are dated observations: recheck the installed CLI
and current server behavior before repeating them as advice.

## Security rules

- Never print or expose Nostr secrets, Folder Keys, grant plaintext, wrapped
  grant events, or auth files.
- Use `--json` for machine inspection; summarize sensitive output instead of
  pasting raw payloads.
- Treat the configured server as authoritative; do not substitute another
  Brain server for a tree that was opened against a different one.
"#;

pub(crate) fn print_skill<W: Write>(output: &mut W) -> Result<(), CliError> {
    writeln!(output, "{SKILL_GUIDE}")?;
    Ok(())
}
