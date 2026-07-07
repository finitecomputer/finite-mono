# finite-search

Self-hosted web search and web extraction for Finite agent runtimes.

The first job of this repo is to make a boring, repeatable Latitude deploy for
the two web tools agents need:

```text
Hermes / Finite agent runtime
  -> web_search  -> SearXNG
  -> web_extract -> Firecrawl
```

This repo is not a product app. It is an ops and integration repo for:

- SearXNG configuration for search-result discovery.
- Firecrawl self-hosting notes for readable page extraction.
- Latitude host deployment runbooks.
- Smoke scripts that prove both services from the agent point of view.
- Hermes configuration notes.
- Tinfoil follow-up notes now that the plain Docker version works.

## Current Status

The repo bootstrap is complete and pushed to the private GitHub repo:

```text
https://github.com/finitecomputer/finite-search
```

What is done:

- `main` is the active branch.
- GitHub Issues are configured for the bootstrap PRD and tracer-bullet slices.
- GitHub Actions runs `scripts/check-static.sh`.
- `lat2` preflight works over SSH.
- Docker Compose is installed on `lat2`.
- SearXNG is deployed on `lat2` and bound to host-local port `8080`.
- Firecrawl is deployed from upstream source on `lat2` and bound to host-local
  port `3002`.
- SearXNG and Firecrawl strict smokes pass through SSH tunnels.
- Hermes has been proven against both endpoints using its own web provider
  tool layer and a one-shot agent run.
- A small benchmark script records repeat search/extract latency.
- Plain Docker hardening is captured with a Firecrawl Compose override.
- Tinfoil SearXNG is deployed, gated, verified, and proven through Hermes'
  `web_search` tool path.
- Firecrawl remains the next Tinfoil design candidate.

## Current Target

Start on `lat2`:

| Alias | Hostname | Provider | Notes |
| --- | --- | --- | --- |
| `lat2` | `finite-lat-2` | Latitude.sh | First Docker target. Docker and Compose are active. |
| `lat1` | `finite-lat-1` | Latitude.sh | Has k3s active; Docker inactive. |
| `smoke` | `ovh-vps-smoke` | OVH | Existing Finite smoke box, not Latitude. |

## Quick Start

Local static verification:

```bash
scripts/check-static.sh
```

Remote host preflight:

```bash
scripts/doctor.sh lat2
```

The services are deliberately host-local on `lat2`. For local smokes, open SSH
tunnels in one shell:

```bash
ssh -L 18080:127.0.0.1:8080 -L 13002:127.0.0.1:3002 lat2 -N
```

Then run service smokes from this repo:

```bash
SEARXNG_URL=http://127.0.0.1:18080 scripts/smoke-searxng.sh
FIRECRAWL_URL=http://127.0.0.1:13002 scripts/smoke-firecrawl.sh
```

Run the stack benchmark:

```bash
SEARXNG_URL=http://127.0.0.1:18080 \
FIRECRAWL_URL=http://127.0.0.1:13002 \
scripts/benchmark-stack.sh
```

Run a broader non-strict quality probe:

```bash
SEARXNG_URL=http://127.0.0.1:18080 \
FIRECRAWL_URL=http://127.0.0.1:13002 \
scripts/probe-stack.sh
```

## Repository Map

- `CONTEXT.md` - domain vocabulary and operating boundaries.
- `docs/adr/` - durable architectural decisions.
- `docs/runbooks/` - operator procedures for Latitude, Hermes, and Tinfoil.
- `docs/runbooks/search-fallback-policy.md` - proposed SearXNG quality gate and
  Perplexity Search fallback policy.
- `docs/benchmark-comparison-2026-07-01.md` - dated benchmark and hosted
  provider comparison.
- `docs/production-readiness-investigation-2026-07-01.md` - current evidence
  and remaining production-readiness gaps.
- `compose/searxng/` - small SearXNG compose profile with JSON enabled.
- `compose/firecrawl/` - self-host Firecrawl wrapper notes and env template.
- `tinfoil/searxng-public/` - public-repo-ready SearXNG Tinfoil prototype
  bundle.
- `scripts/` - preflight and smoke scripts.
- `docs/feature-dev/` - plebdev feature-dev run ledger.
- `docs/prd/` and `docs/issues/` - local copies of the GitHub PRD and
  tracer-bullet issues.

## Completed Milestone

The first operational milestone was not "perfect private browsing infra." It
was:

1. `lat2` has a simple Docker deploy path with Compose available.
2. SearXNG returns JSON search results.
3. Firecrawl returns readable content for a normal public URL.
4. A Hermes runtime can point `web_search` and `web_extract` at those services.
5. Resource and latency evidence is recorded.
6. The remaining Tinfoil work is documented as a follow-up, not mixed into the
   first Docker proof.

## GitHub Issues

The bootstrap tracker is:

| Issue | Purpose |
| --- | --- |
| `#1` | PRD: self-hosted web search and extract bootstrap |
| `#2` | Repo scaffold, context, and ADRs |
| `#3` | SearXNG Latitude Compose profile |
| `#4` | Firecrawl self-host wrapper |
| `#5` | Hermes integration notes |
| `#6` | Tinfoil follow-up boundary |

## Next Step

The SearXNG-only Tinfoil prototype bundle now lives in
`tinfoil/searxng-public/`, with the public deployment repo at
`finitecomputer/finite-searxng-tinfoil`.

Current Tinfoil state:

- `finite-searxng` is the canonical container and verifies on gated `v0.0.5`.
- `finite-searxng-medium` remains a working `v0.0.4` fallback.
- `v0.0.5` adds a measured bearer-token proxy in front of `/search` using the
  `FINITE_SEARCH_TOKEN` Tinfoil secret.
- Anonymous `/search` now returns 401; authorized `/search` works through the
  verified Tinfoil proxy.
- Stock Hermes `web_search` works against the gated Tinfoil endpoint through a
  localhost token proxy in front of `tinfoil-proxy`; Hermes only needs
  `SEARXNG_URL`.

Before wiring this into a long-lived client runtime, rotate the development
Tinfoil admin key and replace the generated `FINITE_SEARCH_TOKEN` with a
team-owned secret. Firecrawl should stay on the plain Docker path until its
multi-container state, browser, egress, and auth model are designed for Tinfoil.
