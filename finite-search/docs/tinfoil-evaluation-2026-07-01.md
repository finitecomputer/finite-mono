# Tinfoil Packaging Evaluation: Search/Extract

Date: 2026-07-01

## Sources Checked

- Tinfoil Containers overview: https://docs.tinfoil.sh/containers/overview
- Tinfoil configuration reference: https://docs.tinfoil.sh/containers/configuration
- Tinfoil request/attestation flow: https://docs.tinfoil.sh/containers/connecting
- Tinfoil private image notes: https://docs.tinfoil.sh/containers/private-images

## Current Evidence From `lat2`

- SearXNG runs as one service on `127.0.0.1:8080`.
- SearXNG strict smoke passed after disabling the noisy DuckDuckGo engine. The
  final pre-commit smoke returned `searxng smoke ok: 136 results`.
- Firecrawl runs from upstream source at commit
  `25d95174274a91723b145780fadddefe298d7e5c`.
- Firecrawl strict smoke passed after hardening:
  `firecrawl smoke ok: 180 markdown chars`.
- Benchmark through explicit SSH tunnels:
  - search: `5/5`, avg `4.188s`, min `1.832s`, max `10.278s`
  - extract: `5/5`, avg `0.307s`, min `0.281s`, max `0.374s`
- Resource snapshot:
  - SearXNG: about `124MiB`
  - Firecrawl API: about `2.37GiB` of its `8GiB` limit
  - Playwright service: about `154MiB` of its `4GiB` limit
  - RabbitMQ/Postgres/Redis/FoundationDB add smaller but real overhead.

## Tinfoil Constraints That Matter

- `tinfoil-config.yml` must live at the root of a public GitHub repo.
- Container images must be pinned by SHA256 digest.
- The Docker image may be private, but registry auth for private images is an
  enterprise feature.
- The enclave filesystem has no persistent disk. Writable state is lost on
  restart/redeploy.
- Containers are public-inbound through the Tinfoil shim; there is no inbound
  private networking.
- Containers have no outbound network by default. Search/scrape services need
  explicit egress, probably `open` for general web access.
- Tinfoil has Docker-style healthchecks and restart policies, which map well to
  the hardening added in the plain Docker spike.

## Candidate Ranking

1. SearXNG alone.
   It is a small, mostly stateless single-container workload. The main open
   design question is exposure: SearXNG would be reachable on the public
   internet unless we put authentication in front of it or build a tiny
   token-gated wrapper.

2. Firecrawl later.
   It is possible in principle because Tinfoil supports multi-container
   configs, but Firecrawl has more moving pieces: API, Playwright, Redis,
   RabbitMQ, Postgres, FoundationDB, queue workers, writable scratch, and
   open-web egress. It also benefits from durable service state, while Tinfoil
   containers do not provide persistent disk.

3. Combined SearXNG plus Firecrawl later still.
   Combining them increases the public config, networking, healthcheck, startup,
   and troubleshooting surface. It should wait until each service has an
   independent candidate image and smoke.

## Recommendation

Update on 2026-07-02: the SearXNG-only path is now Tinfoil-ready as a verified
prototype.

Completed after this evaluation:

- Created public repo `finitecomputer/finite-searxng-tinfoil`.
- Found the supported CPU-only hardware shape:
  `cvm-version: 0.10.4`, `8 CPU / 16384 MiB`.
- Deployed and verified `finite-searxng` on `v0.0.4`.
- Published and deployed `v0.0.5` with a measured bearer-token proxy.
- Verified anonymous `/search` rejects with 401 and authorized `/search` works
  through `tinfoil-proxy`.
- Proved the gated `v0.0.5` endpoint through stock Hermes' SearXNG provider and
  `web_search_tool` by placing a localhost token proxy in front of
  `tinfoil-proxy`; no Hermes patch is required for that path.

The remaining SearXNG work is operational: rotate development keys and decide
where the long-lived client token lives. Firecrawl remains a separate Tinfoil
design task.

Historical recommendation:

The next Tinfoil milestone should be a SearXNG-only prototype:

- Build or select a SearXNG image and pin it by digest.
- Create a public Tinfoil config repo with no secrets.
- Add a healthcheck for `/search?q=health&format=json`.
- Decide whether to expose raw SearXNG or a token-gated wrapper.
- Use `egress: open` unless a realistic engine allowlist is chosen.
- Deploy in debug first, then production.
- Smoke through Tinfoil's verified client or proxy, not plain `curl`.

That prototype bundle is now prepared in `tinfoil/searxng-public/`, but it has
not been deployed. Public repo creation and Tinfoil deployment remain human
gates.

Firecrawl should remain a follow-up until we know how to handle ephemeral
Postgres/Redis state, browser scratch directories, open egress, and public API
authentication inside the enclave model.
