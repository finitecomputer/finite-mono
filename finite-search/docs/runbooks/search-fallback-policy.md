# Search Fallback Policy

Status: proposed

This runbook documents the first fallback shape for Hermes / Finite
`web_search` when self-hosted SearXNG is reachable but produces weak results.

## Goal

Keep SearXNG as the primary self-hosted search path while allowing agents to
fall back to a managed search provider when SearXNG is blocked, thin, slow, or
obviously low quality.

## Provider Fit

Perplexity is a good first fallback candidate if Finite agents already have it
available, but use the right Perplexity surface:

- Use Perplexity Search API for `web_search` fallback. It returns raw ranked
  results with fields such as `title`, `url`, `snippet`, `date`, and
  `last_updated`.
- Do not use Sonar as the default `web_search` fallback. Sonar returns a
  grounded prose answer with citations, which is useful for research briefs but
  is not a drop-in replacement for URL discovery.

References:

- https://docs.perplexity.ai/docs/search/quickstart
- https://docs.perplexity.ai/api-reference/search-post
- https://docs.perplexity.ai/docs/sonar/quickstart

## Initial Decision Rule

Run SearXNG first. Fall back to Perplexity Search when any of these happen:

- SearXNG request fails, times out, or returns invalid JSON.
- SearXNG returns zero results.
- SearXNG returns fewer than three results for a broad query.
- SearXNG reports three or more unresponsive engines.
- The query is domain-constrained and the top results do not match the requested
  domain. Example: `site:github.com firecrawl firecrawl` should not be treated
  as strong if the top results are mirrors or unrelated domains.
- Result shape is unusable for Hermes: missing URL, repeated duplicates, or
  empty titles/snippets across most results.

These thresholds are intentionally conservative. They should be tuned after a
24-hour probe run on representative agent queries.

## Result Contract

The fallback layer should normalize both providers into the same agent-facing
shape:

```text
title
url
snippet
provider
rank
published_or_updated_at
fallback_used
fallback_reason
```

Keep provider metadata. Agents and operators should be able to tell whether a
result came from SearXNG or Perplexity.

## Recommended Flow

```text
Hermes web_search
  -> SearXNG
    -> quality gate passes
      -> return SearXNG results
    -> quality gate fails
      -> Perplexity Search API
        -> normalize results
        -> return results with fallback metadata
```

Do not hide fallback use. At minimum, record the fallback provider and reason in
logs or tool metadata.

## Sonar Use

Sonar is still useful, but for a different job:

- Use Sonar when the caller asks for a fast cited answer or research brief.
- Use Perplexity Search when the caller needs candidate URLs for later
  extraction.
- If Sonar is used after failed URL discovery, expose citations/results as
  citations, not as if they were ordinary SearXNG results.

## Operational Notes

- Treat Perplexity as a managed fallback, not proof that self-hosting is fully
  productionized.
- Track fallback rate. A high fallback rate means SearXNG quality or egress
  reputation needs more work.
- Keep costs visible. A fallback provider changes the cost shape from fixed
  Latitude capacity to per-request API usage.
- Keep Firecrawl fallback separate. Empty Firecrawl markdown is an extraction
  failure, not a SearXNG search failure.

## Open Questions

- What exact Hermes provider hook should own the SearXNG quality gate?
- Should fallback be automatic for all agents, or opt-in per runtime profile?
- What is the initial max monthly Perplexity fallback budget?
- Should we expose a user-visible "searched with fallback" marker in high-stakes
  research flows?
