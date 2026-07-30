# Partition Hybrid Wiki Search By Folder

Status: accepted

FiniteBrain will keep derived Hybrid Wiki Search indexes physically separated
by Folder so searchable plaintext follows the same boundary as Folder Access
and Folder Keys. One `fbrain search` operation may fan out across every Folder
currently readable by the acting Member Identity and merge those results, but
revocation, removal, or rebuilding of one Folder must not depend on filtering
or rewriting a Brain-wide plaintext index.

Cross-Folder search is the default. Callers may narrow a search explicitly with
one or more repeatable `--folder` filters. Search scope is never inferred from
the process's current directory because that would make identical commands
silently search different knowledge sets.

## Consequences

- Agents experience one cross-Folder search rather than one search per Folder.
- Agents can deliberately restrict retrieval to one or more Folders without
  issuing separate searches.
- Search candidate generation begins from the currently readable Folder set.
- Each Folder index can be removed or rebuilt independently when its access or
  content lifecycle changes.
- Each Folder index persists locally across restarts to avoid unnecessary
  re-indexing and embedding requests, but remains disposable derived state that
  can be rebuilt from the Folder's Markdown Pages.
- For the internal beta, indexes rely on the same private, owner-only Agent
  Runtime boundary as the plaintext Working Tree rather than introducing a
  second encryption and key-management layer. Indexes remain outside the wiki
  and are never synced.
- Removing a Folder Working Tree or losing readable access deletes that
  Folder's complete local search index.
- Before broader rollout, a placeholder hardening ticket must reassess local
  index encryption at rest and align the decision with the production client
  security model.
- The implementation accepts extra per-Folder bookkeeping in exchange for a
  simpler confidentiality and revocation boundary.
