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

The same closure installs `finite-brain-labels.service` and its one-minute
refresh timer. It derives labels for existing Brain principals from Core's
linked WorkOS accounts plus the hosted device's `hosted-web` public account key,
and from Core's runner-pinned Agent Principal plus Project display name. An
invitation's delivery email and an agent-creation request's caller-supplied
owner key are never identity evidence. Conflicting bindings remain unidentified.
Newly admitted members appear after the next successful refresh (normally
within 65 seconds); existing members use the same path without a database backfill.

The exporter runs locally under the operator boundary. It loads the existing
Core database credential by name from `/etc/finite/core.env`, starts a read-only
Postgres transaction, and opens Brain and hosted Chat SQLite files read-only.
It queries public identity columns only; it never reads identity secret files,
messages, ciphertext, invite tokens, or encrypted grants. The retained source
contract is `users(workos_user_id, normalized_email, link_status)`,
`projects(id, display_name)`, `agent_runtimes(project_id, health_reporting_npub)`,
and the hosted `client_device_states(account_id, device_id)` row inside the
SHA-256 WorkOS-subject namespace. Source schema changes must preserve or update
this reader. Missing or ambiguous hosted state is never repaired by this worker.

`/run/finite-brain-labels/principals.json` is an atomic, disposable private
projection, owned by root and readable only by the label group (directory
0750, file 0640). Brain receives its path through
`FINITE_BRAIN_PRINCIPAL_LABELS`; it receives no Core credential. Only authorized
Brain metadata responses use labels, for the principals already in that
response. Global identity resolution and public NIP-05 aliases are unchanged.
Human/agent type, source, and observation time accompany the display label.

A missing, malformed, oversized, future-dated, or more-than-five-minute-old
projection supplies no private labels. The refresh is bounded to 4,096 source
accounts/principals and 35 seconds; failure leaves the previous projection to
expire. Brain access and sync do not depend on the worker. On an authorized
rollout, verify existing and newly admitted identities with `fbrain brain
metadata --brain "$BRAIN_ID" --json` and an updated `fbrain access list --brain
"$BRAIN_ID" --json`; unknown keys remain unverified. Old clients ignore the
additive label metadata, and the new CLI accepts older servers without it.

No Brain or Chat schema migration is required. Labels can be regenerated from
retained source bindings after an empty-target service restore; the projection
is not part of the Recovery Set. Rolling back the closure removes the label
consumer/timer without undoing or rewriting source state or accepted access
repairs. Do not manually edit the projection to assert an unverified name.

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
