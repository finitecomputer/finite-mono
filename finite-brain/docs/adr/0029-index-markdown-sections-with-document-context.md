# Index Markdown Sections With Document Context

Status: accepted

Hybrid Wiki Search will use the Markdown Section as its canonical retrieval
unit rather than whole files or anonymous fixed-size chunks. Every indexed
section retains its Folder, Page path, Page title, and heading ancestry; a
section too large for the configured bound may be split internally only when
each split preserves that document context. Oversized sections are split at
paragraph boundaries where possible, with a small neighboring overlap. The
bound and overlap are tunable implementation settings governed by index and
provider limits, not permanent author-facing content rules.

## Consequences

- The first version indexes only readable Markdown files materialized in the
  Brain Working Tree. Attachments, binary Assets, generated control files, and
  hidden FiniteBrain state are excluded.
- Agents may still discover attachments by following links from retrieved
  Markdown Pages.
- Search results identify an original Page and heading for agent navigation.
- Files without headings remain searchable as one section.
- Section parsing is shared by lexical and semantic indexing so both signals
  rank the same knowledge units.
- Index format changes must account for the sectioning contract and trigger a
  rebuild when that contract changes.
