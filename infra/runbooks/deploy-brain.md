# Deploying finite-brain on lat1

Finite Brain runs as `finite-brain-app.service` on finite-lat-1, bound only to
`127.0.0.1:3015`. Caddy exposes its canonical signing/API origin at
`https://brain.finite.computer`. The dashboard also proxies the embedded
Product Client under `https://finite.computer/client`, where WorkOS protects
the user session and issues a bounded capability naming the canonical Brain
origin. Brain still enforces its own route-level Nostr, invitation-proof, or
other narrowly specified authorization; no oauth2-proxy or parent-domain
WorkOS cookie is added to the Brain subdomain.

The SQLite database is `/var/lib/private/finitebrain/finite-brain.sqlite3`.
Compute deployment and data migration are separate operations. Never replace
the database without a byte-for-byte rollback copy.

## Preconditions

- The exact mono commit is pushed and its production NixOS configuration
  evaluates successfully.
- The reviewed revision has a successful `Lat1 NixOS Closure` workflow artifact
  and the deploy operator can SSH to `root@64.34.82.77`. Do not evaluate or
  build the production closure on the Mac, clawland, lat1, or lat2.
- `finite.computer` dashboard auth is healthy and `brain.finite.computer`
  resolves to lat1.
- A consistent SQLite backup has been copied from the current source and its
  size plus SHA-256 recorded outside the database contents.
- The previous NixOS generation and source Brain service remain available for
  rollback until the lat1 Product Client and `fbrain` proofs pass.

## First migration from smoke

1. On smoke, make a SQLite online backup (or briefly stop the service if the
   installed SQLite lacks `.backup`), then restart it immediately. Do not move
   the live file while the service is writing.
2. Copy the backup to a root-only staging path on lat1 and record its SHA-256.
3. Deploy the pinned mono revision. Let systemd create the DynamicUser state
   directory and an empty database if necessary.
4. Stop `finite-brain-app` on lat1, keep a rollback copy of any destination
   database, replace it with the staged backup, match the destination file's
   owner/mode, and start the service.
5. Leave smoke unchanged until verification completes.

## Normal deploy

Build and download the reviewed revision's `lat1-nixos-closure-REV` artifact
with the shared procedure in [deploy-core.md](deploy-core.md#steps). `REV`
must be exactly 40 lowercase hex characters on `origin/main`, not a tag,
branch, abbreviation, or dirty tree.

Deploy that artifact with:

```sh
just deploy-lat1-closure "$ARTIFACT_DIR"
```

The script validates the manifest, copies the prebuilt file binary cache to
lat1, activates it in a transient systemd unit, and proves
`/run/current-system` equals the artifact's exact `SYSTEM` path. It does not
evaluate or build on lat1 or lat2. Brain is built with the rest of the
monorepo from that revision; no source tarball or legacy-repo deploy is part of
the path.

## Verify

```sh
set -euo pipefail
ssh root@64.34.82.77 systemctl is-active finite-brain-app
ssh root@64.34.82.77 curl -fsS http://127.0.0.1:3015/health
curl -fsS https://brain.finite.computer/health
curl -fsS -o /dev/null -w '%{http_code}\n' https://brain.finite.computer/client
curl -fsS -o /dev/null -w '%{http_code}\n' https://finite.computer/client
```

The canonical `/health` route must report the Brain service healthy. The
canonical `/client` may serve the public shell, but it never receives a hosted
user capability; the dashboard `/client` must require a WorkOS session. A
signed `fbrain` request to `/_admin/*` must reach Brain without a WorkOS
session. In an authenticated browser, verify the embedded Product Client loads
and completes a real `/_admin/*` request through the dashboard while signing
for `https://brain.finite.computer`. Then run `fbrain doctor` and a write/read
proof from an authorized Nostr identity against
`https://brain.finite.computer`.

For an invite-delivery change, use a disposable Brain and an operator-owned
inbox. Create one email-targeted Brain Invitation and one email-targeted Folder
Invitation. Both responses and Product Client receipts must report email
delivery as `sent`; both emails must contain the invite code and public
instructions but no URL fragment or Invite Secret. Copy the private invite link
from the unlocked Product Client as the required separate client-only channel,
then revoke both disposable invitations after verification.

## Rollback

1. Switch lat1 to the previous NixOS generation and record the resulting
   `/run/current-system`; for a deliberate rollback, build/download/deploy the
   previous known-good rev's exact closure artifact and verify that path.
2. If Brain data was written on lat1, preserve that database before restoring
   the pre-migration rollback copy; do not discard either side.
3. Keep or restore the smoke service as the temporary endpoint while deciding
   how to reconcile post-cutover writes.

A NixOS rollback is not a data rollback. Offsite Recovery Snapshot and
empty-target restore remain TODO; do not claim them until a restore drill has
passed.
