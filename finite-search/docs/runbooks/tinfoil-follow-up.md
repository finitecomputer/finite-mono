# Tinfoil Follow-Up Runbook

Tinfoil is a later packaging target, not the first deploy target.

## Prerequisites

Before attempting Tinfoil:

- Plain Docker deployment passes on `lat2`.
- Resource usage is measured for both services.
- Image build inputs are pinned.
- Public `tinfoil-config.yml` constraints are understood.
- Secrets and state handling are documented.
- The team decides whether SearXNG, Firecrawl, or only one of them belongs in
  Tinfoil.

## Open Questions

- Does Firecrawl's browser/queue stack fit the Tinfoil container shape?
- Should SearXNG be Tinfoil-packaged independently because it is smaller?
- Do these services need durable state, or can they be stateless with external
  caches disabled?
- What network exposure is acceptable for web scraping from an enclave?
- Are any API keys needed for proxies, AI features, or private extraction
  backends?

## First Tinfoil Candidate

The likely first candidate is SearXNG alone, because it has a smaller runtime
surface than Firecrawl.

Firecrawl should wait until the plain Docker deploy proves which upstream image,
compose profile, queue backend, and browser settings are actually needed.

## 2026-07-01 Evaluation Result

Plain Docker now passes on `lat2` for both services. Based on the live deploy:

- SearXNG is the first candidate.
- Firecrawl is a later candidate.
- The combined stack is not a first Tinfoil target.

Key blockers before a Tinfoil prototype:

- `tinfoil-config.yml` must be public and must pin image digests.
- Tinfoil containers have no persistent disk.
- Tinfoil containers are public-inbound through the shim.
- Search/extract workloads need explicit outbound egress; general scraping
  probably needs `egress: open`.
- Any production endpoint needs auth or a wrapper before public exposure.

## 2026-07-01 SearXNG Prototype Bundle

The SearXNG-only prototype bundle now lives at:

```text
tinfoil/searxng-public/
```

It includes:

- a pinned-base Dockerfile;
- a baked SearXNG `settings.yml` with JSON enabled;
- root-ready `tinfoil-config.yml`;
- Tinfoil release workflows for the future public repo; and
- a local Docker smoke script:
  `scripts/smoke-tinfoil-searxng-bundle.sh`.

Next action: create a separate public repo, copy the bundle into its root,
release `v0.0.1`, deploy in Tinfoil staging/debug, and smoke through Tinfoil's
verified client/proxy. Do not use plain `curl` as proof of Tinfoil attestation.
