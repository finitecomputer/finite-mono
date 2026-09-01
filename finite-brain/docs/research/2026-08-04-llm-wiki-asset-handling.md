# LLM Wiki asset handling and FiniteBrain alignment

Date: 2026-08-04

## Bottom line

FiniteBrain's decision to keep bulk Asset bytes out of Folder Objects is
consistent with the canonical LLM Wiki pattern at the level that matters: raw
sources remain the authority, while the durable knowledge surface is
LLM-maintained Markdown derived from those sources. It is not a literal copy of
the pattern's optional local-file workflow. It is a distributed,
access-controlled adaptation of it.

The canonical source is Andrej Karpathy's **LLM Wiki** idea file. It is a
pattern, not a formal interoperability or storage specification: Karpathy says
that concrete directories, schemas, page formats, and tooling are deliberately
left to each implementation. [Canonical LLM Wiki idea file, pinned
revision](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f/ac46de1ad27f92b28ac95459c782c07f6b8c964a#file-llm-wiki-md-L73-L75)

## What upstream actually says about assets

The pattern separates three layers:

1. **Raw sources** are curated documents including articles, papers, images,
   and data files. They are immutable, read-only to the LLM, and explicitly the
   source of truth.
2. **The wiki** is LLM-generated Markdown: summaries, entity and concept pages,
   comparisons, overviews, and synthesis.
3. **The schema** tells the agent how to maintain the wiki.

[Canonical three-layer architecture](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f/ac46de1ad27f92b28ac95459c782c07f6b8c964a#file-llm-wiki-md-L25-L33)

On ingest, the agent reads a raw source, writes a summary page, updates related
Markdown pages, and records the work. Queries operate over the maintained wiki
and produce cited synthesis. Thus the upstream pattern does not make a PDF,
image, or dataset into the primary wiki article; it compiles that evidence into
Markdown while retaining the raw authority separately. [Canonical ingest and
query operations](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f/ac46de1ad27f92b28ac95459c782c07f6b8c964a#file-llm-wiki-md-L35-L41)

For images specifically, Karpathy suggests an **optional** Obsidian workflow:
download linked images into a fixed local attachment directory such as
`raw/assets/`. The reason is practical—the agent can inspect local images and
does not depend on URLs that may break. The agent still views images separately
from the Markdown text. [Canonical image-handling
tip](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f/ac46de1ad27f92b28ac95459c782c07f6b8c964a#file-llm-wiki-md-L55-L62)

The idea file does **not** define:

- a required binary storage backend;
- a first-class attachment or pointer schema;
- content hashes, revisions, custody, or availability states;
- remote authorization semantics for Google Drive or another source system;
- the meaning of a machine-local path in a shared wiki; or
- whether an implementation must copy every raw source into the wiki's own
  storage.

Those omissions are intentional because the upstream document leaves exact
implementation choices to the user and agent. [Canonical implementation-scope
note](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f/ac46de1ad27f92b28ac95459c782c07f6b8c964a#file-llm-wiki-md-L73-L75)

## Comparison with the settled FiniteBrain direction

| Concern | Canonical LLM Wiki | FiniteBrain direction |
| --- | --- | --- |
| Knowledge surface | Interlinked, LLM-maintained Markdown | Markdown Pages and Source Notes |
| Original evidence | Immutable raw sources are authoritative | Asset bytes remain authoritative under declared Asset Custody |
| Binary placement | Unspecified; local `raw/assets/` is an optional image tactic | Never inside Folder Objects; an Asset Record locates the bytes |
| Source handle | Ingest creates a Markdown summary and cited wiki updates | A Source Note is the human/agent-readable handle paired with the Asset Record |
| Distributed access | Not specified | Asset Record distinguishes external, runtime-local, and future Brain-managed custody; Folder Access alone does not grant byte access |
| Integrity and availability | Not specified | Asset Record carries provenance, integrity facts, location, and availability |

FiniteBrain therefore preserves the upstream **epistemic boundary** while
changing the **physical storage boundary**. This is the right adaptation for a
shared encrypted Brain. The accepted decision is recorded in [ADR
0044](../adr/0044-keep-bulk-asset-bytes-out-of-folder-objects.md), and the
settled domain distinction between Asset, Asset Record, Asset Custody, Source
Note, and Asset Source Note Pair is captured in
[`CONTEXT.md`](../../CONTEXT.md#asset-record).

The pairing is stronger than a bare Markdown link:

- The **Asset Record** accounts for the original evidence and states where its
  bytes live, who or what governs them, and whether this client can currently
  retrieve them.
- The **Source Note** supplies the provenance, extraction status, summary, and
  citations that humans, agents, search, and graph flows can read.
- Synthesized wiki Pages cite the Source Note, keeping opaque or unavailable
  blobs out of the primary reasoning surface.

That extends the Source Note decision captured in [ADR
0044](../adr/0044-keep-bulk-asset-bytes-out-of-folder-objects.md), which keeps
bulk asset bytes out of Folder Objects.

## Design implication

The proposal should be described as **LLM Wiki-aligned, not LLM Wiki-required**.
Upstream favors local copies when they improve durability and inspectability,
but it does not require them. FiniteBrain instead makes the raw-source
relationship explicit enough to work across machines and authorities.

To preserve the upstream promise that raw sources are immutable authority, an
Asset Record must identify a particular source version or content identity—not
merely contain a mutable URL or path. A Google Drive link may name the external
authority, and a runtime-local path may account for a local source, but neither
should be presented as shared, available, or unchanged without corresponding
availability and integrity evidence. Future Brain-managed blob storage can
provide a more durable custody option behind the same record without changing
the Markdown-first model.
