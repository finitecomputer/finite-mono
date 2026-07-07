# Issue 0004: Document Hermes Integration Shape

Type: AFK

Status: proven-lat2

GitHub issue: https://github.com/finitecomputer/finite-search/issues/5

## Acceptance Criteria

- The runbook explains the SearXNG and Firecrawl URLs Hermes should consume.
- Service-level smokes are separated from true Hermes profile proof.
- The repo records the actual Hermes proof once a profile calls both endpoints.

## Evidence

- Hermes provider config uses `web.search_backend=searxng` and
  `web.extract_backend=firecrawl`.
- Tool-layer proof through SSH tunnels returned search results and extracted
  `https://example.com`.
- `hermes -z --toolsets web` one-shot proof returned:
  `{"search_ok": true, "search_first_title": "Open Source", "extract_ok": true, "extract_title": "Example Domain", "extract_chars": 133}`.
