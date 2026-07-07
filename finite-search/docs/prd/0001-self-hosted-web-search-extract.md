# PRD 0001: Self-Hosted Web Search And Extract Bootstrap

Status: implemented-lat2

GitHub issue: https://github.com/finitecomputer/finite-search/issues/1

## Goal

Create the initial `finite-search` repo so Finite can deploy self-hosted
`web_search` and `web_extract` services on a Latitude host, prove them with
simple smokes, and leave a clear path toward Hermes integration and later
Tinfoil packaging.

## Users

- Finite operators deploying agent infrastructure.
- Agents and agent maintainers configuring Hermes profiles.
- Future reviewers evaluating whether the services can move into Tinfoil.

## Requirements

- The repo has domain context and ADRs.
- The repo has Matt Pocock skill setup docs.
- SearXNG has a minimal Compose profile with JSON output enabled.
- Firecrawl has a self-hosting wrapper that points at upstream source and
  records required env.
- `lat2` is documented as the first Docker target.
- Smoke scripts prove SearXNG and Firecrawl independently.
- Tinfoil is documented as a follow-up, not as part of the first proof.

## Non-Goals

- Production DNS.
- Production traffic.
- Public unauthenticated endpoints.
- Replacing hosted providers before benchmark evidence exists.
- Final Tinfoil deployment.

## Acceptance Criteria

- `scripts/check-static.sh` passes.
- A Git repo exists on `main`.
- The initial scaffold is committed.
- The repo can be pushed to `finitecomputer/finite-search`.
- GitHub Issues can track the PRD and tracer-bullet slices after publication.

## Implementation Evidence

- SearXNG is deployed on `lat2` and strict smoke returned non-empty JSON
  results; the final pre-commit smoke returned 136 results.
- Firecrawl is deployed on `lat2` from upstream commit
  `25d95174274a91723b145780fadddefe298d7e5c` and strict scrape smoke returned
  180 markdown chars.
- Hermes was proven against both endpoints with its web provider tool layer and
  a `hermes -z --toolsets web` one-shot run.
- `scripts/benchmark-stack.sh` records repeated search/extract success and
  latency through SSH tunnels.
- `docs/benchmark-comparison-2026-07-01.md` compares the self-hosted benchmark
  with hosted provider pricing, latency references, and observed failure modes.
- `docs/tinfoil-evaluation-2026-07-01.md` records why SearXNG-only is the next
  Tinfoil candidate and why Firecrawl is deferred.
