# finite-search Context

## Product Boundary

`finite-search` owns the self-hosted web-retrieval substrate for Finite agents.
It is the place where operators can deploy, smoke test, and eventually harden
`web_search` and `web_extract` services without changing the main
`finitecomputer` host workspace for every experiment.

Avoid treating this repo as:

- a general browser automation product;
- a hosted search SaaS;
- a replacement for Hermes;
- the final Tinfoil attestation story.

## Glossary

**Web Search**

Finding candidate URLs for a natural-language query. In the first implementation
this is SearXNG with JSON output enabled.

Avoid: page extraction, crawling, browser control.

**Web Extract**

Fetching a known URL and returning clean readable text or markdown for the
agent. In the first implementation this is Firecrawl.

Avoid: search-result ranking, agent reasoning, long-term memory.

**Latitude Host**

A Latitude.sh machine reachable from the operator laptop through SSH aliases
such as `lat1` and `lat2`. Latitude hosts are the first target for this repo's
plain Docker deployment.

Avoid: assuming the OVH smoke box is Latitude.

**Smoke Box**

The existing `ovh-vps-smoke` host. It is useful Finite infrastructure, but it is
not the target of the first Latitude deployment.

**Plain Docker Deploy**

The first deploy mode: normal containers, normal host networking, normal logs,
and simple operator commands. This is intentionally before Tinfoil.

Avoid: enclave-specific packaging, attestation, or no-disk constraints.

**Tinfoil Candidate**

A later packaging target for search/extract services after the plain Docker
deploy is proven. Tinfoil changes the operating model because persistent disk,
debug access, image pinning, and public config/attestation constraints matter.

**Hermes Integration**

The agent-runtime wiring that points Hermes' `web_search` provider at SearXNG
and Hermes' `web_extract` provider at Firecrawl.

## Operating Model

This repo should answer four questions quickly:

1. What runs where?
2. How do we start it?
3. How do we prove it works from an agent's point of view?
4. What is still unsafe or unproven before production or Tinfoil?

## Current Decisions

- Use `lat2` as the first Docker target because Docker is already active there.
- Keep SearXNG and Firecrawl as separate services behind separate smoke checks.
- Run plain Docker before Tinfoil packaging.
- Use root `CONTEXT.md` plus root `docs/adr/` as the domain-doc layout.
- Use GitHub Issues once the repo is published.

## Out Of Scope For The Bootstrap

- Public production DNS.
- Production traffic.
- User secrets.
- Tinfoil attestation-gated key release.
- A custom agent UI.
- Replacing hosted providers before benchmark evidence exists.

