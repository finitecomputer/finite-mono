# Finite Mono

The Finite company monorepo: every first-party product, service, protocol,
and infrastructure definition in one tree.

| Component | What it is |
|---|---|
| `finitechat/` | finitechat CLI, server, iOS app, Electron app, Hermes agent bridge, agent runtime containers |
| `finitecomputer-v2/` | finite.computer SaaS: Core control plane, dashboard, Phala runner, Finite Private limiter |
| `finite-sites/` | fsite CLI + finitesitesd (`*.finite.chat` hosting) |
| `finite-brain/` | fbrain CLI + FiniteBrain server |
| `finite-identity/`, `finite-nostr/` | active shared identity/protocol crates |
| `finite-search/` | SearXNG/Firecrawl search stack + Tinfoil bundle |
| `finite-skills/` | sole authored managed-skills baseline; immutable revisions hot-activate in compatible runtimes |
| `finite-specialization/` | Hermes capability vocabulary and safe specialization config examples |
| `devfinity/` | local integration harness (Fedimint devimint-style) |
| `infra/` | **the single deploy root**: per-host config, images, runbooks |
| `docs/` | doctrine, plan, migration log, architecture |

Read [docs/monorepo-doctrine.md](docs/monorepo-doctrine.md) for the rules
(single-repo model, component-scoped release tags, what deliberately stays
outside). New here? Start with [CONTRIBUTING.md](CONTRIBUTING.md) — local
stack in ~15 minutes.

## Everyday commands

- `just` — list all root commands and modules
- `just check` / `just fmt` / `just test` — Rust workspace gates
- `just dev up` — boot the real local SaaS, including an Apple Container Agent Runtime
- `just dev saas-smoke` — prove real launch, Hosted Web chat, and restart healing on macOS
- `just dev smoke` — portable services-only smoke + teardown (Linux CI gate)
- `just sites …`, `just search …`, `just skills …` — component modules

## Releases

Component-scoped tags on this repo: `finitechat/vX.Y.Z`, `fsite/vX.Y.Z`,
`fbrain/vX.Y.Z` (see `.github/workflows/release-*.yml`). Container images
(`finite-agent-runtime`, `finite-saas-core`, `finite-saas-dashboard`,
`finite-private-limiter`) build via workflow dispatch, digest-pinned to GHCR.
Legacy per-repo release URLs keep working via mirror releases until retired.
Compatible Finite Skills Revisions are separately promotable product artifacts:
each skills-only promotion records a new Finite Product Release manifest while
reusing unchanged service, binary, and Runtime image digests.

## More

- [Docs index](docs/README.md)
- [Migration log](docs/monorepo-migration-log.md) · [import-sync provenance](scripts/import-sync.toml)
- [Deploy root](infra/README.md)
- [Fedimint structure analysis](docs/fedimint-monorepo-structure-analysis.md)
