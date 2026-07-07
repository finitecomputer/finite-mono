# Hermes Integration Runbook

This runbook records how a Hermes or Finite agent runtime should consume the
services from this repo.

## Desired Shape

```text
Hermes web_search  -> SearXNG JSON endpoint
Hermes web_extract -> Firecrawl API endpoint
```

SearXNG remains the primary `web_search` backend. If SearXNG is thin, blocked,
or low quality, the proposed fallback policy is to use Perplexity Search API as
the first managed fallback. See `docs/runbooks/search-fallback-policy.md`.

## Tested Environment

The live `lat2` services are bound to host-local ports:

```bash
SEARXNG_URL=http://127.0.0.1:8080
FIRECRAWL_API_URL=http://127.0.0.1:3002
```

From an operator machine, use SSH tunnels:

```bash
ssh -L 18081:127.0.0.1:8080 -L 13003:127.0.0.1:3002 lat2 -N
```

Then set Hermes endpoint env vars:

```bash
export SEARXNG_URL=http://127.0.0.1:18081
export FIRECRAWL_API_URL=http://127.0.0.1:13003
```

For the smoke scripts, use `FIRECRAWL_URL` instead of Hermes'
`FIRECRAWL_API_URL`:

```bash
SEARXNG_URL=http://127.0.0.1:18081 scripts/smoke-searxng.sh
FIRECRAWL_URL=http://127.0.0.1:13003 scripts/smoke-firecrawl.sh
```

The live Tinfoil SearXNG prototype is gated by `FINITE_SEARCH_TOKEN`. Stock
Hermes does not need to know that token if a localhost token proxy sits in
front of the endpoint.

```bash
# Terminal 1: attested local Tinfoil proxy
tinfoil-proxy \
  -e finite-searxng.finite.containers.tinfoil.dev \
  -r finitecomputer/finite-searxng-tinfoil \
  -p 3399

# Terminal 2: localhost-only token injection proxy
FINITE_SEARCH_TOKEN='<token>' \
  SEARXNG_UPSTREAM_URL=http://127.0.0.1:3399 \
  scripts/searxng-token-proxy.py --port 18999

# Hermes environment
export SEARXNG_URL=http://127.0.0.1:18999
unset SEARXNG_TOKEN
unset FINITE_SEARCH_TOKEN
```

Do not put the raw token in this repo or in the Hermes profile. Keep it in the
proxy process environment, a service manager secret, or a private gateway.

## Hermes Config

Hermes v0.18.0 was current during the completion audit. The bundled provider
paths are:

```text
~/.hermes/hermes-agent/plugins/web/searxng/provider.py
~/.hermes/hermes-agent/plugins/web/firecrawl/provider.py
```

Set the web providers in the Hermes profile:

```yaml
web:
  search_backend: searxng
  extract_backend: firecrawl
```

For the gated Tinfoil SearXNG endpoint, stock Hermes should point at the local
token proxy, not the public Tinfoil URL directly. A direct `SEARXNG_URL` of
`https://finite-searxng.finite.containers.tinfoil.dev` reaches the service but
receives `401 Unauthorized` because Hermes does not send custom headers.

## Smoke Order

1. Run `scripts/smoke-searxng.sh` against the SearXNG URL.
2. Run `scripts/smoke-firecrawl.sh` against the Firecrawl URL.
3. Configure one Hermes profile to use the endpoints.
4. Ask Hermes to search for a simple query.
5. Ask Hermes to extract a known public URL.
6. Record the exact Hermes config and test transcript in the run ledger.

## Proof From 2026-07-01

The real Hermes profile was copied into a temporary `HERMES_HOME` for the proof;
the operator's normal profile was not mutated.

Hermes' own web provider tool layer returned:

```json
{
  "search_success": true,
  "search_results": 3,
  "search_first_title": "Open source - Wikipedia",
  "search_first_url": "https://en.wikipedia.org/wiki/Open_source",
  "extract_success": true,
  "extract_title": "Example Domain",
  "extract_content_chars": 180
}
```

A `hermes -z --toolsets web` one-shot run also passed:

```json
{"search_ok": true, "search_first_title": "Open Source", "extract_ok": true, "extract_title": "Example Domain", "extract_chars": 133}
```

## Completion Audit From 2026-07-01

A fresh temporary `HERMES_HOME` was created with:

```yaml
web:
  search_backend: searxng
  extract_backend: firecrawl
```

Against fresh SSH tunnels to `lat2`, Hermes' web tool layer selected the
expected backends and returned search plus extraction content:

```json
{
  "search_backend": "searxng",
  "extract_backend": "firecrawl",
  "search_success": true,
  "search_results": 3,
  "search_first_title": "Open source - Wikipedia",
  "extract_title": "Example Domain",
  "extract_chars": 180
}
```

A fresh `hermes -z --toolsets web` one-shot agent run also passed:

```json
{"search_ok":true,"search_first_title":"Open source - Wikipedia","extract_ok":true,"extract_title":"Example Domain","extract_chars":148}
```

## Tinfoil SearXNG Proof From 2026-07-02

Canonical container tested:

```text
name: finite-searxng
tag: v0.0.5
domain: finite-searxng.finite.containers.tinfoil.dev
mode: non-staging
secrets: SEARXNG_SECRET, FINITE_SEARCH_TOKEN
```

The development `FINITE_SEARCH_TOKEN` was replaced with a fresh generated value
and the container was relaunched on the same `v0.0.5` tag before testing.

Service-level gate proof:

```text
anonymous /search -> 401
bearer-token /search -> 87 raw results
```

Historical direct-token experiment with a local provider patch:

```text
SearXNG provider without token -> success false, HTTP 401
SearXNG provider with token -> success true, 3 normalized results
```

Hermes `web_search_tool` proof:

```text
web_search_tool("finite computer", limit=3) -> success true, 3 results
first title -> Finite-state machine - Wikipedia
```

This isolated the Tinfoil endpoint behavior, but it is not the recommended
runtime shape because it required a local Hermes provider patch. It does not
change Firecrawl: `web_extract` still points at the `lat2` Docker proof until a
separate Firecrawl Tinfoil design is done.

The preferred no-Hermes-patch proof was run immediately afterward with the
local Hermes SearXNG provider restored to stock behavior. The tested path was:

```text
Hermes web_search
  -> http://127.0.0.1:18999
  -> scripts/searxng-token-proxy.py
  -> tinfoil-proxy on http://127.0.0.1:3399
  -> finite-searxng.finite.containers.tinfoil.dev
```

No-Hermes-patch proof:

```text
direct public anonymous /search -> 401
standalone tinfoil-proxy anonymous /search -> 401
local token proxy /search -> 96 raw results
Hermes token env present -> false
stock Hermes SearXNG provider -> success true, 3 normalized results
stock Hermes web_search_tool -> success true, 3 results
first title -> Finite-state machine - Wikipedia
```

Use standalone `tinfoil-proxy` for this chain. In the local 2026-07-02 test,
`tinfoil container connect` started but failed forwarded `/search` requests with
`Request.RequestURI can't be set in client requests`; standalone
`tinfoil-proxy` did not have that failure.

## Notes

- Keep search and extraction test failures separate.
- Service-level curl tests are useful but are not Hermes proof by themselves.
- Keep Perplexity Search separate from Sonar-style cited briefs: Search is a
  plausible URL-discovery fallback, while Sonar is not a drop-in replacement for
  ranked search results.
- Do not expose these endpoints publicly without auth, rate limits, and an ADR.
