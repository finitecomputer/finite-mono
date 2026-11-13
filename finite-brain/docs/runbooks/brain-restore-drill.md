# Brain restore drill

The server Recovery Set contains ciphertext and access facts. Clients retain
their identity and Folder keys; restoring the server database does not recreate
lost client keys. Use [Litestream recovery](../../../infra/runbooks/litestream-chat-replication.md)
or the [coordinated snapshot](../../../infra/runbooks/hosted-web-chat-recovery.md).

1. Select a verified archive and compatible service revision. Restore onto an
   empty isolated target with writers and outbound side effects disabled. Do
   not restore over a live database or copy an open SQLite file without its WAL.
2. Check integrity with `scripts/snapshot-sqlite` or a scratch copy. Preserve
   the current database untouched as the rollback boundary.
3. Start the isolated `finite-brain-app` service using the configured
   StateDirectory and ownership. Verify health and authorized `fbrain` access.
4. Using synthetic identities, prove membership, grant provenance, pending
   invitations, revoked access and Folder Key Grants survived. A revoked
   Principal must stay revoked. Key-holding clients must sync retained content
   and complete pending key wraps without gaining unrelated access.
5. Record the archive, checksum, service version and count-only results.
   Moving production traffic is a separate authorized operation.

The process acceptance test `built_fbrain_process_brain_restore_drill` exercises
shutdown, copy, empty-target restore and permission continuity with real CLI
and server processes. A green backup timer alone is not this proof.

## Deleted Organization Brains

A deleted Organization Brain stays deleted on its live server: the
`deleted_brains` record retires its id, a trigger refuses to reinsert it, and
former readers get `410 Gone`. Recover it by restoring a backup taken before
the deletion onto an empty isolated target as above; never copy its rows back
into the live database. Client identity keys come from their own backups.
`built_fbrain_deletes_v1_and_leaves_content_and_marketing_brains_intact`
proves this for one Brain database and one admin identity key, not for the
hosted five-part Recovery Set.
