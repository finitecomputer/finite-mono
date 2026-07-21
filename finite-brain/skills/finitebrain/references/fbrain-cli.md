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
  then legacy `FINITE_BRAIN_PUBLIC_BASE_URL`.

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
fbrain auth status|import [--file <path>]
fbrain signer status|public-key|sign|encrypt|decrypt
fbrain daemon status|start|stop|logs|tick|watch
fbrain sync status|now [--summary]
fbrain open <brain-id> [path]
fbrain status [--json]
fbrain conflicts
fbrain resolve <id>
fbrain search <query> [--folder <folder>...] [--limit <1-50>] [--lexical-only] [--json]
fbrain activity
fbrain wiki check [--json]
fbrain access explain|list|grant|revoke
fbrain brain list|create|bootstrap-personal|metadata|export
fbrain folder create|list
fbrain mount list
fbrain permissions add-member|remove-member|add-admin|remove-admin|grant-folder
fbrain invites create|show|accept|revoke
fbrain share link|accept|revoke|source|folder-invite|folder-accept
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

Use `auth status --json` to confirm the acting npub, identity file, and config
directory. Do not print or request secrets during normal agent work.

## Working Tree And Sync

```sh
fbrain doctor --server "$SERVER"
fbrain brain list --server "$SERVER" --json
fbrain open <brain-id> <tree-path> --server "$SERVER"
cd <tree-path>
fbrain status --json
fbrain sync status --json
fbrain sync now --summary
fbrain sync now --json
fbrain conflicts --json
fbrain resolve <conflict-id>
fbrain search "credential rotation" --json
fbrain activity
fbrain wiki check --json
```

`open` creates `.finitebrain/` state, saves the server URL when provided, marks
the daemon running, and attempts an initial sync. `sync now` fetches the encrypted
export, opens available grants, pushes local markdown changes, bootstraps latest
state, and materializes readable Folders back into the tree.

When the path is omitted, `open` uses `$FBRAIN_WORKING_TREE_ROOT/<brain-id>` if
configured, otherwise `<current-directory>/<brain-id>`. The hosted runtime sets
`FBRAIN_CONFIG_DIR=/data/agent/fbrain` and
`FBRAIN_WORKING_TREE_ROOT=/data/workspace/finitebrain`.

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
sync disposition, and lexical signal. The default is ten results and the
maximum explicit limit is fifty.

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

`daemon status --json` exposes `lastTickAt`, `lastError`, `tickCount`,
`failureCount`, `retryBackoffMillis`, `watchStrategy`, and
`lastLocalChangeCount`.

## Access And Admin

```sh
fbrain access explain <folder-id>
fbrain access list --brain <brain-id>
fbrain access grant --brain <brain-id> --folder <folder-id> --target <npub>
fbrain access revoke --brain <brain-id> --folder <folder-id> --target <npub>
fbrain access revoke --brain <brain-id> --folder <folder-id> --target <npub> --rotation-body rotation.json
```

`access grant` delegates to `permissions grant-folder` and requires the current
agent to have the Folder Key opened for the Folder's current key version.
`access revoke` refuses unsafe metadata-only removal unless `--rotation-body`
contains `newKeyVersion`, `grants`, `reencryptedRecords`, and
`accessChangeEvent`.

```sh
fbrain brain bootstrap-personal --server "$SERVER" --json
fbrain brain create <brain-id> --kind organization --name "Org Brain"
fbrain brain create <brain-id> --kind organization --name "Org Brain" --requesting-user-npub <npub|hex>
fbrain brain metadata --brain <brain-id>
fbrain brain export --brain <brain-id>

fbrain folder list --brain <brain-id>
fbrain folder create <folder-id> --brain <brain-id> --name Notes --path Notes
fbrain folder create <folder-id> --brain <brain-id> --role folder --access restricted --member <npub>
fbrain mount list --brain <brain-id>
```

`--requesting-user-npub` is Organization Brain-only. It atomically makes the
distinct signing creator and authenticated requester initial members and
admins. The new Brain starts empty, so it creates no Folder Key Grants until an
admin creates a Folder. Pass only authenticated sender metadata; the option
does not resolve email or NIP-05 input.

Folder roles are `personal_home`, `brain_ops`, `general`, and `folder` (hyphen
aliases are accepted). Folder access modes are `owner`, `admin_only`,
`all_members`, and `restricted` (hyphen aliases are accepted). For organization
brains, `folder create` defaults to restricted access; for personal brains it
defaults to owner access.

```sh
fbrain permissions add-member --brain <brain-id> --target <npub>
fbrain permissions remove-member --brain <brain-id> --target <npub>
fbrain permissions add-admin --brain <brain-id> --target <npub>
fbrain permissions remove-admin --brain <brain-id> --target <npub>
fbrain permissions grant-folder --brain <brain-id> --folder <folder-id> --target <npub>
```

## Invitations And Sharing

```sh
fbrain invites create --brain <brain-id> --target <npub> --folder <folder-id>
fbrain invites create --brain <brain-id> --target <npub> --expires 2099-01-01T00:00:00Z
fbrain invites show --code <invite-code>
fbrain invites accept --code <invite-code>
fbrain invites accept --brain <brain-id> --id <invitation-id>
fbrain invites revoke --brain <brain-id> --id <invitation-id>

fbrain share link --brain <brain-id> --folder <folder-id> --target <npub>
fbrain share link --brain <brain-id> --folder <folder-id> --target <npub> --personal-mount
fbrain share accept --id <share-link-id>
fbrain share revoke --id <share-link-id>
fbrain share source --brain <brain-id> --folder <folder-id>
fbrain share folder-invite --brain <brain-id> --folder <folder-id> --destination-brain <brain-id> --destination-admin <npub>
fbrain share folder-accept --id <shared-folder-invitation-id>
```

Share-link and shared-folder invitation creation need the source Folder Key
opened locally so the CLI can wrap the grant for the recipient or destination
admin.
