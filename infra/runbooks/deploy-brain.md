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

`finite-brain-labels.service` is a demand worker, with no timer, startup scan,
or one-shot mode. After authorizing a metadata request, Brain sends the validated
Brain ID through a nonblocking Unix datagram. Sync, dashboard inventory, and
`fbrain access list` use this route. The worker reads that Brain's roster itself;
the request cannot supply a public key or label. Missing workers and full queues
do not fail the read.

The worker publishes one atomic `<brain-id>.json` file per Brain under
`/run/finite-brain-labels`. Roster and output limits derive from the supported
per-Brain Member and Folder Access envelope; other Brains do not consume that
budget. Files contain resolved labels, human/agent type, source type, and
observation time. Internal source identifiers and conflicting candidate records
stay in the worker. Brain reads and validates the requested file outside the
BrainStore lock, then checks current sharing choices with one bounded query.
Labels never confer membership, Folder Access, keys, or admin standing.

Successful files have a five-minute refresh cooldown. One worker processes
requests sequentially, with no cross-Brain success delay and global
failure backoff from 30 seconds to five minutes. Requests during cooldown do not
schedule later work. Cached labels expire after one hour; sharing choices are
checked on every response, so hide does not wait for cache expiry.

Hosted identity discovery has a separate global fifteen-minute cooldown,
including negative results and failed scans. It streams public keys and hashed
storage namespaces into a temporary, worker-private SQLite index with a fixed
page-cache budget. The index has no account names, emails, or identity secrets;
only a complete successful discovery replaces it. It serves subsequent requests
for different Brains without a new scan. Requested keys are revalidated at their
indexed locations before use. Corrupt sources fail discovery rather than hiding
a potentially conflicting owner. New/conflicting bindings are found on the next
complete discovery, independent of Brain additions or caller activity.

The index is disposable and lost on restart. No index schema is added to Brain,
Chat, or Core. Discovery still scans hosted accounts and has a 25-second time
budget; a corpus that cannot complete within that budget requires a discovery
redesign before expansion, not an increased Principal limit. Each refresh has a
30-second deadline. Source failures preserve the old file until expiry. A bad
Brain roster cannot invalidate another Brain's output.

A cold read can precede discovery. Repeat `fbrain access list --brain
"$BRAIN_ID"` after the worker completes. Newly created hosted bindings can wait
fifteen minutes plus the next eligible request. CLI reads do not poll. The
current dashboard lists Brains and Folders, not member identities; this change
adds no roster UI.

Identity evidence comes from linked Core WorkOS accounts joined to the hosted
`hosted-web` public account key, or runner-pinned Agent keys joined to Project
names. Invitation delivery emails and caller-supplied owner keys are not identity
evidence. The retained source contract is:

- Core `users(workos_user_id, normalized_email, link_status)`;
- Core `projects(id, display_name)` and
  `agent_runtimes(project_id, health_reporting_npub)`;
- hosted `client_device_states(account_id, device_id)` in the SHA-256
  WorkOS-subject storage namespace.

Source schema changes must preserve or update this reader. It never repairs
source state or reads secrets, messages, ciphertext, invitations, or grants.
The dedicated Unix user and peer-authenticated Postgres role have only the
required column-level SELECT grants. The confined filesystem exposes read-only
sources, the Nix closure, the Postgres socket, and writable output. Its sole
capability allows reading DynamicUser-owned source files inside that filesystem.
The temporary index remains private to the worker; label files are group-readable
0640 inside a 0750 directory. Brain can signal the 0660 socket and read files,
but cannot replace them. Configure `FINITE_BRAIN_PRINCIPAL_LABELS_DIR` and
`FINITE_BRAIN_LABEL_SOCKET`; Brain receives no Core credential.

Postgres wants the grant unit and worker. Both follow its stop/restart lifecycle;
the worker starts after grants so its confined Postgres socket mount is recreated.
Requisite prevents either unit from starting an intentionally stopped database.
Other source errors leave the worker alive with bounded backoff. Label failure
does not change Brain access or prevent sync.

A Principal always sees its own label. Admin visibility requires accepted
invitation provenance or explicit per-Brain sharing; explicit hide overrides the
invitation default, including retained accepted memberships. Adding an arbitrary
key, granting Folder Access, or promoting an admin cannot reveal private identity.
Other Members and Guests do not see another Principal's private label. Public
NIP-05 aliases and global identity resolution are unchanged.

Directly added Members and Guests use `fbrain brain label status|share|hide
--brain "$BRAIN_ID"`. Each human or Agent acts with its own key; there is no admin
target selector or manual name override. Existing-member rollout still requires
that sharing step and verification of the intended identities. Unknown or
conflicting keys remain unidentified.

Schema V30 stores sharing preferences in the whole Brain SQLite Recovery Set.
The signed self-only route checks access and writes the preference in one
immediate transaction. Membership or last-guest-access deletion clears the old
choice, including retained SQL from older binaries. Restore explicit hide rows
with the complete database. Tests cover V29 upgrade, old-writer deletion,
restart, and empty-target recovery. Label files and the temporary index are
regenerated and are not part of the Recovery Set. Per-Brain files remain in the
runtime directory until reboot; expiry controls use, not file deletion.

Old clients ignore additive label metadata; new clients accept older servers
without it. The share/hide command requires the new server. The worker format and
configuration are an unreleased hard cut: deploy worker and server from the same
closure. Rollback to the pre-label closure removes their consumers but retains
privacy preferences and accepted repairs. Read-only observer grants can remain
or be removed deliberately when retiring the feature. Do not edit label files
to assert a name, or restore an old database over later accepted writes.

Before rollout, exercise the confined Linux service on synthetic sources:
verify permissions, idle startup, authorized Brain-scoped wakeups, multiple
Brains, burst handling, source failure, and Postgres stop/start recovery. Nix
evaluation and process tests do not replace this systemd canary. Use
`scripts/finite-status` before and after the authorized rollout; verify labels
with Brain metadata and access-list reads from the intended identities.
