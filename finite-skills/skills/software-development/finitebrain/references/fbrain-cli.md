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
fbrain brain list|create|rename|bootstrap-personal|metadata|export
fbrain folder create|list|delete
fbrain collaborator ensure-admin
fbrain invite brain create|list|inspect|accept|revoke
fbrain invite-token create|list|revoke
fbrain invite-accept <url-or-token>
fbrain approvals list [--brain <brain-id>] [--all]|approve --id <request-id> [--brain <brain-id>]|deny --id <request-id> [--brain <brain-id>]
fbrain invite folder create|list|inspect|accept|revoke
fbrain mount offer create|list|inspect|revoke
fbrain mount accept|list|inspect|revoke
fbrain mount participant add|remove
fbrain admin member add|remove
fbrain admin role grant|revoke admin
fbrain admin folder-access grant|revoke
fbrain admin ensure-access
```

Use `brain bootstrap-personal` for first-time Personal Brain setup. It creates
the empty user-owned Personal Brain and establishes the authenticated agent as
its Personal Agent through Brain's account-bound authority. Direct `brain
create` is for Organization Brains and is not a substitute for this Personal
Agent bootstrap flow.

## Rename a Brain

With a server and CLI release that support rename, use:

```sh
fbrain brain rename "New display name" --brain <brain-id> --json
# From inside the intended Brain Working Tree:
fbrain brain rename "New display name" --json
fbrain sync now --summary
```

An Organization Brain admin, Personal Brain owner, or active Personal Agent
may rename it. Choose the Brain by stable ID or its already-open Working Tree;
if the intended Brain is ambiguous, resolve it with `brain list --json` first.
The display name changes; the Brain ID, Working Tree path, Folders, content,
membership, and key grants stay intact. The rename is signed and appears in
administrative sync history. Other clients receive the name on refresh or sync.
An older server rejects this command without mutation; upgrade the server
before promoting the matching CLI and managed skill revision.

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
lifecycle, model contract, and counts, never credentials or wiki text. Folder
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
fbrain access list --brain <brain-id> [--json]
fbrain access summary --brain <brain-id> [--json]
```

From `fbrain` 0.7, `access list` is the named access report; its JSON carries
`"version": "finite-brain-access-report-v1"`. Older CLIs print the metadata
summary under `access list` with no `version` field; treat that as "use an
updated operator CLI for the named report", never as a complete report. Do not
roll the Agent fleet for this task. The named report is one server snapshot
with coverage
per scope, exact keys, permitted identity descriptions, Folder entitlements
and their recorded sources, current-grant readiness with issuer and time, and
Folders mounted in from other Brains. Every page must carry the same `authorityFingerprint`;
if Brain access changes mid-read the CLI restarts the report, and it refuses
partial, foreign, or inconsistent pages. It requires the acting key's own
admin standing and fails with `unsupported` on an older server. `access
summary` is the older metadata view of Folder recipients; it names no
identities and proves no coverage.

Descriptions are separate from permission. A `resolved` human shows the
account's shared email; a `resolved` agent shows its name, lifecycle and
responsible account. `notShared` means Core has nothing shared for this Brain,
or (`noParticipation`) the key has not acted in this Brain, so it was never
looked up. `ambiguous`, `unknown` and `unavailable` say exactly that.
`storedNip05` is a stored public name with its stored time, not rechecked and
not a mailbox. Incoming Mount source access can remain unverified even when
native Folder and current-grant checks are complete. Content edits do not
invalidate an access-report cursor.

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
fbrain admin member add --target <npub|hex|NIP-05>
fbrain admin member remove --target <npub|hex|NIP-05>
fbrain admin role grant admin --target <npub|hex|NIP-05>
fbrain admin role revoke admin --target <npub|hex|NIP-05>
fbrain admin folder-access grant --target <npub|hex|NIP-05>
fbrain admin folder-access revoke --target <npub|hex|NIP-05>

# Normal, convergent Organization Brain collaboration from its Working Tree
fbrain collaborator ensure-admin \
  --target agent@example.finite.vip \
  --json
```

`--target` resolution is unified with `invite brain create`: a bare npub (or
hex public key) is used directly, and an email-shaped name resolves only
through public NIP-05. Brain does not consult account authorities; a name
without a matching NIP-05 record fails with the resolver's error rather than
falling back to a guess.

`collaborator ensure-admin` is the normal email-first Organization Brain
sharing operation. Do not precede it with an ad hoc public NIP-05 probe. The
command resolves the Managed Agent Email through public NIP-05 and returns one
typed receipt:

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

`admin member remove` removes the exact target's Admin role, Membership and
direct Folder Access together. It also rotates affected current Folder Keys,
including retained grants for a target whose Membership was already removed.
`admin role revoke admin` removes only the Admin role: Membership and explicit
Folder Access remain, and only Folders that lose entitlement are rotated.
Another current admin must perform these operations; the last admin is
protected. A source-owned Mount that cannot be updated safely stops the
operation and names the blocker before access changes.
The acting identity must also be able to verify every active incoming Mount's
source. Current metadata cannot distinguish restricted direct source access
from access left by a Mount; that overlap blocks removal unless independent
source entitlement can be proved from owner, admin or all-member standing.

The client prepares grants and re-encrypts live content, then checks fresh
authority, current grants and exact content revisions. Inspect the receipt:
`state: complete` proves the operation; `outcome: changed` means access changed,
and `outcome: alreadyComplete` means a checked retry made no further changes.
`folders` records each affected Brain, Folder and old/new key version.
`preservedSourceAccess` names any independently authorized source Folder
access that remains after removing participation in a destination Mount.
Removal from one Brain does not revoke independent access in another Brain.
An older server that cannot perform the guarded operation rejects it. Upgrade
the server before retrying; never demote first to bypass that rejection.

`Result unknown` means the client could not prove completion. Refresh and
retry the same operation; do not report success, restore access, or rotate
again manually. Earlier keys and downloaded copies cannot be recalled, even
after a complete removal.

## Invitations And Sharing

The finitebrain skill's Brain Invitations section is the contract for choosing
between a key-addressed invitation and an email capability invitation, for
`deliveryStatus` values, for membership versus readable Folders, and for the
unsupported cases. This section lists the command syntax.

`invite brain list` lists received invitations only when run without `--brain`
from outside any Brain Working Tree, with the intended server selected.
Inside a Working Tree it infers that Brain and lists issued invitations,
just like `--brain <id>`; that scoped listing requires admin standing.
To answer "have I been invited to anything?", use the outside-tree form; `brain list --json` rows with
`role: "invited"` are the same incoming invitations from the Brain side.
`invite brain inspect` and `accept` want the invitation id
(`invitation-...`); an invite code (`invite-...`) is resolved to its id by
the code's public `llms.txt` instructions URL.

When the user asks whether anything is waiting for them, check both sides:
your own `fbrain invite brain list` outside any Brain Working Tree for
invitations addressed to your principal, and their pending approval and invitation cards in chat.

```sh
fbrain invite brain create --brain <brain-id> --target <npub|hex|NIP-05>
fbrain invite brain create --brain <brain-id> --target <npub|hex|NIP-05> --folder <folder-id> --expires-in 7d
fbrain invite brain list # outside a Working Tree for the incoming inbox
fbrain invite brain inspect <invitation-id>
fbrain invite brain accept <invitation-id>
fbrain invite brain revoke <invitation-id>

fbrain invite-token create --brain <brain-id> --email <address> [--role member|admin] [--expires-in 7d]
fbrain invite-token create --brain <brain-id> [--role member|admin]
fbrain invite-token list --brain <brain-id>
fbrain invite-token revoke --brain <brain-id> --token-id <token-id>
fbrain invite-accept <url-or-token>

fbrain admin ensure-access --brain <brain-id> --target <npub|hex|NIP-05>

fbrain approvals list
fbrain approvals approve --id <request-id>
fbrain approvals deny --id <request-id>

fbrain invite folder create --folder <folder-id> --target <npub|hex|NIP-05>
fbrain invite folder list
fbrain invite folder inspect <invitation-id>
fbrain invite folder accept <invitation-id>
fbrain invite folder revoke <invitation-id>

fbrain mount offer create --destination-brain <brain-id> --destination-controller <npub|hex|NIP-05>
fbrain mount offer list
fbrain mount offer inspect <offer-id>
fbrain mount accept <offer-id>
fbrain mount participant add <mount-id> <npub|hex|NIP-05>
fbrain mount participant remove <mount-id> <npub|hex|NIP-05>
fbrain mount revoke <mount-id>
```

Invitations, Invite Tokens, and Mount Offers default to seven days. The CLI
accepts `--expires-in` in whole hours or days from `1h` through `30d`. An
older Brain server can reject `1h` when the request arrives a few seconds
late; report the error and preserve the requested expiry until the user
chooses a longer duration or the server is updated. A leading client clock
can exceed the strict `30d` ceiling. Brain
Invitations create Members. Folder Invitations create bounded Guest access.
Mounts are source-backed and work between either Brain kind; the CLI opens and
wraps required Folder grants in memory.
Folder Invitations target an exact key, supplied directly or resolved through
public NIP-05; they do not invite an unregistered email address. A Folder's
native access mode remains unchanged; explicit Guest access is orthogonal to it.
