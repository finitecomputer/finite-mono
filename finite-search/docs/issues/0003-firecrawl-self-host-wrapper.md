# Issue 0003: Add Firecrawl Self-Host Wrapper

Type: AFK

Status: deployed-hardened-lat2

GitHub issue: https://github.com/finitecomputer/finite-search/issues/4

## Acceptance Criteria

- Firecrawl env requirements are documented.
- The runbook points at upstream Firecrawl source for the first proof.
- `scripts/bootstrap-firecrawl-upstream.sh` can prepare an upstream checkout.
- `scripts/smoke-firecrawl.sh` proves URL extraction.

## Evidence

- Firecrawl is deployed on `lat2` from upstream commit
  `25d95174274a91723b145780fadddefe298d7e5c`.
- The API is bound to `127.0.0.1:3002`.
- `compose/firecrawl/docker-compose.override.yml` adds restart policies, API
  readiness checks, a named Postgres volume, and idempotent FoundationDB init.
- SSH-tunneled strict smoke passed on 2026-07-01:
  `firecrawl smoke ok: 180 markdown chars`.
- API liveness and readiness returned `{"status":"ok"}`.
