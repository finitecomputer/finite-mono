# Latitude Docker Spike

This runbook proves the first self-hosted search/extract stack on a Latitude
host.

## Target

Use `lat2` first.

Observed current point on 2026-07-01:

- SSH alias: `lat2`
- Hostname: `finite-lat-2`
- Provider: Latitude.sh
- User: `ubuntu`
- Docker: active
- Docker Compose plugin: available as `docker compose`
- CPU: 32
- Memory: about 187 GiB
- Root disk: about 439 GiB, 69 GiB used, about 17 percent used after deploy

## Preflight

From this repo:

```bash
scripts/doctor.sh lat2
```

Expected result:

- SSH works.
- Docker is present and active.
- Compose availability is reported.
- Host resources are printed.

If `docker compose` becomes unavailable again, reinstall `docker-compose-v2`
with apt or temporarily use plain `docker run` while keeping the final
deployment in this repo as Compose.

## Deploy SearXNG

Copy the SearXNG compose directory to the host:

```bash
rsync -av compose/searxng/ lat2:~/finite-search/searxng/
```

On the host:

```bash
cd ~/finite-search/searxng
cp .env.example .env
printf 'SEARXNG_SECRET=%s\n' "$(openssl rand -hex 32)" >> .env
docker compose up -d
```

Smoke from the host:

```bash
curl -fsS 'http://127.0.0.1:8080/search?q=test&format=json' | head
```

The Compose healthcheck uses SearXNG's local `/healthz` endpoint. Do not use a
real search query as the healthcheck; doing so creates background traffic to
upstream engines and can make rate limits look worse than they are.

Smoke from the operator machine through SSH:

```bash
ssh -L 18080:127.0.0.1:8080 lat2 -N
SEARXNG_URL=http://127.0.0.1:18080 scripts/smoke-searxng.sh
```

Verified on 2026-07-01:

```text
searxng smoke ok: 136 results
```

The live instance is intentionally narrowed to engines that worked from the
Latitude datacenter IP. The default public engines `brave`, `duckduckgo`, and
`startpage` were blocked by rate limits or CAPTCHA during the first smoke.
`duckduckgo web` was later disabled because it produced repeated backend
exceptions from this IP range.

## Deploy Firecrawl

Use the upstream Firecrawl source for the first proof. The current upstream
self-host path expects building from source with Docker Compose.

On the host:

```bash
mkdir -p ~/finite-search
cd ~/finite-search
git clone https://github.com/firecrawl/firecrawl.git firecrawl-upstream
cd firecrawl-upstream
```

From the operator machine, copy the Finite env template and hardening override
into the upstream checkout:

```bash
rsync -av compose/firecrawl/.env.example \
  lat2:~/finite-search/firecrawl-upstream/.env.example.finite
rsync -av compose/firecrawl/docker-compose.override.yml \
  lat2:~/finite-search/firecrawl-upstream/docker-compose.override.yml
```

Then on the host:

```bash
cd ~/finite-search/firecrawl-upstream
cp .env.example.finite .env
BULL_AUTH_KEY="$(openssl rand -hex 32)"
POSTGRES_PASSWORD="$(openssl rand -hex 32)"
sed -i "s/^BULL_AUTH_KEY=.*/BULL_AUTH_KEY=${BULL_AUTH_KEY}/" .env
sed -i "s/^POSTGRES_PASSWORD=.*/POSTGRES_PASSWORD=${POSTGRES_PASSWORD}/" .env
docker compose up -d --build
```

The current proof uses upstream commit
`25d95174274a91723b145780fadddefe298d7e5c`.

Keep these Firecrawl details as-is unless upstream changes:

- `PORT=127.0.0.1:3002` keeps the API host-local.
- `POSTGRES_DB=postgres` is required because upstream `nuq-postgres` configures
  `pg_cron` against the `postgres` database.
- `compose/firecrawl/docker-compose.override.yml` adds restart policies, an API
  readiness healthcheck, a named Postgres volume, and idempotent
  FoundationDB initialization.

Set `SEARXNG_ENDPOINT=http://127.0.0.1:8080` in Firecrawl's `.env` only if
Firecrawl's own `/search` API should use the same SearXNG instance. Hermes
`web_search` should still talk directly to SearXNG.

Smoke from the operator machine through SSH:

```bash
ssh -L 13002:127.0.0.1:3002 lat2 -N
FIRECRAWL_URL=http://127.0.0.1:13002 scripts/smoke-firecrawl.sh
```

Verified on 2026-07-01:

```text
firecrawl smoke ok: 180 markdown chars
```

Direct host health checks also returned:

```text
/v0/health/liveness  -> {"status":"ok"}
/v0/health/readiness -> {"status":"ok"}
```

## Hermes Proof

Hermes was tested from the operator machine through SSH tunnels. The profile
used these settings:

```yaml
web:
  search_backend: searxng
  extract_backend: firecrawl
```

```bash
SEARXNG_URL=http://127.0.0.1:18081
FIRECRAWL_API_URL=http://127.0.0.1:13003
```

Hermes' own web provider tool layer returned:

```json
{
  "search_success": true,
  "search_results": 3,
  "search_first_title": "Open source - Wikipedia",
  "extract_success": true,
  "extract_title": "Example Domain",
  "extract_content_chars": 180
}
```

A `hermes -z --toolsets web` one-shot run also passed:

```json
{"search_ok": true, "search_first_title": "Open Source", "extract_ok": true, "extract_title": "Example Domain", "extract_chars": 133}
```

## Benchmark

Run a simple repeated smoke benchmark through SSH tunnels:

```bash
SEARXNG_URL=http://127.0.0.1:18081 \
FIRECRAWL_URL=http://127.0.0.1:13003 \
BENCHMARK_ITERATIONS=5 \
scripts/benchmark-stack.sh
```

Observed on 2026-07-01:

```text
search_success: 5/5
search_latency: avg=4.188s min=1.832s max=10.278s
search_last_results: 122
extract_success: 5/5
extract_latency: avg=0.307s min=0.281s max=0.374s
extract_last_markdown_chars: 180
```

Resource snapshot during the spike:

- SearXNG: about `124MiB`
- Firecrawl API: about `2.37GiB` of its `8GiB` limit
- Playwright service: about `154MiB` of its `4GiB` limit
- RabbitMQ, Postgres, Redis, and FoundationDB add smaller but real overhead.

The host was not an idle lab baseline; unrelated high-CPU containers were
running during this benchmark.

Completion audit on 2026-07-01 through a fresh SSH tunnel:

```text
searxng smoke ok: 155 results
firecrawl smoke ok: 180 markdown chars
search_success: 3/3
search_latency: avg=4.147s min=3.567s max=4.690s
extract_success: 3/3
extract_latency: avg=0.355s min=0.210s max=0.531s
extract_last_markdown_chars: 180
```

## Acceptance

The Latitude Docker spike is accepted when:

- `scripts/doctor.sh lat2` reports usable container tooling.
- SearXNG JSON smoke passes.
- Firecrawl scrape smoke passes.
- Service logs show no repeated crash loops.
- Resource usage is acceptable during a few repeated smokes.
- Hermes integration notes are updated with the tested URLs.
- Tinfoil follow-up notes identify the first candidate and blockers.
