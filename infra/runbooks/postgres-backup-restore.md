# Postgres backup and restore

Core uses native Postgres on the app-plane host, finite-lat-2. The database is
`finite_core`, role `finite`; configuration is in `infra/nixos/modules/postgres.nix`.
Local custom-format dumps live under `/data/backups/postgres/`. The coordinated
[Hosted recovery snapshot](hosted-web-chat-recovery.md) provides the off-host
service-consistent copy, including dependent Chat/Brain/Identity state.

## Isolated drill

1. Select an immutable dump with its timestamp, checksum and snapshot manifest.
   Copy it to private scratch storage using read-only access to the source.
   Preserve expected schema and count/identity evidence from that recovery
   point; a historical fixed key count is not a recovery invariant.
2. Start an isolated compatible Postgres target with no application writers,
   network consumers, billing/webhook jobs or public ingress. Create an empty
   scratch database and its role.
3. Inspect the archive using `pg_restore --list`, then restore with:

   ```sh
   pg_restore --exit-on-error --single-transaction --no-owner \
     --dbname="$SCRATCH_DATABASE_URL" "$DUMP"
   ```

   Supply credentials through the normal private credential mechanism. Do not
   print credentials, database content or customer identifiers in evidence.
4. Verify schema, row counts and the specific ownership/identity relationships
   recorded with the snapshot. Test issued Finite Private key/grant continuity
   and Core reads on the isolated stack. A successful SQL restore alone is
   not complete product recovery.
5. Record archive, checksum, versions, elapsed time and pass/fail. Remove the
   scratch database and protected copies after retaining the evidence.

## Production recovery

Requires explicit authorization, an identified Recovery Set and rollback
boundary. Stop all writers and preserve the damaged/current database before
restoring. Prefer an empty replacement target. Restore role ownership using
secret custody; `FC_CORE_DATABASE_URL` must agree with the configured role.
Do not expose passwords in SQL arguments or logs.

Use the coordinated recovery procedure when other services share the recovery
boundary. Verify the restored product relationships before moving traffic;
never overwrite accepted newer writes with an older dump as a routine rollback.
Run `scripts/finite-status` before and after activation.
