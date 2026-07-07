# finite-search Bootstrap Goal Ledger

## Run

- Run ID: 2026-06-30-finite-search-bootstrap
- Loop: plebdev-feature-dev, adapted for a new repo on `main`
- Target repo: `finite-search`
- Base branch: `main`
- Feature branch: none; user requested all work on `main` to start
- Human owner: Austin
- Started: 2026-06-30
- Current status: SearXNG and Firecrawl deployed on `lat2`; Hermes proven
  through both endpoints; Tinfoil candidate evaluated
- Skill setup status: created with GitHub Issues, default triage labels, and
  single-context domain docs

## Goal

Set up a new `finite-search` repo end to end for self-hosted SearXNG
`web_search` and Firecrawl `web_extract` on a Latitude box, starting on `main`,
with context and ADRs defined.

## Durable Artifacts

- CONTEXT updates: `CONTEXT.md`
- ADRs:
  - `docs/adr/0001-self-host-search-extract-boundary.md`
  - `docs/adr/0002-latitude-plain-docker-first.md`
  - `docs/adr/0003-keep-search-and-extract-independent.md`
  - `docs/adr/0004-follow-up-tinfoil-packaging.md`
- PRD issue: https://github.com/finitecomputer/finite-search/issues/1
- Slice issues:
  - https://github.com/finitecomputer/finite-search/issues/2
  - https://github.com/finitecomputer/finite-search/issues/3
  - https://github.com/finitecomputer/finite-search/issues/4
  - https://github.com/finitecomputer/finite-search/issues/5
  - https://github.com/finitecomputer/finite-search/issues/6
- Issue sessions: local bootstrap handled in this thread
- Agent briefs: `AGENTS.md`, `docs/agents/`
- Review packets:
  - `docs/feature-dev/2026-06-30-bootstrap-review-packet.md`
- Local CodeRabbit report:
  - `docs/feature-dev/2026-06-30-coderabbit-round-1.md`
- PR URL: not applicable; user requested `main`

## Commands

- Install: no install required for static docs/scripts
- Typecheck: `scripts/check-static.sh`
- Test: `scripts/check-static.sh`
- Build: not applicable
- Visual verification: not applicable
- Remote smokes:
  - `scripts/smoke-searxng.sh` through an SSH tunnel to `lat2`
  - `scripts/smoke-firecrawl.sh` through an SSH tunnel to `lat2`
- Benchmark: `scripts/benchmark-stack.sh` through SSH tunnels to `lat2`

## Slice Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| #2 repo scaffold context ADRs | AFK | implemented-main | none | none | `scripts/check-static.sh` |
| #3 SearXNG Latitude compose | AFK | deployed-on-lat2 | none | none | `scripts/check-static.sh`; `scripts/smoke-searxng.sh` |
| #4 Firecrawl wrapper | AFK | deployed-hardened-lat2 | none | none | `scripts/check-static.sh`; `scripts/smoke-firecrawl.sh`; API readiness healthcheck |
| #5 Hermes integration notes | AFK | proven-lat2 | none | none | Hermes web tool-layer proof; `hermes -z --toolsets web` |
| #6 Tinfoil follow-up | AFK | evaluated-lat2 | none | none | `docs/tinfoil-evaluation-2026-07-01.md` |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| None | | | | |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| bootstrap scaffold | empty repo | current thread | latest pushed `main` commit containing this ledger row | fallback review passed | `scripts/check-static.sh`; `scripts/doctor.sh lat2`; service smokes; Hermes proof; benchmark |

## Completion Audit

Audit date: 2026-07-01

- Branch/worktree: clean `main`, pushed to `origin/main`.
- GitHub: issues `#1` through `#6` closed; latest main check passed.
- Static checks: `scripts/check-static.sh` passed.
- Latitude preflight: `scripts/doctor.sh lat2` passed with Docker active and
  Compose available.
- Remote containers: SearXNG and Firecrawl API were healthy; Firecrawl queue,
  browser, Redis, Postgres, RabbitMQ, and FoundationDB services were running.
- Live smokes through fresh SSH tunnel:
  - `searxng smoke ok: 155 results`
  - `firecrawl smoke ok: 180 markdown chars`
- Live benchmark through fresh SSH tunnel: search `3/3`, extract `3/3`.
- Hosted comparison: `docs/benchmark-comparison-2026-07-01.md` records latency,
  reliability, failure-mode, and dated hosted-provider cost references.
- Hermes tool-layer proof: selected `searxng` for search and `firecrawl` for
  extract, returned search results and extracted `Example Domain`.
- Hermes one-shot proof: `hermes -z --toolsets web` returned compact JSON with
  `search_ok: true` and `extract_ok: true`.

## Open Questions

- Which public or private hostname, if any, should expose the services after
  local smokes pass?
- Should Firecrawl be exposed only to Hermes over a private network, or behind
  auth for broader operator use?
- Should SearXNG get a tiny token-gated wrapper before any public or Tinfoil
  exposure?
- Should the SearXNG-only Tinfoil prototype live in this repo or a separate
  public config repo?

## Escalations

- None.
