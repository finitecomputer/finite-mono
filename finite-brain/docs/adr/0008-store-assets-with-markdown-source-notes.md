# ADR 0008: Store Assets With Markdown Source Notes

Status: accepted

Date: 2026-07-03

## Context

FiniteBrain is a Folder-scoped LLM Wiki, but users and agents need to preserve
non-Markdown source material such as PDFs, images, audio, and generated files.
The existing encrypted Folder Object model can store opaque plaintext, while
the current Product Client and Agent CLI primarily reason over Markdown Pages.

## Decision

FiniteBrain will model non-Markdown source material as encrypted Assets paired
with Markdown Source Notes. The Asset preserves the original evidence under the
same Folder access boundary, and the Source Note is the human/agent-readable
handle containing provenance, extraction status, summaries, and links into the
compiled wiki.

The LLM Wiki knowledge surface remains Markdown-first: agents and local indexes
reason from Source Notes and synthesized wiki Pages, not directly from blob
bytes. Large-object chunking or external encrypted blob backends may be added
later behind the same Asset concept.

## Considered Options

- Store every source as Markdown only, losing original binary evidence.
- Treat blobs as a separate storage system outside Folder Objects.
- Store Assets as Folder Objects and require Markdown Source Notes for LLM Wiki
  use.

## Consequences

- Server routes can remain mostly plaintext-blind because the encrypted object
  envelope stays the sync unit.
- The Product Client and Agent CLI must stop assuming every decrypted Folder
  Object is a Markdown Page.
- Agent instructions and sync validation should enforce that non-Markdown files
  live under `raw/assets/` and have matching Source Notes.
- Search, graph, OKF, and LLM workflows should index Source Notes and extracted
  Markdown, not opaque Asset bytes.
