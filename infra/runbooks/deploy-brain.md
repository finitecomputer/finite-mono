# Deploying finite-brain on lat2

Finite Brain runs as `finite-brain-app.service` on finite-lat-2, bound only to
`127.0.0.1:3015`. Caddy exposes its canonical signing/API origin at
`https://brain.finite.computer`. Brain enforces its own signed-request and
capability authorization. A dashboard session is not a Folder Key Grant.

The SQLite database is `/var/lib/private/finitebrain/finite-brain.sqlite3`.
Compute deployment and data migration are separate operations. Never replace
the database without a byte-for-byte rollback copy. Continuous WAL replication
to Latitude object storage is enrolled next to chat
(`infra/runbooks/litestream-chat-replication.md`); that lane is DR-only and
does not replace the stop-the-world Recovery Snapshot.

## Preconditions

- The exact mono commit is pushed and its production NixOS configuration
  evaluates successfully.
- The reviewed revision has a successful `Lat2 NixOS Closure` workflow artifact
  and the deploy operator can SSH to `root@64.34.80.19`. Do not evaluate or
  build the production closure on the Mac, clawland, lat1, or lat2.
- `finite.computer` dashboard auth is healthy and `brain.finite.computer`
  resolves to lat2.
- A consistent SQLite backup has been copied from the current source and its
  size plus SHA-256 recorded outside the database contents.
- The previous NixOS generation and source Brain service remain available for
  rollback until the `fbrain` proofs pass.

## Normal deploy

Build and download the reviewed revision's `lat2-nixos-closure-REV` artifact
with the shared procedure in [deploy-core.md](deploy-core.md#steps). `REV`
must be exactly 40 lowercase hex characters on `origin/main`, not a tag,
branch, abbreviation, or dirty tree.

Deploy that artifact with:

```sh
just deploy-lat2-closure "$ARTIFACT_DIR" --prepare
scripts/finite-status
just deploy-lat2-closure "$ARTIFACT_DIR" --activate
```

The script validates the manifest, copies the prebuilt file binary cache to
lat2, dry-activates during `--prepare`, activates only on the explicit
`--activate` boundary, and proves `/run/current-system` equals the artifact's
exact `SYSTEM` path. It does not evaluate or build on lat1 or lat2. Brain is
built with the rest of the
monorepo from that revision; no source tarball or legacy-repo deploy is part of
the path.

## Verify

```sh
set -euo pipefail
ssh root@64.34.80.19 systemctl is-active finite-brain-app
ssh root@64.34.80.19 curl -fsS http://127.0.0.1:3015/health
curl -fsS https://brain.finite.computer/health
```

The canonical `/health` must report Brain healthy. Run `fbrain doctor` and an
authorized write/read proof against `https://brain.finite.computer`. Signed
`/_admin/*` requests use Brain's own authorization and do not require a
WorkOS browser session.

## Restoring explicit Folder access after admin demotion

Admin demotion can remove inherited restricted-Folder permission while leaving
the recipient's encrypted key grant stored. Deploying the server fix does not
restore anyone's permission automatically. Treat each human and agent npub as
a separate principal; resolve the intended recipient before authorizing repair.

After an exact target set is approved, capture the consistent database backup
and deployed revision described above. From an authorized admin's configured
`fbrain` identity holding the Folder key, grant only the reviewed Folder:

```sh
fbrain admin folder-access grant --brain "$BRAIN_ID" --folder "$FOLDER_ID" --target "$TARGET_NPUB" --json
```

This preserves an existing current-version key grant, or supplies one when
missing. It does not restore admin standing. Verify the recipient's explicit
Folder permission and unchanged admin set in authoritative metadata. In the
recipient's Working Tree for this Brain, run `fbrain sync now --summary` and
`fbrain conflicts --json`. Prove restored decryption using a new remote revision
written after demotion, or by bootstrapping a fresh Working Tree. Previously
downloaded plaintext can survive access loss; neither those cached bytes nor
stored grant presence proves restored access. Confirm unrelated Folders remain
inaccessible.

A mistaken grant requires the supported Folder-access revoke operation, which
rotates keys and re-encrypts content; it is not undone by deleting an access
row or rolling back the server binary. Do not restore an old database over
subsequent accepted writes. Coordinate the explicit repair with any concurrent
runtime recovery and run `scripts/finite-status` before and after rollout.

## Private principal labels

The same closure installs `finite-brain-labels.service`, an idle local worker
woken by authorized Brain metadata reads. Sync and the current dashboard both
use that route; `fbrain access list` does too. There is no refresh timer or
startup scan. The server sends a constant, nonblocking Unix datagram after
permission checks, outside the Brain store lock. It never sends a caller-chosen
key or waits for discovery. A missing worker or full queue cannot fail sync.

The worker coalesces demand across all Brains. Successful refreshes have a
five-minute cooldown; failures back off from 30 seconds to five minutes.
Requests during cooldown reuse the projection; they do not schedule future
work. Known hosted key locations are revalidated on eligible refreshes. Full
hosted-directory discovery, including negative results and failed scans, has a
separate fifteen-minute cooldown. Unknown/new human keys can therefore take
up to fifteen minutes plus the next eligible metadata request to be discovered.
There is no periodic reconciliation while idle. Restart discards the location
cache; the next request performs a bounded discovery. Source discovery still
scales with hosted account count, but repeated reads no longer repeat the scan.

Labels derive from Core's linked WorkOS accounts plus the hosted device's
`hosted-web` public account key, and from Core's runner-pinned Agent Principal
plus Project display name. An invitation's delivery email and an agent-creation
request's caller-supplied owner key are never identity evidence. Conflicting
bindings remain unidentified; newly introduced conflicting hosted bindings are
detected on the next complete discovery. Existing members use the same lookup
path as new members without a database backfill.

A cold metadata request may return unidentified keys before the worker finishes.
A subsequent `fbrain access list --brain "$BRAIN_ID"` shows available labels;
one-shot CLI commands do not wait or poll. The current dashboard lists Brains
and folders, not a member roster: opening it wakes the same worker, but this
change does not add a new roster UI. Any future label-rendering view must refetch
metadata with fresh auth evidence and a bounded retry policy.

The exporter runs as the dedicated `finite_brain_labels` Unix user. Its local
Postgres peer-authenticated role has column-level SELECT grants only; no Core
credentials or API tokens are loaded. NixOS provisions those grants once per
database service lifetime, separately from Brain startup, and never starts an
intentionally stopped database to refresh labels. The grant unit is wanted by
PostgreSQL and follows its explicit stop/restart lifecycle. The worker is also wanted by and follows PostgreSQL's explicit stop/restart,
ordered after grants. This recreates its confined mount of `/run/postgresql`
after PostgreSQL recreates that directory. Its Requisite check rejects manual
worker activation while PostgreSQL is stopped without starting the database.
Other source failures leave the worker alive with demand-driven backoff;
metadata reads stay available when the worker is stopped. The worker uses a confined
root filesystem exposing the Nix closure, Postgres socket, read-only source
directories, and writable output.
Its sole capability permits reading the DynamicUser-owned source files inside
that filesystem. It starts a read-only Postgres transaction and opens Brain
and hosted Chat SQLite files read-only. It queries public identity columns only;
it never queries identity secret files, messages, ciphertext, invite tokens,
or encrypted grants. The retained source contract is
`users(workos_user_id, normalized_email, link_status)`,
`projects(id, display_name)`, `agent_runtimes(project_id, health_reporting_npub)`,
and the hosted `client_device_states(account_id, device_id)` row inside the
SHA-256 WorkOS-subject namespace. It first finds hosted public keys matching
current Brain principals, then queries Core for those exact hashed subject
namespaces; unrelated enrolled Core accounts do not consume the label limit.
Source schema changes must preserve or update this reader. Missing or ambiguous
hosted state is never repaired by this worker.

`/run/finite-brain-labels/principals.json` is an atomic, disposable private
projection, owned by the dedicated worker and readable only by the label group
(directory 0750, file 0640). Brain receives its path through
`FINITE_BRAIN_PRINCIPAL_LABELS` and the signal socket through
`FINITE_BRAIN_LABEL_SOCKET`; it receives no Core credential. The socket is
worker-owned, mode 0660 inside the 0750 directory. Brain can signal but cannot
replace the socket or write the projection. Only authorized
Brain metadata responses use labels, for the principals already in that
response. Each principal can see their own private label. Brain admins may
also see labels for members who accepted a Brain invitation or explicitly
shared their label. The invitation default applies to retained accepted
memberships too. An explicit hide always overrides it. Direct additions, Folder
grants, admin promotion, and declared bootstrap requester identities do not
establish that sharing boundary. Other members and Folder guests cannot
see another principal's private label. Visibility of a public key alone does
not expose its account email. Global identity resolution and public NIP-05
aliases are unchanged.
Human/agent type, source, and observation time accompany the display label.

Existing directly added members and Folder guests can inspect and share their
own label with `fbrain brain label status --brain "$BRAIN_ID"` and
`fbrain brain label share --brain "$BRAIN_ID"`. Each human/agent key acts for
itself; there is no admin target selector or manual name override. Use
`fbrain brain label hide --brain "$BRAIN_ID"` to withdraw private-label sharing.
These choices affect only admin visibility, never membership, permissions,
keys, public NIP-05 aliases, or the principal's own label. A choice made before
a verified label exists applies when a later refresh resolves it.

A missing, malformed, oversized, future-dated, or more-than-one-hour-old
projection supplies no private labels. One hour is the hard display lifetime,
separate from the five-minute refresh cooldown. Failure preserves the last
projection until it expires; a successful refresh replaces it atomically.
Each refresh is bounded to 4,096 relevant principals/bindings and 30 seconds
(with synchronous SQLite discovery bounded to 25 seconds). Private-label
sharing is rechecked on every response: hiding does not wait for cache expiry. Brain access and sync do not depend on the worker. On an authorized
rollout, verify existing and newly admitted identities with `fbrain brain
metadata --brain "$BRAIN_ID" --json` and an updated `fbrain access list --brain
"$BRAIN_ID" --json`; unknown keys remain unverified. Old clients ignore the
additive label metadata, and the new CLI accepts older servers without it.

Brain schema V30 adds `brain_identity_label_preferences` and two cleanup
triggers; it leaves existing customer rows and Chat schemas unchanged. The
signed self-only endpoint is the preference writer; metadata and preference
status are its readers. The access check and preference write are one immediate
transaction. Membership deletion or the last guest-access deletion clears the
choice, including when an older binary issues those retained SQL statements.
An unrelated future direct re-add cannot inherit the former sharing choice.

Preferences are durable privacy state in the whole Brain SQLite Recovery Set:
restore explicit `hide` rows with the rest of that database. The disposable
projection can be regenerated from verified source bindings and is not part of
the Recovery Set. Tests cover V29 upgrade, retained deletion forms, restart,
and a stopped-writer whole-database backup restored onto an empty target,
including hidden labels staying absent from admin metadata. The old server
ignores the additive table and optional metadata; the new sharing command
requires the new server. Rolling back to the pre-label closure removes the
label consumer/worker while retaining preferences, triggers, and accepted access
repairs. No source identity or encrypted state is rewritten. The observer's
read-only Postgres role/grants may remain after binary
rollback; remove that observer role deliberately if retiring the label service.
Do not manually edit the projection to assert an unverified name.

Before rollout, exercise the evaluated Linux service on synthetic sources:
verify socket/output ownership, idle startup without discovery, authorized
metadata wakeup, burst coalescing, source failure, and restart recovery. Nix
evaluation and process tests do not substitute for this confined-systemd
canary. Use `scripts/finite-status` before and after the authorized rollout.

## Rollback

1. Switch lat2 to the previous NixOS generation and record the resulting
   `/run/current-system`; for a deliberate rollback, build/download/deploy the
   previous known-good rev's exact lat2 closure artifact and verify that path.
2. Preserve the current database and all accepted writes. A binary rollback is
   safe only if the old binary can read the current schema. Otherwise restore
   and diagnose on an isolated target; never start a second writable service.

A NixOS rollback is not a data rollback. Continuous Litestream replication is
the between-deploy restore lane (see
`litestream-chat-replication.md`); empty-target restore still requires an
explicit drill before claiming it.
