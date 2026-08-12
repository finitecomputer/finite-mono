# Skill A/B Testing Harness

Local Promptfoo + Playwright harness for comparing two web-design-oriented
agent skills with a human spot-check review page.

This is intentionally not a statistical product experiment. It answers a
practical local question: "given the same design brief, which skill produces
the more useful first-pass web UI artifact?"

## Setup

Use the repo dev shell or root `just` recipes; current Promptfoo requires
Node 22.22 or newer, and the pinned dev shell supplies a compatible Node.

From the monorepo root:

```sh
just skills ab-setup
```

Equivalent direct commands from this directory, inside the dev shell:

```sh
pnpm install --frozen-lockfile
pnpm run browsers
```

Real runs use the OpenAI Responses API:

```sh
export OPENAI_API_KEY=...
just skills ab-test
```

To verify the harness without API calls:

```sh
just skills ab-test-mock
```

The review page is written to:

```text
runs/latest/review/index.html
```

Open it directly, or run:

```sh
cd finite-skills/ab-testing
pnpm run open
```

## What It Does

1. Promptfoo runs every brief in `promptfooconfig.yaml` against two provider
   variants.
2. Each provider loads one `SKILL.md`, asks the model for a complete
   single-file HTML page, and writes the artifact under `runs/latest/artifacts`.
3. Playwright opens every generated HTML page at desktop and mobile viewports
   and captures screenshots.
4. `scripts/build-review.mjs` creates a side-by-side human review page with
   links to the generated HTML, raw model output, and exact prompt.

The review page stores winner picks and notes in browser local storage and can
export them as JSON.

## Changing The Skills

The default variants compare:

- `skill-a`: `../skills/software-development/website-building-finite/SKILL.md`
- `skill-b`: `../skills/software-development/impeccable-finite/SKILL.md`

Edit `promptfooconfig.yaml` to change labels or paths.

For quick one-off overrides without editing the config:

```sh
SKILL_AB_SKILL_A_PATH=../skills/software-development/website-building-finite/SKILL.md \
SKILL_AB_SKILL_B_PATH=../skills/software-development/impeccable-finite/SKILL.md \
pnpm run ab
```

## Model Settings

Defaults:

- `SKILL_AB_MODEL=gpt-5-mini`
- `SKILL_AB_MAX_OUTPUT_TOKENS=6000`
- `SKILL_AB_TIMEOUT_MS=120000`

Override them as environment variables:

```sh
SKILL_AB_MODEL=gpt-5-mini pnpm run ab
```

## Adding Cases

Add more entries under `tests` in `promptfooconfig.yaml`. Each case should
include:

- `caseId`: stable filesystem-safe identifier
- `title`: human-readable title
- `brief`: the design task given to both skill variants

Keep cases small enough that you can review every screenshot. For this harness,
human attention is the scoring mechanism.
