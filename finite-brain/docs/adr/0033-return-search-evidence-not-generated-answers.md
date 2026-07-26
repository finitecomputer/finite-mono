# Return Search Evidence Not Generated Answers

Status: accepted

`fbrain search` will return ranked Markdown Section evidence and locations in
the original Brain Working Tree rather than generating an answer or durable
summary. Each result identifies its Folder, Page path, heading, excerpt,
local-sync disposition, and whether lexical, semantic, or both retrieval
signals contributed; the CLI provides both readable terminal output and a
stable structured form for agents.

The first version merges the independent BM25 and semantic result rankings
using rank-based fusion rather than comparing their incompatible raw scores.
Sections retrieved by both signals receive stronger combined rank, and the
result preserves which signals contributed.

Search returns the top ten sections by default. Callers may request a larger
set with `--limit`, capped at fifty results so one retrieval does not flood an
agent's context or terminal output.

## Consequences

- Agents continue to open original Pages, follow wiki links, and reason through
  their normal context window.
- Hybrid Wiki Search remains a read accelerator rather than another knowledge
  authoring or synthesis system.
- Search quality can be evaluated independently from model answer quality.
- Ranking remains stable across embedding models whose raw similarity score
  ranges differ.
- Internal-beta evaluation uses a small set of realistic wiki queries with
  expected relevant sections, comparing normal hybrid retrieval with
  `--lexical-only` and recording retrieval quality and latency. The first beta
  gathers a baseline rather than enforcing an arbitrary launch score.
- Generated answers, citations assembled across results, and saved synthesis
  are outside this search capability.
