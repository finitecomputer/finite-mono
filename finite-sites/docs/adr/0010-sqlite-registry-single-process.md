# SQLite Registry In One Process

The registry is one SQLite database (WAL mode) owned by one `finitesitesd`
process. Control-plane mutations use one writer Engine. The serving plane uses
a bounded pool of independent query-only connections, and runs registry and
blob reads on Tokio's blocking pool. Unrelated Site reads do not take the
control-plane Engine lock.

Publication stores and verifies immutable content-addressed blobs before
atomically changing `active_version_id`; serving lookups retain that exact
Version id while reading its blobs. Schema constraints and transactions enforce
registry invariants across restarts.

The dedicated Fly volume contains the registry, blobs and Git repositories.
The complete stopped-Sites Recovery Set is archived off-host with Borg;
[backup and empty-target restore](../../../infra/runbooks/deploy-sites.md#backups-and-restore)
cover the cookie key and permission state as well as content. The volume alone
is not a backup.
