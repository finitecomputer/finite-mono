# Firecrawl Compose Wrapper

Firecrawl's current self-host path is source-first: clone the upstream repo and
run its top-level `docker-compose.yaml` from that checkout.

This directory keeps Finite-specific defaults and scripts around that upstream
deploy instead of copying a large upstream compose file into this repo.

## Prepare Upstream

For local prep or a host where this repo is checked out:

```bash
scripts/bootstrap-firecrawl-upstream.sh ~/finite-search/firecrawl-upstream
```

For `lat2`, clone upstream on the host:

```bash
mkdir -p ~/finite-search
cd ~/finite-search
git clone https://github.com/firecrawl/firecrawl.git firecrawl-upstream
```

Copy Finite's env template and override into that upstream checkout before
starting the stack:

```bash
rsync -av compose/firecrawl/.env.example \
  lat2:~/finite-search/firecrawl-upstream/.env.example.finite
rsync -av compose/firecrawl/docker-compose.override.yml \
  lat2:~/finite-search/firecrawl-upstream/docker-compose.override.yml
```

Then on the target host:

```bash
cd ~/finite-search/firecrawl-upstream
cp .env.example.finite .env
BULL_AUTH_KEY="$(openssl rand -hex 32)"
POSTGRES_PASSWORD="$(openssl rand -hex 32)"
sed -i "s/^BULL_AUTH_KEY=.*/BULL_AUTH_KEY=${BULL_AUTH_KEY}/" .env
sed -i "s/^POSTGRES_PASSWORD=.*/POSTGRES_PASSWORD=${POSTGRES_PASSWORD}/" .env
docker compose up -d --build
```

The override keeps upstream Compose as the source of truth while adding restart
policies, an API readiness healthcheck, a named Postgres volume, and idempotent
FoundationDB initialization.

## Notes

- Self-hosted Firecrawl does not include every hosted Firecrawl anti-blocking
  feature.
- Keep `BULL_AUTH_KEY` strong before exposing any admin route.
- Keep database and queue ports private.
- Keep `POSTGRES_DB=postgres` unless upstream changes `nuq-postgres`; its
  `pg_cron` config currently expects the `postgres` database.
- The completed Docker milestone only needed `POST /v1/scrape` to return
  readable content for a normal public URL.

## Verified On `lat2`

- Upstream commit: `25d95174274a91723b145780fadddefe298d7e5c`
- API bind: `127.0.0.1:3002`
- Strict smoke: `firecrawl smoke ok: 180 markdown chars`
- Health endpoints: `/v0/health/liveness` and `/v0/health/readiness` both
  returned `{"status":"ok"}`.
