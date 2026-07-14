# Finite Skills Audit

Status: PROPOSED

Date: 2026-07-13

## Catalog descriptions

The monorepo contains 47 `finite-skills/skills/**/SKILL.md` entries. Every entry
has a description, but two valid YAML descriptions use block-scalar syntax:
`tufte-viz-finite` uses `|` and `llm-wiki-finite` uses `>`. The dashboard's
minimal frontmatter reader returned the marker rather than its indented text;
an empty `description:` would likewise suppress the body fallback.

The branch teaches the reader folded and literal block scalars and ignores
empty scalar values. The catalog test now proves a folded multi-line
description renders as one useful sentence. No skill frontmatter needed to be
invented.

## Which tree the dashboard shows

The catalog source order is:

1. `FC_FINITE_SKILLS_SOURCE_DIR`, when set;
2. a sibling `finite-skills/` checkout;
3. `finitecomputer/finite-skills` on GitHub.

The local monorepo hits the sibling tree and enumerates the real 47-skill
baseline. The production infra definitions do not set
`FC_FINITE_SKILLS_SOURCE_DIR`, and the packaged dashboard does not establish a
sibling monorepo checkout contract, so production falls back to the archived
repository. This can make the dashboard disagree with `/runtime/finite-skills`
inside the Agent image. The existing 2026-07-10 parking-lot entry already names
the infra fix. Changing production env or image layout is CREEP for this run.

## Runtime-reference sweep

All 47 skill entrypoints were searched for managed-skill paths, venv paths,
retired repository references, and install commands. Findings:

- Google Workspace claimed no binary existed even though this branch now pins
  `gws` 0.22.5. Its description and execution guidance now match the image.
- `meme-from-template-finite` hard-coded `~/.hermes/venv`, but the hosted image
  sets `HERMES_HOME=/data/agent/hermes-home` and places
  `/runtime/hermes-venv/bin` on `PATH`. It now uses `python3` and correctly says
  Pillow is preinstalled.
- `fal-image-editing-finite` instructed the Agent to grep and hardcode a secret
  from `~/.hermes/.env`. That unsafe, misplaced fallback is removed; the skill
  now fails closed for a missing managed credential and uses the managed Python
  on `PATH`. `fal-client` remains absent and is not baked because this run's
  dependency fence excludes it.
- `x-search-finite` claimed `xai-sdk` was bootstrapped, but an image probe found
  it absent. It also used the retired
  `/profile-assets/hermes-local/managed-skills` path. The claim is now honest
  and its helper path uses the current Finite managed-skill root.
- The same retired `/profile-assets/hermes-local/managed-skills` prefix remains
  in Linear, X API, arXiv, Google Places, Polymarket, Perplexity, and the
  trading aggregator. Converting these repeated command blocks safely is a
  follow-up skill-text cleanup; the correct hosted prefix is
  `${FINITECHAT_HOME:-/data/agent}/managed-skills/finite/current`.
- `generate-pdf-finite` and `trading-agent-finite` still source
  `~/.hermes/venv`; Notion and Linear describe credentials under
  `~/.hermes/.env`; `find-nearby-finite` retains a sync note naming the archived
  repository. These are documentation drift, not evidence that the named paths
  exist in the mono runtime.
- Runtime install instructions remain for the PDF/document group,
  `fal-client`, `ddgs`, mutable `blogwatcher@latest`, and Parallel CLI variants.
  Their bake/no-bake disposition is in the preinstall audit; none was added by
  this item.

The unfixed repeated-path set is bounded documentation debt. No nonexistent
binary, path, or dependency is asserted as available by this audit.
