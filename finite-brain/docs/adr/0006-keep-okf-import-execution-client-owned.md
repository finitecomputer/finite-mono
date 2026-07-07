# ADR 0006: Keep OKF Import Execution Client-Owned

Status: accepted

FiniteBrain Rust v1 will execute readable OKF imports in the Product Client,
not in a server plaintext import endpoint.

The Product Client parses OKF bundles, plans conflicts, opens destination
Folder Keys, encrypts imported Pages as Folder Objects, signs revisions, and
uploads those revisions through the normal secure object routes. The server
continues to validate metadata, signed events, sync rules, Folder Access, and
encrypted object envelopes, but it does not parse readable Markdown or receive
plaintext Page content during import.

## Considered Options

- Add a server import endpoint that accepts readable OKF and performs
  encryption/upload work.
- Keep import execution in the trusted Product Client and reuse existing secure
  object routes.

## Consequences

- OKF import follows the same plaintext boundary as search, graph, replay, and
  OKF export.
- Server tests should continue to reject plaintext search/import behavior and
  focus on encrypted object validation.
- Product Client tests own import conflict planning, inaccessible Folder
  rejection, encryption preparation, and signed revision submission.
