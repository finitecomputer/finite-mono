# Agent Instructions

This repo is a small ops and integration repo for self-hosted agent web search
and extraction.

## Working Rules

- Keep deploy changes boring and reversible.
- Prefer explicit runbooks and smoke scripts over hidden operator knowledge.
- Do not commit secrets, host-private env files, API keys, or object-storage
  credentials.
- Treat `lat2` as the first Latitude Docker target until an ADR changes that.
- Treat `smoke` / `ovh-vps-smoke` as an OVH canary host, not a Latitude host.
- Do not move services into Tinfoil until the plain Docker deployment is proven.
- Keep Firecrawl and SearXNG smokes independent so failures are easy to isolate.

## Agent skills

### Issue tracker

Issues and PRDs are tracked in GitHub Issues after the repo is published. See
`docs/agents/issue-tracker.md`.

### Triage labels

Use the default five-role triage label vocabulary. See
`docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repo: read root `CONTEXT.md` and relevant ADRs in
`docs/adr/`. See `docs/agents/domain.md`.

