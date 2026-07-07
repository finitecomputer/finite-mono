# Production Readiness Investigation

Date: 2026-07-01

## Question

The Docker proof answered whether SearXNG and Firecrawl can run on `lat2` and
whether Hermes can consume them. This investigation asks what is still unproven
before Finite treats the stack as a boring daily dependency.

## Current Live Evidence

Baseline checks on 2026-07-01:

- `scripts/check-static.sh` passed.
- `scripts/doctor.sh lat2` reported Docker active with Compose available.
- `lat2` reported 32 CPUs, about 187 GiB RAM, and root disk at 17 percent used.
- The finite-search containers were healthy with restart count `0` during the
  probe window.

Happy-path smokes through SSH tunnels still pass:

```text
searxng smoke ok: 156 results
firecrawl smoke ok: 180 markdown chars
```

Five repeated happy-path benchmark iterations also passed:

```text
search_success: 5/5
search_latency: avg=3.220s min=0.703s max=6.209s
extract_success: 5/5
extract_latency: avg=0.280s min=0.228s max=0.320s
```

## Search Findings

The search service was available, but result quality varied by query.

Observed examples:

| Query | Result |
| --- | --- |
| `open source` | 122 results; first result was Wikipedia. |
| `finite computer hermes runtime` | 10 results; first result was unrelated dictionary content. |
| `firecrawl self hosted docker compose` | 14 results; first result was Firecrawl's product site. |
| `site:github.com firecrawl firecrawl` | 0 results. |

SearXNG also reported unresponsive engines during the initial manual probe:

- `presearch`: suspended for too many requests.
- `wikibooks`: suspended for too many requests.
- `wikiquote`: suspended for too many requests.
- `yep`: suspended for access denied.

A later run through `scripts/probe-stack.sh` showed the variability directly:
the `site:github.com firecrawl firecrawl` query returned 20 results instead of
0, but the first result was a SourceForge mirror rather than GitHub. At that
point only `presearch` was reported unresponsive. Treat these probes as evidence
of fluctuation, not stable quality scores.

Important config finding: the SearXNG Docker healthcheck was using a real search
query every 30 seconds. That creates background upstream traffic. The compose
healthcheck now uses `/healthz` instead, so future engine-rate observations are
less polluted by our own health checks.

## Extract Findings

Firecrawl extracted normal pages quickly:

| URL | Result |
| --- | --- |
| `https://example.com` | Success; 180 markdown chars. |
| `https://en.wikipedia.org/wiki/Open_source` | Success; 200596 markdown chars. |
| `https://github.com/firecrawl/firecrawl` | Success; 49209 markdown chars. |
| `https://docs.firecrawl.dev/introduction` | Success; 10156 markdown chars. |
| `https://news.ycombinator.com/` | Success; 17795 markdown chars. |
| `https://www.reuters.com/technology/` | API returned `success=true`, but markdown length was `0`. |

That Reuters result is the important limitation: service health and API success
are not enough. Hermes needs a content-quality gate that treats empty markdown
as extraction failure and falls back.

## Tooling Added

Use `scripts/probe-stack.sh` for a broader non-strict probe:

```bash
SEARXNG_URL=http://127.0.0.1:18080 \
FIRECRAWL_URL=http://127.0.0.1:13002 \
scripts/probe-stack.sh
```

The script prints TSV rows for search and extract cases, including zero-result
searches, unresponsive SearXNG engines, empty Firecrawl markdown, and a summary.
Set `PROBE_STRICT=true` when the caller wants any failed case to produce a
non-zero exit.

## Interpretation

The current stack is healthy enough for controlled internal use and continued
Hermes experiments. It is not yet proven as a production dependency because the
unproven parts are quality, coverage, access, and operations:

- Search availability does not imply useful ranking for Finite-specific or
  operator-shaped queries.
- Search quality depends on upstream engines, and some engines are already
  access-denied or rate-limited from the Latitude IP.
- Firecrawl can return an API-level success with empty content on real sites.
- The current evidence is minutes of probing, not a multi-hour or multi-day
  soak.
- Endpoints remain host-local through SSH tunnels, with no production access
  layer, auth, rate limit, TLS, service token, or runtime-networking decision.
- Monitoring, alerting, log retention, backup/restore, upgrade procedure, and
  fallback implementation are still design work.

The first proposed search fallback policy is captured in
`docs/runbooks/search-fallback-policy.md`. It keeps SearXNG primary and treats
Perplexity Search API as the first managed fallback candidate for weak search
results.

## Next Measurements

1. Deploy the `/healthz` healthcheck on `lat2`, then re-run the probe after the
   old SearXNG engine suspensions have cooled down.
2. Run `scripts/probe-stack.sh` on a representative query and URL matrix every
   15 minutes for at least 24 hours, saving TSV output.
3. Implement and test the first Hermes fallback policy: zero search results,
   too many unresponsive engines, Firecrawl timeout, and empty markdown should
   all be explicit fallback triggers.
4. Separate search quality evaluation from service uptime by maintaining a
   small expected-result query set.
5. Add production access and operations ADRs only after the soak data shows
   which failure modes are common enough to design around.
