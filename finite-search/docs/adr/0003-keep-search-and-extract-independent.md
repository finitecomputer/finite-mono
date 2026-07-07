# ADR 0003: Keep Search And Extract Independent

Date: 2026-06-30

Status: accepted

## Context

Firecrawl can be configured to use a SearXNG endpoint for its own `/search`
API, but Hermes' model is clearer if `web_search` and `web_extract` are
operator-visible services with independent smoke checks.

Combining them too early would make failures harder to debug:

- Search engines can rate-limit or block SearXNG.
- Firecrawl can fail because of Playwright/browser, Redis, queue, or page
  extraction issues.
- Hermes can be misconfigured even when both services are healthy.

## Decision

Run and verify SearXNG and Firecrawl independently. Firecrawl may point at the
same SearXNG instance for Firecrawl's own `/search` API, but Hermes integration
must still prove both paths separately.

## Consequences

- Smoke scripts stay small and targeted.
- A failed search smoke does not imply extraction is broken.
- A failed extract smoke does not imply search is broken.
- Later dashboards can expose separate health and latency metrics.

