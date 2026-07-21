# Spec: Hybrid Wiki Search For Brain Working Trees

## Problem Statement

FiniteBrain deliberately keeps an LLM Wiki simple: authorized knowledge is
materialized as ordinary Markdown folders and files in a private Brain Working
Tree, and an agent reads original Pages and reasons over them in its own context
window. That model is transparent, portable, and secure, but normal filesystem
search becomes less effective as a wiki grows. Exact words and filenames work
well; paraphrases, related concepts, and information buried under headings are
easier to miss.

FiniteBrain needs a stronger retrieval layer without turning the wiki into a
second knowledge database, moving authority away from Markdown, requiring an
answer-generating RAG service, or making remote embeddings a dependency for
ordinary agent work. The first rollout is an internal beta and may temporarily
trust centralized Finite Specialization with narrowly scoped plaintext, but it
must preserve a clean path to client-local and organization-operated embedding
models.

## Solution

Add Hybrid Wiki Search to `fbrain` over the current Brain Working Tree. A single
`fbrain search` operation searches every Folder currently readable by the
acting Member Identity and returns one ranked list of Search Evidence pointing
back to original Markdown Sections. Callers can explicitly narrow the scope to
one or more Folders, but search never infers scope from the process's current
directory.

BM25 is the always-available lexical base. When current semantic vectors exist,
the same command also embeds the query, runs semantic retrieval, and combines
the independent rankings through rank-based fusion. Missing, disabled, stale,
or unavailable embeddings silently reduce search to BM25 rather than blocking
sync or agent work.

Each readable Folder has its own persistent, disposable local index. Markdown
headings define the retrieval units, while live file changes, startup
reconciliation, and FiniteBrain sync feed one incremental indexing pipeline.
For the internal beta, a replaceable Embedding Provider adapter calls a
centralized Finite Specialization batch endpoint. The adapter owns the provider
protocol and authentication behavior; the Agent Runtime injects endpoint and
credential values. Vectors remain local, indexes are never synced, and the
provider must not log or retain submitted sections or queries.

## User Stories

1. As an agent working in a large LLM Wiki, I want one command to search all knowledge I can currently read, so that I do not have to search every Folder separately.
2. As an agent, I want search to return original file and heading locations, so that I can open authoritative Markdown and reason over it normally.
3. As an agent, I want exact terms, identifiers, filenames, and rare phrases to rank well, so that precise technical lookup remains dependable.
4. As an agent, I want paraphrases and conceptually related language to be discoverable, so that I can find relevant knowledge even when my query uses different words.
5. As an agent, I want BM25 and semantic retrieval to work through the same command, so that I do not need to choose a retrieval technology before every search.
6. As an agent, I want search to keep working when the embedding service is unavailable, so that remote inference never blocks my work.
7. As an agent, I want search to work while embeddings are still being generated, so that a newly materialized wiki is useful immediately.
8. As an agent, I want a single merged result list across readable Folders, so that relevance rather than Folder boundaries determines what I inspect first.
9. As an agent, I want to repeat a Folder filter when I need a narrow search, so that I can deliberately limit retrieval without issuing separate commands.
10. As an agent, I want identical search commands to have identical scope regardless of my current subdirectory, so that retrieval is predictable.
11. As an agent, I want the top ten results by default, so that search evidence fits comfortably in my working context.
12. As an agent, I want to request as many as fifty results explicitly, so that I can broaden an investigation without receiving an unbounded dump.
13. As an agent, I want readable terminal output, so that I can inspect search interactively.
14. As an agent, I want stable JSON output, so that tools can consume Search Evidence without parsing presentation text.
15. As an agent, I want every result to identify its Folder, Page path, Page title, heading, and excerpt, so that I understand and can navigate the match.
16. As an agent, I want every result to report whether BM25, semantic retrieval, or both contributed, so that I can understand why it surfaced.
17. As an agent, I want every result to report whether its source is synced, local-only, or conflicted, so that I do not mistake local working state for settled shared state.
18. As an agent, I want saved unsynced edits to be searchable, so that retrieval reflects my current Brain Working Tree rather than only server state.
19. As an agent, I want conflicted Markdown to remain searchable and clearly marked, so that useful content is not hidden while I resolve sync state.
20. As a wiki author, I want Markdown headings to define searchable sections, so that ordinary document structure improves retrieval without special chunk markup.
21. As a wiki author, I want files without headings to remain searchable, so that simple notes do not require restructuring.
22. As a wiki author, I want unusually long sections split without losing their file, title, or heading ancestry, so that implementation limits do not erase context.
23. As a wiki author, I want attachments and binary Assets left out of the first index, so that Hybrid Wiki Search stays aligned with the Markdown LLM Wiki model.
24. As a wiki author, I want agents to follow attachment links from retrieved Markdown, so that excluded binaries remain reachable through their authored context.
25. As a user editing locally, I want saved Markdown changes reflected promptly in search, so that results track my active work.
26. As a returning user, I want the Agent Runtime to detect changes made while it was stopped, so that a persistent index cannot silently remain stale.
27. As a syncing user, I want remote additions, edits, and removals reflected in the same index, so that synced knowledge becomes searchable incrementally.
28. As a user with a large wiki, I want only changed sections re-indexed and re-embedded, so that routine maintenance is efficient.
29. As a user, I want a full rebuild only when the index is absent, corrupt, or incompatible, so that ordinary startup does not repeat expensive work.
30. As a privacy-conscious beta participant, I want embedding requests to omit identities, Brain and Folder metadata, paths, keys, grants, and sync metadata, so that the central provider receives only the text needed for embeddings.
31. As a privacy-conscious beta participant, I want submitted sections and search queries neither logged nor retained by Finite Specialization, so that the temporary plaintext trust boundary is narrow.
32. As a privacy-conscious beta participant, I want returned vectors and search metadata to remain on my Agent Runtime, so that derived semantic state is not uploaded through Brain sync.
33. As a Folder controller, I want semantic indexing enabled by default during the internal beta, so that the feature can be evaluated without manual setup for every readable Folder.
34. As a Folder controller, I want to disable semantic indexing for an individual Folder, so that its content no longer goes to the beta Embedding Provider.
35. As a Folder controller, I want disabling semantic indexing to delete that Folder's local vectors while retaining BM25, so that privacy control does not remove basic search.
36. As a Folder controller, I want re-enabling semantic indexing to rebuild vectors in the background, so that search remains available during restoration.
37. As an operator, I want to inspect semantic index status per Folder, so that I can see whether it is disabled, building, ready, stale, or failed.
38. As an operator, I want embedding endpoint and credentials supplied securely at runtime, so that secrets never enter the wiki, index, repository, or synced Brain state.
39. As an organization operating its own stack in the future, I want the provider contract independent of the centralized beta deployment, so that I can host the model beside my FiniteBrain deployment without rewriting search.
40. As a future client developer, I want the same provider contract to support an embedded local model, so that semantic retrieval can eventually avoid plaintext network egress.
41. As a user restarting an Agent Runtime, I want unchanged local indexes to persist, so that startup is fast and sections are not embedded again unnecessarily.
42. As a user whose Folder access is revoked, I want that Folder's complete local index deleted, so that derived search state follows the Folder Access lifecycle.
43. As a user removing a Brain Working Tree, I want its search indexes removed with it, so that orphaned derived plaintext does not remain behind.
44. As an internal beta evaluator, I want a lexical-only diagnostic mode, so that I can compare BM25 with hybrid retrieval using the same wiki and queries.
45. As an internal beta evaluator, I want realistic queries paired with expected relevant sections, so that retrieval quality is measured against actual wiki use.
46. As an internal beta evaluator, I want retrieval quality and latency recorded without an arbitrary first-beta launch threshold, so that early evidence guides later tuning.
47. As a security reviewer, I want the local index treated as disposable derived state rather than a backup or source of truth, so that recoverability claims remain grounded in authoritative Markdown and the Recovery Set.
48. As a security reviewer, I want the beta index kept outside the wiki and protected by the existing private, owner-only Agent Runtime boundary, so that it does not broaden sync or authoring surfaces.
49. As a maintainer, I want provider model identity, version, and vector dimension recorded with derived state, so that incompatible vectors are invalidated rather than mixed.
50. As a maintainer, I want a Folder's first semantic generation activated atomically, so that whichever files embed first do not receive a temporary ranking advantage.
51. As a maintainer, I want unchanged current vectors to remain useful during later incremental edits, so that one changed section does not disable hybrid retrieval for an entire Folder.
52. As a maintainer, I want one end-to-end CLI acceptance seam, so that the complete member-visible retrieval behavior can be verified without coupling tests to internal storage choices.

## Implementation Decisions

- Markdown Pages in the persistent Brain Working Tree remain the sole knowledge authority. Search indexes, vectors, rankings, excerpts, and provider metadata are disposable derived state and are not part of the Recovery Set.
- The first delivery targets `fbrain` and the Agent Runtime over persistent Brain Working Trees. Product Client/browser search integration is outside this feature, although section parsing and ranking concepts should remain reusable.
- Add one normal retrieval interface: `fbrain search <query>`. It searches all Folders currently readable by the acting Member Identity and emits one merged ranking.
- Support repeatable `--folder` filters for explicit narrowing. Never derive Folder scope from the process's current directory. Reject unknown or unreadable requested Folders rather than silently broadening scope.
- Support `--limit` with a default of ten and a hard maximum of fifty, `--json` for a stable structured contract, and `--lexical-only` for internal-beta comparison. Do not expose a semantic-only user mode.
- Search returns Search Evidence rather than an answer. Each result includes rank, Folder identity suitable for navigation, Page path, Page title, heading ancestry, a short excerpt, source disposition (`synced`, `local-only`, or `conflicted`), and contributing retrieval signals (`lexical`, `semantic`, or `both`).
- Index only readable Markdown files materialized in the Brain Working Tree. Exclude hidden FiniteBrain control state, generated files, attachments, and binary Assets. Do not follow symlinks outside the Working Tree boundary.
- Use Markdown Section as the canonical retrieval unit shared by lexical and semantic indexing. A section is the readable content under a heading and retains Folder, Page path, Page title, and full heading ancestry. A file with no headings is one section.
- Split an oversized Markdown Section only when required by configured index or provider limits. Prefer paragraph boundaries, retain all document context, and use a small neighboring overlap. Keep size and overlap tunable rather than making them author-facing content rules.
- Maintain physically separate local indexes per Folder. Candidate generation begins with the currently readable Folder set; cross-Folder merging happens only after candidates have been produced inside those boundaries.
- Persist each Folder index across Agent Runtime restarts in private
  Finite-managed `.finitebrain/` control state outside authored wiki content.
  The beta relies on the same owner-only OS boundary as the plaintext Brain
  Working Tree and does not add a second index-encryption key system. This
  derived state is never materialized or synced as a Page.
- Delete a Folder's complete index when readable Folder Access is lost or its Working Tree is removed. A selected result or navigation state never supplies authority to retain or rewrite durable content.
- Feed one incremental index updater from three triggers: live Working Tree file notifications, startup reconciliation, and FiniteBrain sync materialization/removal events. Compare content fingerprints and update, insert, or delete only affected Markdown Sections.
- Rebuild a complete lexical index only when it is missing, corrupt, or incompatible with the current index/sectioning format. Rebuild semantic state when its model contract is incompatible.
- Update BM25 synchronously enough for changed content to become searchable promptly. Queue embeddings asynchronously for new or changed sections. Embedding failure, delay, rate limiting, or provider outage must never block Working Tree sync or lexical search.
- Use BM25 as the lexical ranking method and vector similarity as the semantic method. Merge their independent ranked lists with rank-based fusion rather than comparing incompatible raw score scales. Preserve signal provenance in each result.
- A Folder remains BM25-only until its first complete semantic generation is ready, then activates that generation atomically. During later incremental changes, omit stale vectors for changed sections while continuing to use current vectors for unchanged sections.
- Treat saved local-only and conflicted Working Tree Markdown as current searchable input. Search must label its disposition and must not imply that retrieval resolves or authorizes conflict handling.
- Define a replaceable Embedding Provider adapter used by indexing and query embedding. Search/indexing modules depend on this adapter interface rather than a particular runtime, model, endpoint, or vendor.
- For the internal beta, implement the adapter with centralized Finite Specialization. Semantic indexing is enabled by default for every newly readable Folder. Beta participants are informed personally; this spec does not add an in-product disclosure or consent surface.
- The adapter owns provider-specific endpoint paths, request serialization, bounded batching, authentication behavior, retries, timeouts, and response validation. Endpoint and credential values are injected securely by the Agent Runtime at startup and are independently revocable from Brain identities and Folder Keys.
- Expose a simple authenticated batch embedding capability from Finite Specialization. Each section input carries only a request-local opaque identifier, Page title, heading ancestry, and section text. Each response maps the opaque identifier to one vector and reports model identity, model version, and vector dimension.
- Query embedding sends only query text. Section and query requests exclude Member Identity, Brain and Folder identifiers or names, filesystem paths, keys, grants, revision data, and sync metadata.
- Finite Specialization must not persist or log submitted section text or query text. Operational logging may contain non-content request metadata such as timing, status, batch size, and model identity, provided it cannot reconstruct submitted knowledge.
- Validate response cardinality, identifiers, numeric values, vector dimensions, and model metadata before accepting vectors. A malformed response fails closed for semantic retrieval and leaves BM25 available.
- Record the provider model identity, model version, vector dimension, sectioning/index format version, and content fingerprint with local semantic state. Never mix vectors from incompatible generations.
- Returned vectors and associated semantic-index metadata remain client-side. Do not upload them through FiniteBrain sync or place them inside Markdown Pages.
- Add `fbrain search-index status`, `fbrain search-index enable --folder <folder>`, and `fbrain search-index disable --folder <folder>` for the internal beta. `status` reports each readable Folder's selection and lifecycle state without exposing credentials or submitted plaintext.
- Disabling semantic indexing stops new requests and deletes the selected Folder's local vectors while preserving BM25. Re-enabling it schedules a new background semantic generation.
- Provider unavailability, missing configuration, disabled semantic indexing, incomplete initial generation, invalid responses, and semantic rebuilds all degrade to lexical results through the same `fbrain search` command. Human-readable and JSON output must make the contributing signals observable without turning normal fallback into a fatal error.
- Keep plaintext body handling within the existing private Working Tree and Agent Runtime boundary except for the explicitly trusted beta Embedding Provider calls described here. Do not move decryption or embedding into the core plaintext-blind FiniteBrain server as part of this work.

## Testing Decisions

- The primary acceptance boundary is the real `fbrain search --json` CLI operating over a synthetic Brain Working Tree containing multiple readable Folders, representative Markdown structure, local-only changes, conflicts, and removals, with a fake Embedding Provider that returns deterministic vectors.
- Prefer this one high seam over parallel subsystem acceptance harnesses. Tests assert external command behavior, lifecycle effects, provider request contracts, and stable JSON rather than the concrete index library or on-disk representation.
- The primary seam must prove default cross-Folder search, repeatable Folder narrowing, unreadable/unknown Folder rejection, top-ten defaults, the fifty-result cap, and independence from the current subdirectory.
- It must prove heading-based Section results, heading ancestry, no-heading files, paragraph-aware oversized-section behavior, Markdown-only inclusion, and exclusion of control state and Assets.
- It must prove exact BM25 retrieval, deterministic semantic retrieval, rank-based fusion, signal provenance, and the stronger combined rank of evidence found by both methods.
- It must prove readable terminal behavior separately from the stable JSON contract, including Folder, path, heading, excerpt, sync disposition, and retrieval signals.
- It must prove BM25-only behavior when semantic indexing is disabled, unconfigured, initially building, stale, malformed, rate-limited, timed out, or unavailable.
- It must prove that an initial semantic generation is invisible until complete, becomes active atomically, and later updates preserve current vectors for unchanged Sections while withholding stale vectors.
- It must prove incremental maintenance from live local changes, startup reconciliation after offline changes, and sync-driven additions, edits, and removals without requiring a full rebuild.
- It must prove index persistence across restart, safe rebuild after missing/corrupt/incompatible state, vector invalidation after model or dimension changes, and complete deletion after Folder revocation or Working Tree removal.
- It must prove that semantic disablement removes vectors, preserves BM25, stops provider calls for that Folder, and that re-enablement schedules a background rebuild.
- It must inspect fake-provider requests to prove bounded batching and the approved minimum payload. Tests must fail if identity, Brain/Folder metadata, filesystem paths, keys, grants, revision information, sync metadata, or credentials enter content payloads.
- It must prove that the provider adapter receives endpoint and credential configuration at runtime, applies authentication itself, and never writes credentials into indexes, Working Trees, JSON output, or logs.
- It must prove response validation for missing, duplicate, unknown, non-finite, wrong-dimension, and mixed-model vectors, with BM25 remaining available after rejection.
- Focused unit tests are appropriate beneath the acceptance seam for deterministic Markdown Section parsing, content fingerprinting, BM25 scoring, vector similarity, rank-based fusion, and index-generation validation. These tests support the CLI seam and do not replace it.
- Existing `fbrain` command tests, synthetic Working Tree fixtures, fake HTTP server patterns, and JSON report assertions are the repository prior art. Extend those seams rather than introducing a second CLI or integration framework.
- Add a small internal retrieval evaluation fixture containing realistic wiki questions and expected relevant Sections. Run both normal hybrid search and `--lexical-only`, recording retrieval quality and latency. The internal beta establishes a baseline; it does not require an arbitrary quality threshold for release.
- Run formatting, Clippy with warnings denied, the focused CLI/index/provider suites, the full locked Rust workspace tests against their required services, and the repository's normal smoke/static checks before completion.

## Out of Scope

- Generating answers, summaries, synthesized citations, or durable knowledge from search results.
- Replacing Markdown Pages, headings, folders, wiki links, or normal agent file reading with a RAG database.
- Indexing attachments, PDFs, images, audio, arbitrary office documents, external websites, Slack, email, or other connectors.
- Product Client/browser integration or changing its existing in-memory Page search in this first delivery.
- Server-side storage or synchronization of plaintext indexes, vectors, excerpts, or semantic metadata.
- Making the core FiniteBrain server decrypt Folder content or embedding content inside its current plaintext-blind role.
- A semantic-only end-user search mode, query rewriting, reranking models, generated contextual summaries, graph ranking, expertise ranking, or multi-hop answer orchestration.
- Production-quality in-product consent, disclosure, provider selection, retention-policy UI, or customer self-service controls.
- Shipping an embedding model inside every Agent Runtime during this beta.
- Requiring an organization-operated embedding service during this beta.
- Adding a second encryption/key-management layer around local indexes in this beta.
- Treating search indexes or embeddings as backups, authoritative knowledge, sync state, or part of a Recovery Set.
- Defining a permanent launch-quality threshold before internal-beta measurements exist.
- Production rollout beyond the explicitly managed internal beta.

## Further Notes

- This specification follows the accepted Hybrid Wiki Search, Markdown Section,
  Embedding Provider, and Search Evidence vocabulary and the associated
  Folder-boundary, current-Working-Tree, asynchronous-indexing, and
  evidence-not-answer architecture decisions.
- The centralized Finite Specialization provider is a temporary internal-beta
  deployment choice. It must not silently become the production default.
- When this spec is broken into implementation tickets, create explicit
  placeholder follow-up tickets for: client-local embeddings; an
  organization-operated provider deployment; production provider selection,
  disclosure, and consent; local index encryption-at-rest and production
  security-model alignment; and any separately proposed core-server plaintext
  trust-boundary change.
- The provider's no-retention/no-content-logging behavior is part of the beta
  security contract and must be verified in implementation and deployment
  review, not assumed from the adapter alone.
- Folder Access remains the authorization boundary. Search scope begins with
  currently readable Folders; post-search filtering is not an acceptable
  substitute.
- Search Evidence is navigation state, not authority to select, overwrite,
  merge, resolve, or delete durable user content.
- User data availability remains the first security invariant. Search-derived
  state may always be deleted and rebuilt; authoritative Markdown and Recovery
  Sets must remain independently restorable.
