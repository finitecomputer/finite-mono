# ADR 0005: Derive Graph And Replay From Client-Decrypted Indexes

Status: accepted

FiniteBrain Graph View and Graph Replay will be derived from the Product
Client's local decrypted Page index, not from a server-side graph model. The
server may continue to expose encrypted sync records and server-visible object
metadata, but Page titles, Page paths, links, backlinks, tags, graph nodes,
graph edges, and replay frames are client-side products of content the active
User can decrypt.

## Consequences

- Graph and replay visibility follows the same Folder Key and Folder Access
  boundary as search, OKF export, and agent reports.
- The server does not need graph-specific schema or plaintext graph indexing to
  support the first Product Client.
- Replay is a projection over the client's applied sync/decrypted Page history,
  not a second authoritative event model.
