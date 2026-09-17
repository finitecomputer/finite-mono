# Brain persistence

`finite-brain-store` owns authoritative server state in SQLite through
synchronous `rusqlite` behind a narrow storage interface. Durable invariants
belong in schema, constraints and transactions; route and CLI code use that
interface. Tests include reopen, rollback and supported historical schemas.

Back up consistent state and prove an independent restore before claiming
recoverability. See [the restore drill](../runbooks/brain-restore-drill.md).
