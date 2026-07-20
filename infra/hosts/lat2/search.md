# finite-search on finite-lat-2

> **SUPERSEDED 2026-07-09 — DO NOT DEPLOY FROM THIS FILE.** Search moved to
> `finite-lat-1`; lat2 remains CI/build infrastructure. This is a dated
> 2026-07-08 capture only. Current topology is
> [`infra/README.md`](../../README.md), and current Search configuration lives
> under [`infra/nixos/`](../../nixos/).

Self-hosted web_search (SearXNG) + web_extract (Firecrawl) for the agent
runtimes. Both loopback-only; nothing public. Compose sources are **not**
duplicated here — they live in `finite-search/compose/` in this repo (the
former finitecomputer/finite-search repo). Captured 2026-07-08.

## On-host layout

| Compose project | Host path | Config files | Bind |
|---|---|---|---|
| `finite-search-searxng` | `/home/ubuntu/finite-search/searxng/` | `compose.yml` + `settings.yml` (from `finite-search/compose/searxng/`), `.env` | 127.0.0.1:8080 |
| `firecrawl` | `/home/ubuntu/finite-search/firecrawl-upstream/` | upstream `docker-compose.yaml` (shallow git clone of firecrawl/firecrawl) + `docker-compose.override.yml` (from `finite-search/compose/firecrawl/`), `.env` | api on 127.0.0.1:3002 |

- SearXNG: `searxng/searxng:latest`, healthchecked, `settings.yml` curates
  ~21 engines (`keep_only`), `limiter: false`, `public_instance: false`,
  JSON output enabled (agents consume `format=json`).
- Firecrawl: upstream compose is the source of truth; the Finite override
  adds restart policies, an api readiness healthcheck, a named
  `nuq-postgres-data` volume, and idempotent FoundationDB init. Running
  services at capture: api, playwright-service, redis, rabbitmq,
  nuq-postgres, foundationdb (7.3.63). `docker compose ls` reports
  `exited(1), running(6)` — one exited service, likely the one-shot
  `foundationdb-init` (`restart: "no"`), but its identity was not confirmed
  during capture.
- Env files (names only; values on host): `searxng/.env` — `SEARXNG_BIND`,
  `SEARXNG_PORT`, `SEARXNG_BASE_URL`, `SEARXNG_LIMITER`, `SEARXNG_SECRET`;
  `firecrawl-upstream/.env` — `PORT`, `HOST`, `USE_DB_AUTHENTICATION`,
  `BULL_AUTH_KEY`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB`,
  `MAX_CPU`, `MAX_RAM`.

## Repo sources and deploy flow

- `finite-search/compose/searxng/` — compose.yml + settings.yml.
- `finite-search/compose/firecrawl/` — docker-compose.override.yml +
  README.md documenting the wrapper flow (clone upstream on the host, copy
  the override and env template in, `docker compose up -d --build`).
- `finite-search/scripts/` — bootstrap, smoke (`smoke-searxng.sh`,
  `smoke-firecrawl.sh`, `smoke-stack.sh`), probe, benchmark, doctor.
- Runbooks: `finite-search/docs/runbooks/`.

## Delta: on-host vs repo (checked 2026-07-08)

- `searxng/compose.yml`: **identical**.
- `searxng/settings.yml`: **identical** (capture elided the repeated
  `engines:` disabled:false block; spot-check matched).
- `firecrawl docker-compose.override.yml`: **identical**.
- Firecrawl upstream `docker-compose.yaml`: comes from the on-host shallow
  clone (cloned 2026-07-01), not from this repo — by design per the wrapper
  README. Upstream drift is therefore pinned only by the clone date; there
  is no commit pin recorded in-repo. Worth adding a pin when this flow is
  next touched.
