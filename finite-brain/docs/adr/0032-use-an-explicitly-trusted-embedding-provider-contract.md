# Use A Replaceable Embedding Provider For The Internal Beta

Status: accepted

Hybrid Wiki Search will obtain vectors through a deployment-neutral Embedding
Provider contract rather than binding FiniteBrain to one model runtime. The
internal beta uses the centralized Finite Specialization service and enables
semantic indexing by default for every readable Folder; beta participants are
informed personally rather than through an in-product consent surface and may
disable semantic indexing per Folder. While enabled, `fbrain` may send Markdown
Section plaintext to that service, which must not retain or log the submitted
text. Semantic retrieval may also send the user's query text to the same
provider under the same no-retention and no-logging rule. An unavailable or
disabled provider leaves search in BM25-only mode and never blocks sync or
agent work.

The Embedding Provider adapter owns the provider-specific endpoint protocol,
batching, and authentication behavior. The Agent Runtime injects endpoint and
credential values securely at startup; search and indexing code see only the
adapter interface. Provider secrets are never stored in the Working Tree,
search index, repository, or synced Brain state.

Semantic egress is fail-closed behind the deployment policy contract
`verified-no-content-logging-no-retention-v1`. The runtime injects an endpoint
and independently revocable worker credential only when the specialization
deployment also presents a private evidence identifier for reviewed upstream
logging and retention controls. The worker authenticates the request and
checks that policy before reading its body or contacting upstream. Missing,
revoked, or inconsistent configuration leaves search lexical-only.

## Consequences

- The centralized beta provider is a temporary rollout choice, not the intended
  production default or a permanent expansion of the Brain server trust model.
- Every newly readable beta Folder starts semantically enabled, but its
  controller can deselect it without disabling local BM25 search. Deselection
  stops new embedding requests and deletes that Folder's locally stored
  vectors. Re-enabling it rebuilds vectors asynchronously while BM25 remains
  available.
- Internal-beta management uses `fbrain search-index status`, `fbrain
  search-index enable --folder <folder>`, and `fbrain search-index disable
  --folder <folder>`. Everyday retrieval remains `fbrain search` and does not
  expose provider configuration choices.
- Provider requests contain only the Page title, heading ancestry, and section
  text needed to produce a vector. They exclude Member Identity, Brain and
  Folder identifiers or names, local filesystem paths, keys, grants, and sync
  metadata.
- The provider accepts bounded batches of changed sections and returns one
  vector per input together with model identity, model version, and vector
  dimension. This supports efficient incremental indexing and deterministic
  invalidation when the model contract changes.
- Query-embedding requests contain only the query text. Query text is not
  retained or logged by the provider.
- Provider health and deployment evidence expose only enablement, model name,
  model version, policy identity, and the non-secret evidence identifier.
  Request diagnostics are restricted to status, timing, batch size, and model
  identity; bodies and bearer credentials are never logged.
- Rollback revokes the worker credential and removes the runtime endpoint.
  Existing lexical indexes and authoritative Markdown remain available while
  all future semantic egress is disabled.
- Returned vectors and their search metadata remain client-side derived state
  and are not uploaded through FiniteBrain sync.
- The search index records provider model identity and version so derived
  vectors can be invalidated and rebuilt.
- A model embedded in the client or Agent Runtime, or an organization-operated
  provider alongside its FiniteBrain deployment, may implement the same
  contract later.
- Production provider selection, in-product consent and disclosure, and the
  intended client-local default require explicit follow-up specifications and
  tickets before the internal-beta behavior can graduate.
- Making the core FiniteBrain server decrypt and embed Folder content changes
  its current plaintext-blind role and requires a separate deployment and
  trust-boundary decision.
