# SQLite Registry In One Process (v1)

The registry is one SQLite database (WAL mode) owned by one finitesitesd
process. Control-plane mutations use one writer Engine. The serving plane uses
a bounded pool of independent query-only connections, and runs registry reads,
blob reads, and document rendering on Tokio's blocking pool. Schema and record
shapes stay portable (TEXT ids, INTEGER unix seconds, no SQLite-isms in the
store API) so a Postgres port stays mechanical if multi-process ever demands
it.

The read/write split was added on 2026-07-23 after the original single Engine
mutex was found on every static and document request. Content activation stays
safe because publication stores and verifies immutable content-addressed blobs
before atomically changing `active_version_id`; serving lookups retain that
exact version id while reading its blobs.

SQLite-on-the-host also lines up with the production backup story:
Litestream replicates the registry the same way it will replicate tier-2
tenant databases.

**Considered Options**

- Postgres from day one (like finite Core): operationally heavier than the
  one-box v1 needs, and the prototype's schema ports either way.
- SQLite + WAL + Litestream seam: smallest honest footprint; chosen.
- One SQLite writer plus bounded query-only serving readers: preserves the
  one-process deployment and transaction model without serializing unrelated
  site traffic; chosen when serving concurrency was added.
