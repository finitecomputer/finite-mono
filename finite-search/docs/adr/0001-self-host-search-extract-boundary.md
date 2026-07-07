# ADR 0001: Own Web Search And Web Extract As A Small Service Boundary

Date: 2026-06-30

Status: accepted

## Context

Hermes-style agents need two different web capabilities:

- `web_search`: find candidate URLs for a query.
- `web_extract`: fetch a known URL and return clean readable content.

Nous' Hermes docs describe these as distinct web tools. The recent Hermes
performance framing also points at the same separation: agents are cheaper and
faster when they receive clean extracted content instead of pushing messy pages
through the model.

## Decision

Create `finite-search` as a small Finite-owned ops repo for self-hosting these
two capabilities:

- SearXNG backs `web_search`.
- Firecrawl backs `web_extract`.

The repo owns deploy/runbook/config/smoke artifacts, not agent reasoning or
product UI.

## Consequences

- The service boundary is reusable across Hermes profiles and future Finite
  agent runtimes.
- We can test hosted-vs-self-hosted cost and latency without changing agent
  logic.
- Failures can be isolated to search, extraction, or Hermes integration.
- The repo must avoid becoming a grab bag for unrelated browser automation.

