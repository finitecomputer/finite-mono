# Benchmark And Hosted Provider Comparison

Date: 2026-07-01

## Self-Hosted Evidence From `lat2`

Completion audit through a fresh SSH tunnel:

```text
searxng smoke ok: 155 results
firecrawl smoke ok: 180 markdown chars

benchmark iterations: 3
search_success: 3/3
search_latency: avg=4.147s min=3.567s max=4.690s
extract_success: 3/3
extract_latency: avg=0.355s min=0.210s max=0.531s
extract_last_markdown_chars: 180
```

Earlier 5-iteration benchmark:

```text
search_success: 5/5
search_latency: avg=4.188s min=1.832s max=10.278s
extract_success: 5/5
extract_latency: avg=0.307s min=0.281s max=0.374s
```

Resource snapshot from the Docker spike:

- SearXNG: about `124MiB`
- Firecrawl API: about `2.37GiB` of its `8GiB` limit
- Playwright service: about `154MiB` of its `4GiB` limit
- RabbitMQ, Postgres, Redis, and FoundationDB add smaller but real overhead.

## Hosted References Checked

- Firecrawl pricing: https://www.firecrawl.dev/pricing
- Tavily credits/pricing: https://docs.tavily.com/documentation/api-credits
- Exa pricing: https://exa.ai/pricing
- Exa search latency guide: https://exa.ai/docs/reference/search-api-guide
- Parallel pricing: https://parallel.ai/pricing
- Latitude pricing overview: https://www.latitude.sh/pricing
- Latitude networking pricing: https://www.latitude.sh/pricing/networking

## Hosted Cost Notes

Provider pricing changes over time; treat these as a dated comparison snapshot.

| Provider | Search cost reference | Extract/content cost reference | Latency reference |
| --- | --- | --- | --- |
| Firecrawl hosted | Search costs 2 credits per 10 results | Scrape/Crawl/Map/Monitor cost 1 credit per page; free tier has 1,000 credits/month; no pay-per-use plan | Not enough official latency detail found for this comparison |
| Tavily | Basic search costs 1 credit; advanced search costs 2 credits; pay-as-you-go is $0.008/credit | Basic extract costs 1 credit per 5 successful URL extractions; advanced extract costs 2 credits per 5 successful URL extractions | Not enough official latency detail found for this comparison |
| Exa | Search is $7 per 1,000 requests with up to 10 results | Contents endpoint is $1 per 1,000 pages per content type | Search options range from about 250 ms instant search to 12-40 second deep-reasoning search |
| Parallel | Search API is $5 per 1,000 requests with 10 results; $1 per 1,000 additional results | Extract API is $1 per 1,000 results | Search API is listed as 2-5s; Extract API is listed as 1-3s cached and 60-90s live |

## Cost Model

Self-hosting does not make the work free. It changes the cost shape:

- Hosted APIs charge per credit, request, page, or result.
- The `lat2` deployment has fixed infrastructure cost, operator time, and
  maintenance risk.
- If `lat2` already has spare capacity, the short-term marginal dollar cost for
  these smokes is effectively the box's unused capacity.
- For real adoption, Finite still needs the actual monthly allocation for
  `lat2` before calculating a precise break-even point.

Simple break-even formula:

```text
break_even_requests = monthly_box_share_for_search_extract / hosted_cost_per_request
```

Examples using the hosted prices above:

- Parallel Search: `monthly_box_share / 0.005`
- Exa Search: `monthly_box_share / 0.007`
- Tavily basic search pay-as-you-go: `monthly_box_share / 0.008`
- Parallel Extract: `monthly_box_share / 0.001`

## Failure Modes Observed

- Public search engines can block datacenter IPs. Brave, Startpage, and
  DuckDuckGo were blocked or noisy enough to remove from the first SearXNG
  engine set.
- SearXNG result count and latency fluctuate because upstream engines fluctuate.
- Firecrawl upstream self-hosting is a multi-container stack, not one small
  service. It needs API, Playwright, Redis, RabbitMQ, Postgres, FoundationDB,
  queue workers, writable scratch, and health checks.
- Upstream `nuq-postgres` expects `POSTGRES_DB=postgres` for `pg_cron`; using a
  custom database name broke initialization during the spike.
- Hosted providers may still win on anti-blocking, proprietary indexes, managed
  reliability, and advanced extraction features.

## Interpretation

Self-hosting is viable as a controlled Finite/Hermes option, especially when:

- privacy or data path control matters;
- the workload can tolerate public-engine variability;
- the Latitude box already has spare capacity;
- basic extraction is enough; and
- operators are comfortable owning the Firecrawl stack.

Hosted providers remain valuable as fallbacks or defaults when:

- low-latency indexed search matters;
- anti-blocking quality matters more than infrastructure ownership;
- usage is low enough to fit free tiers;
- the team wants managed SLAs and support; or
- a page needs advanced extraction beyond the self-hosted Firecrawl baseline.

Current recommendation: keep the self-hosted stack as a normal Hermes/Finite
option, not the only option. Use hosted providers as fallbacks until we have
longer-running reliability data and a real `lat2` cost allocation.
