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

## Principal labels and backfill

Labels are display metadata in the Brain database. Public NIP-05 names use the
existing verified alias store. Schema V30 adds `brain_principal_labels` and an
optional Invite Token delivery email. The admin label endpoint writes notes;
first token redemption records `invitation_email` in the same transaction as
membership. Metadata and `fbrain access list` read notes for admins and the
named principal. No Chat store, Core database, worker, socket, or mount is needed.
The delivery address is unverified provenance; possession of a forwarded token
does not prove ownership of its mailbox. Admin notes never resolve keys.

Deploy the server before the CLI. Old clients ignore the additive label field;
new clients accept old metadata with no labels, while label edits against old
servers return an unsupported route. Old servers ignore the added table/column;
SQLite triggers still clear notes on membership/final guest-access removal and
record email provenance if an older server redeems a newly issued token. Old
token creators leave delivery email null. The earlier worker-based draft was
never deployed and is not a supported V30 database input.

The whole consistent Brain SQLite database is the Recovery Set, including label
notes and pending token provenance. Restore it onto an empty target and verify
notes plus unchanged access metadata before rollout. Encrypted Brain export is
not a backup of these server-side notes. Binary rollback retains the additive
schema and notes; it does not require deleting tables or restoring stale data.

For an approved roster backfill, capture a consistent backup and a private
mapping of **Brain ID, exact npub, proposed note, and supporting evidence**.
Re-read current membership/access and check each public NIP-05 name resolves to
that exact key before recording it through the existing identity resolver.
Use `fbrain admin label set --brain "$BRAIN_ID" --target "$TARGET_NPUB" --text "$LABEL"`
for verified-by-the-operator mappings that need a human/agent note. The source
remains `admin_note`; it must not be reported as a verified account binding.
Review mappings per key, leave unknown/conflicting keys unidentified, and never
select a target by roster position. Do not commit private roster mappings.

Verify `fbrain access list --brain "$BRAIN_ID" --json` against the approved mapping,
including source and unchanged admin/Folder access. Correct a mistake with the
same exact-key command or `admin label clear`; labels do not require key rotation.
Run `scripts/finite-status` before and after rollout. Deployment and any live
backfill require their own explicit authorization and recorded target set.

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
