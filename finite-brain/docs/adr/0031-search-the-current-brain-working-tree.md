# Search The Current Brain Working Tree

Status: accepted

Hybrid Wiki Search will index the current saved Brain Working Tree, including
member-authored changes that have not yet synced and local content involved in
a sync conflict. Search results must identify local-only or conflicted state so
an agent can distinguish its working view from the latest server-accepted
revision without making locally relevant knowledge disappear.

## Consequences

- Saved local edits become lexically searchable without waiting for sync.
- Semantic indexing follows those saved local changes asynchronously.
- Search results need enough sync disposition to distinguish accepted,
  local-only, and conflicted content.
- Search remains a local working aid and does not assert that every result is
  the current server-authoritative revision.
