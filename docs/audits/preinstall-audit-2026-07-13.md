# Agent Runtime Preinstall Audit

Status: PROPOSED

Date: 2026-07-13

Scope: tools that bundled `finite-skills` explicitly ask an Agent to install at
runtime. This is a source-and-image inventory, not a proposal to add every tool
to the base image.

## Finding

The highest-confidence mismatch was `gws`: the Google Workspace skill says to
prefer it, the legacy runtime pinned version 0.22.5, and the mono runtime did
not contain it. The overnight branch adds the official 0.22.5 GNU binaries for
amd64 and arm64, verifies their published SHA-256 digests, and runs
`gws --version` while building the image.

The pre-change runtime inventory found Pillow present, while `gws`,
`fal-client`, `ddgs`, `fpdf2`, ReportLab, PyMuPDF, and python-docx were absent.
`blogwatcher` and `parallel` were also absent.

## Other evidenced candidates

| Candidate | Evidence in bundled skills | Recommendation |
| --- | --- | --- |
| `fal-client` | Both FAL image editing and music generation include an install step. | Next strongest bake candidate, but defer: this run permits no dependency other than `gws` and the Telegram messaging extra. Pin and image-test it in a separate change. |
| PDF/document Python libraries | `generate-pdf`, `nano-pdf`, OCR, and Office references install `fpdf2`, ReportLab, PyMuPDF/PyMuPDF4LLM, marker-pdf, pdfminer.six, or python-docx. | Do not bake this whole set without workload frequency and size measurements. The tools overlap only partially and some are large. Prefer a pinned task environment or a deliberately scoped document-tool layer. |
| `ddgs` | The DuckDuckGo skill installs the Python package when missing. | Low-cost candidate, but only one skill is evidence. Pin and test separately. |
| `blogwatcher` | Its skill uses `go install github.com/Hyaxia/blogwatcher/cmd/blogwatcher@latest`. | Do not bake `@latest`. Select and review a release, license, checksum, and architecture support first. |
| `parallel` CLI | Its skill offers mutable Homebrew, npm, pip, and curl installation paths. | Do not choose an implementation implicitly. Reconcile the skill to one supported, pinned tool before considering the runtime image. |
| Pillow | Meme and image skills conditionally install it. | Already supplied transitively by the pinned Hermes environment; update skill language where useful instead of adding another copy. |
| Playwright/site dependencies | Website references contain project-local npm install instructions. | Keep project-local. They are not evidence for a universal Agent runtime dependency. |

## Decision rule

A future preinstall should have repeated transcript or multi-skill evidence,
an exact version and integrity source, architecture coverage, a measurable
reduction in task setup time, and acceptable image-size/security impact.
Mutable `latest` installers are documentation debt, not image specifications.
