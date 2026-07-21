# Keep Hybrid Wiki Search Available During Semantic Indexing

Status: accepted

The Agent Sync Daemon will update lexical search automatically as Brain Working
Tree content changes and will generate embeddings asynchronously for new or
changed Markdown Sections. `fbrain search` must never wait for semantic
indexing: it returns BM25-ranked results while embeddings are missing, stale,
or unavailable and automatically incorporates semantic results when they are
ready.

One incremental index updater receives changes from three sources: the live
Working Tree file watcher, reconciliation when the Agent Runtime starts after
being offline, and files materialized or removed by FiniteBrain sync. It updates
or removes only affected sections. A complete rebuild is reserved for a
missing or corrupt index, an incompatible index format, or an embedding-model
change that invalidates existing vectors.

A Folder's first semantic generation becomes searchable only after all of its
current sections are embedded, so processing order cannot bias early hybrid
results. Until then that Folder remains BM25-only. After initial activation,
unchanged current vectors remain usable while individual changed sections are
refreshed incrementally.

## Consequences

- The embedding capability is an optional quality layer, not a search
  availability dependency.
- Search may temporarily operate in lexical-only mode while remaining the same
  user-facing command.
- The internal beta exposes `--lexical-only` for direct BM25-versus-hybrid
  evaluation. There is no semantic-only user mode; automatic hybrid remains
  the normal product behavior.
- Index state must distinguish current lexical data from pending or stale
  semantic data.
- Embedding failure cannot block Working Tree sync or ordinary agent work.
