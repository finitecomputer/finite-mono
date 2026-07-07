---
name: arxiv-finite
description: Search arXiv and inspect paper metadata, BibTeX, citations, references, related work, and author profiles using bundled helpers for arXiv and Semantic Scholar.
version: 1.1.0
author: Hermes Agent
license: MIT
metadata:
  hermes:
    tags: [Research, Arxiv, Papers, Academic, Science, API]
    related_skills: [ocr-and-documents-finite]
---

# arXiv Research

Use the helper script instead of piping XML or JSON into ad hoc Python one-liners.

Script path:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py --help
```

## Workflow

1. `search` or `get` to identify papers
2. `semantic-paper`, `citations`, `references`, or `recommend` to judge impact and related work
3. `bibtex` when the user needs a citation
4. `web_extract` on the abstract or PDF URL when the user needs full content

## Commands

Search arXiv:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py search \
  --query "GRPO reinforcement learning" \
  --max-results 5 \
  --sort date
```

Search by author or category:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py search \
  --author "Yann LeCun" \
  --max-results 5

python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py search \
  --category cs.AI \
  --sort date \
  --max-results 10
```

Get one or more specific papers:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py get \
  --id-list "2402.03300,2401.12345"
```

Generate BibTeX:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py bibtex \
  --id 1706.03762
```

Semantic Scholar details:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py semantic-paper \
  --id 2402.03300

python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py citations \
  --id 2402.03300 \
  --limit 10

python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py references \
  --id 2402.03300 \
  --limit 10

python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py recommend \
  --id 2402.03300 \
  --limit 5
```

Semantic Scholar search:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py semantic-search \
  --query "GRPO reinforcement learning" \
  --limit 5

python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py author-search \
  --query "Yann LeCun" \
  --limit 5
```

Machine-readable output:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/arxiv-finite/scripts/search_arxiv.py search \
  --query "chain of thought" \
  --json
```

## Reading content

Once you have an arXiv ID, use:

```text
https://arxiv.org/abs/ID
https://arxiv.org/pdf/ID
```

Then use `web_extract` or the `ocr-and-documents-finite` skill for the actual paper content.

## Notes

- arXiv is XML; the helper script removes the need for fragile `curl | python -c` parsing.
- Semantic Scholar is best for citations, references, author profiles, and related work.
- Preserve version suffixes in citations when the exact paper version matters.
