# ADR 0044: Keep Bulk Asset Bytes Out Of Folder Objects

Status: accepted

Date: 2026-08-04

FiniteBrain keeps bulk non-Markdown Asset bytes outside Folder Objects. A
Folder instead contains one Markdown Asset Source Note. Its OKF-compatible
frontmatter has a `type`, `title`, and one canonical `resource`; `description`
is optional. The `resource` may be an external URI or a machine-local file URI.
The note's Markdown body is the human- and agent-readable LLM Wiki surface;
there is no second Brain object that can drift from it.

An Asset Source Note may carry a small `finite_asset` extension containing
facts already known to the writer, such as `content_type`, `size`,
`content_hash`, or `provider_revision`. Those fields are optional. FiniteBrain
does not require a custody model, stored availability state, mirrors, caches,
or a blob-provider abstraction in this version.

The initial shape stays deliberately close to Open Knowledge Format by using
its Markdown concept model and `type`, `title`, `resource`, and `description`
vocabulary. This is an alignment choice, not a claim that FiniteBrain's current
readable export conforms to a particular OKF version. Full OKF export conformance is not claimed.

## Initial Product Scope

Asset Reference authoring is agent-only in the first release. Agents learn the
Markdown convention through the FiniteBrain skill and the nearest `AGENTS.md`,
then create Asset Source Notes through the existing Page-writing flow. This is
a product-surface boundary, not a new authorization role: the Agent still acts
through an authorized Member Identity, and humans with Folder Access can read
the resulting note.

Agents conventionally place Asset Source Notes under the Folder's `raw/` tree.
The new working-tree profile does not create or reserve `raw/assets/`, because
no Asset bytes are materialized there. This is agent authoring guidance, not a
special storage rule or path validator.

## Consequences

- Folder Access governs the Asset Source Note but does not by itself grant
  access to the bytes named by its `resource`.
- Whether a client can retrieve a `resource` is runtime state, not durable
  Brain metadata.
- A bare `resource` is a useful pointer but not proof of immutable evidence.
  Clients may make that claim only when an optional provider revision or
  content hash identifies the bytes.
- Sync, replay, export, and server backup remain bounded by small encrypted
  records rather than bulk binary content.
- Readable OKF export preserves the Asset Source Note and its reference by
  default; it does not copy the Asset bytes into the bundle.
