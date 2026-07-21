# Nomic Embedding Specialization

Status: active verified specialization
Verified: 2026-07-21

Finite exposes semantic embeddings through the authenticated Spark Front Door:

- model alias: `nomic-embed-text-v1-5`
- endpoint: `POST /v1/embeddings`
- output: 768-dimensional L2-normalized float vectors
- measured per-item context: 2,048 tokens
- accepted input: one string or an array of strings

Every input must start with the Nomic task prefix that matches its role:
`search_query:`, `search_document:`, `clustering:`, or `classification:`.
The memory or retrieval adapter owns that choice. It must not ask Front Door to
guess whether text is a query or document.

The 2026-07-21 activation evidence proved invalid-key rejection, authenticated
model discovery, semantic ordering, an embeddings-only endpoint boundary,
temporary-key cleanup, protected Nemotron specialization continuity, and
PersonaPlex health with zero restarts under concurrent embedding load.

This capability is service-first. It is not a chat model, MoA reference model,
vector database, or implicit memory implementation. A consumer still owns
chunking, indexing, storage, retrieval policy, and deletion semantics.
