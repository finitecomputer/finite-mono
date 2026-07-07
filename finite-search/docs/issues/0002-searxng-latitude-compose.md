# Issue 0002: Add SearXNG Latitude Compose Profile

Type: AFK

Status: deployed-on-lat2

GitHub issue: https://github.com/finitecomputer/finite-search/issues/3

## Acceptance Criteria

- `compose/searxng/compose.yml` starts SearXNG.
- `compose/searxng/settings.yml` enables JSON output.
- `.env.example` documents required operator values.
- `scripts/smoke-searxng.sh` proves JSON search output.

## Evidence

- `scripts/doctor.sh ubuntu@64.34.80.19` reports Docker and Compose available.
- SearXNG is running as `finite-search-searxng-searxng-1`.
- SSH-tunneled smoke passed on 2026-07-01 after disabling the noisy
  `duckduckgo web` engine for the Latitude IP range. The final pre-commit smoke
  returned 136 JSON results.
