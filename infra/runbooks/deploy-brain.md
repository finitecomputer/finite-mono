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
