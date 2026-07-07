---
name: perplexity-research-finite
description: Grounded web research using Perplexity Search for raw ranked sources, Sonar Pro for cited briefs, and Firecrawl for exact source fetches.
---

# Perplexity Research

Use this skill for current-events, policy, legal-adjacent, or factual research where grounded sources matter.

## Required credentials

- `PERPLEXITY_API_KEY` for raw search and Sonar Pro briefs
- `FIRECRAWL_API_KEY` for exact-page fetches

## Workflow

Prefer this order:

1. `search` to discover sources
2. `fetch` to read one or more chosen URLs exactly
3. `brief` when the user wants a cited synthesis

Do not treat the brief as the only truth. Show the raw source URLs and inspect key sources when the stakes are high.

## Commands

Raw search results as JSON:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/perplexity-research-finite/scripts/perplexity_research.py search \
  --query "latest SEC crypto enforcement actions" \
  --domain sec.gov \
  --domain law360.com \
  --recency year \
  --json
```

Preferred quick human-readable search output:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/perplexity-research-finite/scripts/perplexity_research.py search \
  --query "OpenAI valuation 2026 funding round" \
  --recency month \
  --max-results 6
```

Cited brief with URLs:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/perplexity-research-finite/scripts/perplexity_research.py brief \
  --query "What changed in recent SEC crypto enforcement this year?" \
  --domain sec.gov \
  --recency year
```

Fetch the exact source text for a chosen URL:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/perplexity-research-finite/scripts/perplexity_research.py fetch \
  --url "https://www.sec.gov/news/press-release/example"
```

## Guidance

- Use `search` first when the user wants inspectable sources.
- Prefer the script's normal markdown output plus `--max-results N` for quick inspection; avoid piping JSON directly into `python -c` because Hermes may treat that as an unsafe interpreter pipe and ask for approval.
- Use `brief` when the user wants a fast cited overview after source discovery.
- Use `fetch` before quoting or relying on a source heavily.
- Prefer domain filters for regulators, courts, or trusted publications.
- Prefer recency filters for evolving topics like regulation, litigation, and news.
- Use `--search-mode academic` for journal-heavy or research-heavy queries.

## Notes

- `search` and `brief` are Perplexity-backed.
- `fetch` is Firecrawl-backed.
- The helper script uses direct HTTP calls and standard library Python only.
