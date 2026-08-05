# OKF Asset Portability Research

Date: 2026-08-04

Decision note: ADR 0044 subsequently adopted the narrower initial profile.
Asset References require only `type`, `title`, and one canonical `resource`;
`description` and known integrity facts are optional. Custody, stored
availability, mirrors, caches, and a blob-provider abstraction are deferred.
Asset Reference authoring is agent-only in the initial product surface. Agents
place the Markdown notes under `raw/` by convention; the new profile has no
Brain-resident `raw/assets/` blob directory. OKF is a design compass rather
than a present conformance claim: the profile should stay close to upstream and
avoid needless incompatibility, but full exporter conformance is not scheduled
without a concrete interoperability need. This subsequent decision supersedes
the broader exporter recommendation later in this research note.

## Conclusion

FiniteBrain's current `finite-okf-brain-export-v1` is a Finite-defined,
OKF-style readable export, not a conformant Open Knowledge Format bundle.
That does not block the proposed Asset Source Note design. In fact, an Asset
represented by one Markdown concept with YAML frontmatter and a canonical
resource pointer is closer to upstream OKF than the current attachment bundle.

The narrow recommendation is to make the next hard-cut readable export an
explicit **OKF v0.2 profile**: emit conformant Markdown concepts, keep
Finite-specific Asset metadata in extension frontmatter, and treat bulk bytes
as referenced resources rather than part of OKF's portable knowledge contract.

## What upstream OKF is

Google Cloud introduced Open Knowledge Format on 2026-06-12 as a portable
formalization of the LLM-wiki pattern. The current upstream specification is
v0.2. It describes a directory of Markdown concept documents with YAML
frontmatter; it deliberately does not prescribe storage, serving, or query
infrastructure.

Sources:

- [Google Cloud announcement](https://cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing)
- [Canonical OKF v0.2 specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
- [Original v0.1 specification at its initial commit](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/ee67a5ca27044ebe7c38385f5b6cffc2305a9c1a/okf/SPEC.md)

Upstream v0.2 conformance is intentionally small:

1. Every non-reserved `.md` file has parseable YAML frontmatter.
2. Every such frontmatter has a non-empty `type`.
3. Reserved `index.md` and `log.md` files follow their specified structures
   when present.

Unknown `type` values and producer-defined frontmatter keys must be tolerated.
A root `index.md` may declare `okf_version: "0.2"`, but that declaration is
optional. The same frontmatter and required-`type` foundation existed in v0.1.

## How OKF handles assets and external material

OKF's `resource` field is the canonical URI for the underlying asset described
by a concept. Its `sources` field records external or bundle-local material
from which a concept derives. Path-valued fields may point to an absolute URL,
a bundle-root-relative path, or a relative path. A conventional `references/`
directory may mirror external material or code, but this is only a naming
convention.

Upstream does **not** currently define:

- a binary Asset or attachment object model;
- a bundle manifest for attachments;
- MIME type, size, checksum, or content-addressing fields for asset bytes;
- authoritative versus access/mirror locators;
- custody, authentication, download, caching, or availability semantics;
- import behavior for referenced or bundled binary files.

The spec does not explicitly forbid auxiliary non-Markdown files: its examples
point at bundle-local `.py` and `.sql` files, and its conformance rules inspect
Markdown files. Such files are therefore possible producer extensions, but
their attachment and round-trip meaning is not interoperable OKF behavior.

Two open upstream discussions confirm that this remains unsettled rather than
normative: [issue #237 asks how diagrams and images should be supported](https://github.com/GoogleCloudPlatform/knowledge-catalog/issues/237),
and [issue #175 proposes an external-bundle registry with refs, checksums,
mirrors, and caches](https://github.com/GoogleCloudPlatform/knowledge-catalog/issues/175).
Neither proposal is part of v0.2.

OKF v0.2 adds useful trust and provenance signals, but they are not byte
integrity. `verified` records who or what confirmed a concept against its
resource or sources; `sources[].last_modified` is a recency signal. Neither
pins the bytes at a locator. A provider revision or content hash remains a
necessary Finite extension when immutable evidence matters.

## Why Finite's current export is not conformant

Finite's portability spec and implementation define their own bundle:

- `okf-brain.json` with version `finite-okf-brain-export-v1`;
- `content/` Markdown Pages;
- `attachments/` raw files;
- `_wiki/` generated reports;
- a Finite manifest with Folder/Object ids and SHA-256 `contentHash` values.

Those are Finite extensions, not upstream OKF structures. Extensions alone are
not the problem. The decisive conformance failure is Markdown shape:

- the exporter passes Page Markdown through without requiring or adding YAML
  frontmatter and `type`;
- generated `_wiki/backlinks.md`, `_wiki/orphans.md`, `_wiki/stale.md`, and
  `_wiki/tags.md` are non-reserved concept filenames with no YAML frontmatter
  or `type`.

Therefore every current generated bundle containing those reports violates the
minimum upstream rules. The repository itself accurately calls the format
"OKF-style" in its seeded documentation; it does not establish a standards
compliance profile. Relevant local sources are
[`finitebrain-portability-spec.md`](../specs/finitebrain-portability-spec.md),
[`portability/okf.rs`](../../crates/finite-brain-core/src/portability/okf.rs),
and [`seed-smoke-doc-pages.mjs`](../../scripts/seed-smoke-doc-pages.mjs).

## Alignment of the proposed Asset Source Note

The proposed one-note representation aligns cleanly with OKF when its
frontmatter exposes the upstream interoperability surface and nests Finite's
stronger contract as extensions:

```yaml
---
type: Asset Reference
title: Quarterly board packet
description: Canonical board packet maintained in Google Drive.
resource: https://docs.google.com/document/d/...
sources:
  - id: board-packet
    resource: https://docs.google.com/document/d/...
    title: Quarterly board packet
finite_asset:
  custody: external-authoritative
  authoritative_locator:
    provider: google-drive
    resource: https://docs.google.com/document/d/...
    revision: "..."
  integrity:
    sha256: "..."
  access_locators: []
  availability: retrievable
  content_type: application/pdf
  size: 123456
---
```

The exact Finite extension key is a later schema decision. The important
mapping is:

- top-level `type` makes the Source Note an OKF concept;
- top-level `resource` carries the canonical URI when one is portable;
- upstream `sources` carries provenance for claims derived from the Asset;
- Finite extension fields carry custody, locator authority, byte integrity,
  access alternatives, and availability because upstream OKF does not.

A machine-local absolute path should not be presented as portable OKF
`resource`: upstream interprets leading `/` as bundle-root-relative, and the
path has no meaning on another machine. Keep that locator in the Finite
extension and mark its custody/availability as runtime-local.

## Recommended compatibility boundary

For a hard cut, replace rather than migrate `finite-okf-brain-export-v1`:

1. Define a versioned Finite **profile of OKF v0.2**, not a separate format
   merely carrying OKF in its name.
2. Emit every Brain Page and Asset Source Note as a conformant concept with a
   non-empty `type`; emit conformant reserved/index documents or omit generated
   reports that cannot meet the rules.
3. Preserve unknown frontmatter on import and round-trip the Finite Asset
   extension losslessly.
4. Make the default export reference-preserving: export the Asset Source Note,
   not the bulk bytes.
5. Keep any future "materialized snapshot" mode explicitly Finite-specific.
   If it fetches bytes into `references/assets/`, record their bundle path and
   snapshot hash separately from the authoritative `resource`; imported bytes
   must become runtime-local or be promoted to a future managed blob provider,
   never silently become inline Folder Objects.
6. Distinguish the manifest hash of the exported Markdown file from the Asset
   Record's provider revision or content hash. They prove different things.
7. Add conformance tests for the three upstream rules and pin the targeted OKF
   version in the portability specification.

This keeps Finite genuinely OKF-compatible at the knowledge-document layer
without pretending that OKF solves blob custody or integrity. Finite's Asset
Record is the necessary stronger layer, not a competing knowledge format.
