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
`.md` file under a Folder — `raw/` for immutable captured sources and Asset
Source Notes, `wiki/` for durable synthesized pages — then sync again:

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

## Sharing: inviting someone (admin)

The smooth path is the invitation plan flow (ADR-0046), which resolves an
email into the human plus their grant-ready account agents:

```sh
fbrain invite brain create --brain <brain-id> --target <email|npub>
```

For account-backed emails the plan flow is: preflight resolves the immutable
participant set, commit writes one npub-bound invitation per participant.
Re-inviting after an expiry supersedes the old invitation automatically.
Invitees accept via the public instructions URL or `fbrain invite brain
accept`; acceptance grants Brain Membership.

Delivery is explicit in every receipt's `deliveryStatus`: `sent` means a
courtesy email carrying the public instructions URL reached the human
account mailbox; `in_app` means the npub-bound invitee (managed agents, or
humans when no mailer is configured) gets the invitation in their
authenticated client instead of email; `not_configured` means the server has
no invite mailer; `failed` means the courtesy email errored after the
invitations were already committed — they remain valid and visible in-app,
so report it, do not treat the invite as lost.

Repair a half-onboarded member (accepted but missing Folder Keys, or
membership lost) with one idempotent command:

```sh
fbrain admin ensure-access --brain <brain-id> --target <email|nip05|npub>
```

It completes membership server-side and wraps every entitled Folder Key this
Finite Home can open; re-run it from a current key holder when a Folder
reports `needsKeyHolder`. Lower-level primitives: `admin member add`,
`admin role grant admin`, `admin folder-access grant --folder <id>`.

Capability Invite Tokens are the kernel-simple alternative: one single-use,
unguessable, revocable link that redeems to membership for whatever npub
presents it (email is delivery only, never identity):

```sh
fbrain invite-token create --brain <brain-id> [--role member|admin] [--email <addr>]
fbrain invite-token list --brain <brain-id>
fbrain invite-token revoke --brain <brain-id> --token-id <token-id>
```

The raw `fbit-...` token is shown once; share the printed URL. The invitee's
agent redeems it with `fbrain invite-accept <url-or-token>`.

## Sharing: being invited (invitee)

```sh
fbrain invite brain list                 # your pending invitations (expired ones are marked)
fbrain invite brain accept --id <invitation-id>
fbrain invite-accept <url-or-token>      # redeem a capability Invite Token link
fbrain open <brain-id>                   # then sync as usual
```

`invite brain list` with no `--brain` is your inbox: invitations addressed
to this identity. The same command with `--brain <id>` lists invitations
ISSUED on that Brain — a different dataset, so do not answer "have I been
invited to anything?" with the `--brain` form. `brain list --json` also
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
are signed admin events, and Folder Keys rotate on revocation — the CLI
prepares rotation material automatically; never hand-build rotation bodies.

## Provenance

Memberships and grants record where they came from: an invitation, an
invitation plan commit, a signed approval artifact, or a direct admin action,
with the roster revision when account agents were involved. When reporting
who has access, read `fbrain brain metadata --json` and `fbrain access list`
rather than inferring from local files.

## Error glossary

- `unsupported: ... retired Brain protocol` / 404 on a route: upgrade fbrain.
- `email auth ...` or identity resolution failures: check connectivity to the
  identity authority; override with `FINITE_IDENTITY_AUTHORITY` only for
  development.
- `invitation plan has expired; run preflight again`: plans live 15 minutes;
  re-run the invite command.
- `plan hash does not match the resolved set` / roster drift conflict: the
  account roster changed; commit the fresh preflight the server returned.
- `approval nonce was already applied`: the signed approval was already
  executed; do not retry the same artifact.
- `already a brain member` on commit: that participant was skipped, not
  failed; remaining invitations still committed.
- `deliveryStatus: in_app` on an invitation: in-band delivery by design —
  the invitee sees it in `fbrain invite brain list` or the Product Client.
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
