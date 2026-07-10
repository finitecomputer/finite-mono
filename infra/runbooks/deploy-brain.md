# Deploying finite-brain on lat1

Finite Brain runs as `finite-brain-app.service` on finite-lat-1, bound only to
`127.0.0.1:3015`. The dashboard proxies `/client` and `/_admin/*` to that
loopback service. WorkOS protects both paths as part of the same
`finite.computer` session; Brain still verifies Nostr request proofs for its
data operations. There is no second Brain vhost and no oauth2-proxy.

The SQLite database is `/var/lib/private/finitebrain/finite-brain.sqlite3`.
Compute deployment and data migration are separate operations. Never replace
the database without a byte-for-byte rollback copy.

## Preconditions

- The exact mono commit is pushed and its production NixOS configuration
  evaluates successfully.
- `finite.computer` dashboard auth is healthy.
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

```sh
nixos-rebuild switch --target-host root@64.34.82.77 \
  --flake github:finitecomputer/finite-mono/<exact-rev>#finite-lat-1
```

Brain is built with the rest of the monorepo from that revision; no source
tarball or legacy-repo deploy is part of the path.

## Verify

```sh
ssh root@64.34.82.77 systemctl is-active finite-brain-app
ssh root@64.34.82.77 curl -fsS http://127.0.0.1:3015/health
curl -fsS -o /dev/null -w '%{http_code}\n' https://finite.computer/client
```

The public `/client` request must require a WorkOS session. In an authenticated
browser, verify the Product Client loads and completes a real `/_admin/*`
request through the dashboard. Then run `fbrain doctor` and a write/read proof
from an authorized Nostr identity against `https://finite.computer`.

## Rollback

1. Switch lat1 to the previous NixOS generation.
2. If Brain data was written on lat1, preserve that database before restoring
   the pre-migration rollback copy; do not discard either side.
3. Keep or restore the smoke service as the temporary endpoint while deciding
   how to reconcile post-cutover writes.

A NixOS rollback is not a data rollback. Offsite Recovery Snapshot and
empty-target restore remain TODO; do not claim them until a restore drill has
passed.
