# fbrain CLI Reference

This reference tracks the Rust `finite-brain-cli` surface. In repo development,
run `cargo run -p finite-brain-cli --bin fbrain -- <args>` from the repo root or
build once and run `target/debug/fbrain`.

Global flags:

- `--config-dir <path>`: override fbrain config state for this invocation. The
  signing identity is not stored here (see Identity below).
- `--json`: return machine-readable output where the command supports it.
- `--server <url>`: command-specific server override. Server resolution is
  explicit `--server`, saved Brain Working Tree server, `FINITE_BRAIN_SERVER_URL`,
  legacy `FINITE_BRAIN_PUBLIC_BASE_URL`, then the built-in hosted production
  endpoint.

Transport accepts `https://` endpoints and `http://` only for localhost,
loopback IPs, or the exact host named by the local-harness-only
`FINITE_BRAIN_DEVELOPMENT_HTTP_HOST`. An unreachable configured endpoint is a
blocked state; `fbrain` never substitutes another Brain server.

`FINITE_BRAIN_SERVER_URL` chooses the transport. When
`FINITE_BRAIN_PUBLIC_BASE_URL` is also set, `fbrain` signs that browser-visible
canonical origin into Nostr HTTP authorization events while sending the request
through the transport URL. This lets the current server-side signer adapter
behave like a future client daemon without teaching Brain multiple identities
for the same request.

## Command Map

```sh
fbrain [--config-dir <path>] doctor
fbrain repair
fbrain auth status|import [--file <path>]|login <email>|redeem <email> <token>
fbrain signer status|public-key|sign|encrypt|decrypt
fbrain daemon status|start|stop|logs|tick|watch
fbrain sync status|now [--summary]
fbrain open personal [path]
fbrain open <brain-id> [path]
fbrain status [--json]
fbrain conflicts
fbrain resolve <id>
fbrain search <query> [--folder <folder>...] [--limit <1-50>] [--lexical-only] [--json]
fbrain search-index status [--folder <folder>...]|enable --folder <folder>|disable --folder <folder> [--json]
fbrain activity
fbrain wiki check [--json]
fbrain access explain|list
fbrain brain list|create|bootstrap-personal|metadata|export
fbrain folder create|list|delete
fbrain collaborator ensure-admin
fbrain invite brain create|list|inspect|accept|revoke
approvals list [--brain <brain-id>] [--all]|approve --id <request-id> [--brain <brain-id>]|deny --id <request-id> [--brain <brain-id>]
fbrain invite folder create|list|inspect|accept|claim|revoke
fbrain mount offer create|list|inspect|revoke
fbrain mount accept|list|inspect|revoke
fbrain mount participant add|remove
fbrain admin member add|remove
fbrain admin role grant|revoke admin
fbrain admin folder-access grant|revoke
```

Use `brain bootstrap-personal` for first-time Personal Brain setup. It creates
the empty user-owned Personal Brain and establishes the authenticated agent as
its Personal Agent through Brain's account-bound authority. Direct `brain
create` is for Organization Brains and is not a substitute for this Personal
Agent bootstrap flow.

## Identity

`fbrain` signs with the current Finite Home's Local Identity Key, at
`$FINITE_HOME/identity/identity.json` when `FINITE_HOME` is set and
`~/.finite/identity/identity.json` otherwise. Whichever Finite tool runs first
mints the key in that home; `fbrain` finds it. Hosted users and Agent Principals
receive separate keys and therefore remain separate Member Identities. The first
`fbrain` command that needs to sign mints an identity if none exists; `auth
status` only reports and never creates one.

```sh
fbrain auth status --json
fbrain auth import < secret.txt
fbrain auth import --file <path>
fbrain auth login <email> --json
fbrain auth redeem <email> <token> --json
fbrain signer public-key
fbrain signer sign --kind text --content "hello"
fbrain signer encrypt --to <npub> --text "..."
fbrain signer decrypt --from <npub> --payload "..."
```

`auth import` adopts an existing secret (`nsec1...` or 64-char hex) as the
shared identity. The secret is read from stdin or `--file`, never from an argv
flag, and import refuses to overwrite an existing identity. The legacy
`auth login --nsec`/`auth logout` verbs and the plaintext `auth.json` config
file are removed.

`auth login <email>` asks the trusted identity authority to send a one-time
email challenge. `auth redeem <email> <token>` completes that challenge and
binds the current identity to the verified email. Treat the one-time token as
sensitive input: never repeat it in logs or reports. Only the old secret-bearing
`auth login --nsec` shape is retired.

Use `auth status --json` to confirm the acting npub, identity file, and config
directory. Do not print or request secrets during normal agent work.

## Working Tree And Sync

```sh
fbrain doctor
fbrain brain list --json
open_result="$(fbrain open personal --json)"
brain_tree="$(printf '%s' "$open_result" | python3 -c 'import json,sys; print(json.load(sys.stdin)["nextCommandWorkingDirectory"])')"
cd "$brain_tree"
fbrain status --json
fbrain sync status --json
fbrain sync now --summary
fbrain sync now --json
fbrain conflicts --json
fbrain resolve <conflict-id>
fbrain search "credential rotation" --json
fbrain search-index status --json
fbrain activity
fbrain wiki check --json
```

`open personal` resolves the unique Personal Brain from the signed authoritative
Brain list. Zero or multiple matches stop with guidance instead of guessing.
`open` creates `.finitebrain/` state, saves the server URL when provided, marks
the daemon running, and attempts an initial sync. `sync now` fetches the
encrypted export, opens available grants, pushes local markdown changes,
bootstraps latest state, and materializes readable Folders back into the tree.

When the path is omitted, `open` uses `$FBRAIN_WORKING_TREE_ROOT/<brain-id>` if
configured, otherwise `<current-directory>/<brain-id>`. The hosted runtime sets
`FBRAIN_CONFIG_DIR=/data/agent/fbrain` and
`FBRAIN_WORKING_TREE_ROOT=/data/workspace/finitebrain`.

From inside a Brain Working Tree, commands infer the Brain from Agent State.
From inside a managed Folder directory, Folder-scoped commands infer that
Folder. If context is absent or ambiguous, pass the explicit Brain or Folder
selector named by the error.

Useful `sync now --json` fields include `status`, `latestSequence`,
`recordCount`, `localChanges`, `remoteChanges`, and `conflicts`. Expected status
values include `caught-up`, `applied-remote-records`, `pushed-local-changes`, and
`blocked-local-conflicts`.

Each `remoteChanges` entry produced from a signed sync record includes
`actorNpub`; `--summary` renders it as `actor=<npub>`.

## Search Evidence

`fbrain search` returns ranked Markdown Sections from every currently readable
Folder in one result list. Repeat `--folder` to deliberately narrow the scope;
an unknown or unreadable Folder fails closed. When mounted Folders reuse an ID,
use `<source-brain-id>:<folder-id>` to select one unambiguously. Results identify
the Folder and source Brain, Page path and title, heading ancestry, excerpt,
sync disposition, and the contributing `lexical`, `semantic`, or combined
signals. The default is ten results and the maximum explicit limit is fifty.

BM25 is always available. When the runtime supplies
`FBRAIN_EMBEDDING_ENDPOINT` and `FBRAIN_EMBEDDING_BEARER_TOKEN` and a Folder has
a current semantic generation, `search` embeds the query once and combines the
lexical and semantic rankings. Missing, disabled, building, stale, corrupt,
timed-out, or unavailable semantic state falls back to BM25 without failing the
search or sync. `--lexical-only` bypasses the provider for diagnostics.

Semantic indexing is selected by default for readable Folders. Inspect it with
`search-index status`; `disable --folder` deletes that Folder's vectors but
keeps BM25, while `enable --folder` durably schedules it for the foreground
`daemon watch` worker to rebuild in the background. Status reports only
lifecycle, model contract, and counts—not credentials or wiki text. Folder
selectors use the same readable-Folder and mounted-source rules as `search`.

The lexical index is private disposable state under `.finitebrain/`. It is
maintained from live daemon saves, startup reconciliation, and sync, but it is
not synced content, authoritative knowledge, a backup, or a Recovery Set.

`wiki check` scans Markdown Pages in materialized readable Folders only. It
resolves exact Page titles, unique filenames, and Folder-root-relative Page
paths using the same local-Folder-first ambiguity rule as the Product Client.
The JSON report includes `resolvedLinkCount`, `missingLinkCount`,
`ambiguousLinkCount`, and source-specific `issues`. Resolve missing and
ambiguous links before the final sync; a clean result verifies link targets but
does not by itself prove that the wiki has no orphans or enough meaningful
connections.

## Operation-Scoped Folder Keys

`sync`, daemon, sharing, and access-administration operations reopen the
encrypted Folder Key Grants they need through the acting Member Identity's
signer and retain raw keys only in memory for that operation. The legacy
`fbrain unlock` command is removed and exits unsuccessfully with guidance to
run `fbrain sync now`.

Existing v1 Agent State is atomically migrated before protected work continues:
`localFolderKeys` and `unlockedFolders` are removed and the state becomes v2.
This scrub is not secure erasure from backups, snapshots, filesystem history,
or prior copies.

## Daemon Watch

```sh
fbrain daemon status --json
fbrain daemon watch --poll-ms 250 --json
fbrain daemon watch --poll-secs 5 --remote-poll-ticks 12
fbrain daemon watch --once --json
fbrain daemon watch --max-ticks 3 --json
fbrain daemon watch --poll-only
fbrain daemon tick --json
fbrain daemon logs --json
fbrain daemon stop
```

`daemon watch` is foreground and should run under tmux, systemd, or an agent
supervisor for long-running work. The default strategy is file-aware:
initial sync, sync when readable Brain Working Tree markdown changes are
detected, and bounded periodic remote polling. Use `--remote-poll-ticks 0` to
disable periodic remote polling and `--poll-only` for legacy every-tick syncing.
When an embedding provider is configured, semantic generations refresh on a
separate background worker; provider work never runs inside the sync path.

`daemon status --json` exposes `lastTickAt`, `lastError`, `tickCount`,
`failureCount`, `retryBackoffMillis`, `watchStrategy`, and
`lastLocalChangeCount`.

## Access And Admin

```sh
fbrain access explain <folder-id>
fbrain access list
```

`access` is read-only. Mutations live under the explicit `admin`, `invite`,
`collaborator`, and `mount` workflows. The CLI prepares Folder Key rotation
automatically; never author or pass a raw rotation payload.

```sh
fbrain brain bootstrap-personal --json
fbrain brain create organization "Org Brain" --json
fbrain brain metadata --brain <brain-id>
fbrain brain export --brain <brain-id>

fbrain folder list --brain <brain-id>
fbrain folder create "Notes" --json
fbrain folder create <folder-id> --brain <brain-id> --role folder --access restricted --member <npub>
fbrain folder delete <folder-id> --brain <brain-id> --json
fbrain mount list --brain <brain-id>
```

`folder delete` permanently deletes the named Folder, all descendant Folders,
and every durable object in that subtree. The CLI submits the current expected
Folder IDs and object count so concurrent scope changes fail closed, then
removes the returned `deletedFolderIds` from the local Working Tree projection.
Read [destructive-operations.md](destructive-operations.md) before using it.

In an authenticated Agent Runtime, `brain create organization` atomically makes
the signing Agent and Runtime-authenticated requester initial admins. The
requester identity flag has been removed; a missing or stale Runtime requester
lease fails without creating a Brain. A direct human CLI invocation makes the
signing human the sole initial admin. The new Brain starts empty.

Folder roles are `personal_home`, `brain_ops`, `general`, and `folder` (hyphen
aliases are accepted). Folder access modes are `owner`, `admin_only`,
`all_members`, and `restricted` (hyphen aliases are accepted). For organization
brains, `folder create` defaults to restricted access; for personal brains it
defaults to owner access.

```sh
fbrain admin member add --target <email|NIP-05|npub>
fbrain admin member remove --target <email|NIP-05|npub>
fbrain admin role grant admin --target <email|NIP-05|npub>
fbrain admin role revoke admin --target <email|NIP-05|npub>
fbrain admin folder-access grant --target <email|NIP-05|npub>
fbrain admin folder-access revoke --target <email|NIP-05|npub>

# Normal, convergent Organization Brain collaboration from its Working Tree
fbrain collaborator ensure-admin \
  --target agent@example.finite.vip \
  --json
```

`--target` resolution is unified with `invite brain create`: an email bound
to a Finite account resolves to that account's Member Identity npub through
the server's account authorities, a NIP-05 name resolves through its domain,
and a bare npub (or hex public key) is used directly. An email with neither
a Finite account nor a serving NIP-05 domain fails with the resolver's error
rather than falling back to a guess.

`collaborator ensure-admin` is the normal email-first Organization Brain
sharing operation. Do not precede it with an ad hoc public NIP-05 probe. The
command resolves the Managed Agent Email natively and returns one typed receipt:

- `complete` proves Admin Brain Role plus current Folder readiness across the
  authoritative Folder snapshot.
- `partial` preserves useful progress but names every known incomplete Folder.
  Retry this exact idempotent command from a named current key holder when the
  receipt supplies a holder email. Otherwise ask another current Folder reader
  who can open the listed Folder to retry; never invent or expose a holder
  identity, and do not report the collaboration as complete.
- `indeterminate` means the mutation may have committed but its postcondition
  was not proved. Retry the exact command and inspect its next receipt; do not
  claim either success or a clean failure.

Human reports should use the target email, safe Folder paths, counts, reason
codes, and holder emails. Do not paste the raw receipt or expose Member Identity
keys, wrapped events, auth material, Folder Keys, or grant plaintext.

Low-level `admin` commands are advanced primitives. Member and role commands
change Brain-wide relationships, while `folder-access` targets one Folder;
they do not prove complete Organization Brain Collaboration and are not the
normal sharing workflow.

## Invitations And Sharing

`invite brain create --target <email>` and
`invite folder create --target <email>` are the blessed invite paths. For a
Finite account email the CLI resolves the account's human and managed agents
into one plan (Brain membership, or Guest access to exactly one Folder); a
signer with Brain admin standing commits it directly, everyone else files an
approval request that a Brain admin signs (`fbrain approvals approve` or the
chat approval card; Folder plans additionally need the key-holding
committer). Emails without a Finite account fall back to the one-time email
invitation.

Every invitation receipt carries a self-describing `deliveryStatus`:
`sent` (a courtesy email with the public instructions URL reached the human
account mailbox), `in_app` (the npub-bound invitee — managed agents, or
humans when no mailer is configured — receives it in their authenticated
client, not by email), `not_configured` (server has no invite mailer), and
`failed` (the courtesy email errored after the invitations were already
committed; they remain valid and visible in-app). `in_app` is the normal
outcome for account-backed invitations, not a delivery failure.

`invite brain list` answers two different questions depending on the flag:
with no `--brain` it lists invitations RECEIVED by the acting identity
(invitation ids, inviter, brain display name); with `--brain <id>` it lists
invitations ISSUED on that Brain. To answer "have I been invited to
anything?", run the no-flag form; `brain list --json` rows with
`role: "invited"` are the same incoming invitations from the Brain side.
`invite brain inspect` and `accept` want the invitation id
(`invitation-...`); an invite code (`invite-...`) is resolved to its id by
the code's public `llms.txt` instructions URL.

When your invite files an approval request, tell the user: an approval card
appears in their chat (they can also run `fbrain approvals list`). When the
user asks whether anything is waiting for them, check both sides: your own
`fbrain invite brain list` for invitations addressed to your principal, and
their pending approval and invitation cards in chat.

```sh
fbrain invite brain create --target <email|npub>
fbrain invite brain create --target <email|npub> --expires-in 7d
fbrain invite brain list
fbrain invite brain inspect <invitation-id>
fbrain invite brain accept <invitation-id>
fbrain invite brain revoke <invitation-id>

fbrain approvals list
fbrain approvals approve --id <request-id>
fbrain approvals deny --id <request-id>

fbrain invite folder create --target <email|npub>
fbrain invite folder list
fbrain invite folder inspect <invitation-id>
fbrain invite folder accept <invitation-id>
fbrain invite folder claim <invite-code> --email <email> --invite-secret-file <path>
fbrain invite folder revoke <invitation-id>

fbrain mount offer create --destination-brain <brain-id> --destination-controller <email|npub>
fbrain mount offer list
fbrain mount offer inspect <offer-id>
fbrain mount accept <offer-id>
fbrain mount participant add <mount-id> <email|npub>
fbrain mount participant remove <mount-id> <email|npub>
fbrain mount revoke <mount-id>
```

Invitations and Mount Offers default to seven days and accept `--expires-in`
from `1h` through `30d`. Brain Invitations create Members. Folder Invitations
create bounded Guest access. Mounts are source-backed and work between either
Brain kind; the CLI opens and wraps required Folder grants in memory.
Folder Invitations work for registered identities and unregistered email
addresses. Unregistered recipients claim the bounded invitation after exact
email verification using the delivered Invite Secret file. A Folder's native
access mode remains unchanged; explicit Guest access is orthogonal to it.
