# Run documents
Status: ACTIVE
1. A run doc scopes one bounded outcome.
2. Its header names status, owner, opened date, acceptance, and expiry.
3. PROPOSED drafts have no work authority; Paul flips them to ACTIVE.
4. ACTIVE run docs are the only work queues.
5. Work them top-down and record unrelated work as one line in parking-lot.md.
6. Every item retained in an ACTIVE run is a closure prerequisite. Move optional work to `parking-lot.md` or a later proposed run before activation; priority controls order and risk, not whether an item may be skipped.
7. Link to doctrine and ADRs; do not restate them.
8. Put the named human acceptance test last. A run ends only when every retained queue item is complete and that final acceptance passes.
9. Expiry is a stop-and-rescope boundary, not acceptance. An incomplete expired run loses work authority until its owner explicitly extends or replaces it.
10. On acceptance, extract durable decisions and delete the doc in the closing commit; Git history is the archive and `parking-lot.md` is the sole permanent resident inside `docs/runs/`.
