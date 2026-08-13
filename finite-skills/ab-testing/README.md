# Skill A/B Testing Harness

Local Promptfoo + Playwright harness for comparing the rendered output of two
web-design-oriented agent skills with a human spot-check review page.

This is intentionally not a statistical product experiment. It answers a
practical local question: "given the same build prompt, what does an agent
produce when guided by each skill, and which result is more useful?"

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

Real runs default to Finite Private. The harness accepts a skill-specific key
override, then follows the same local key convention as devfinity:
`FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY`, then the general
`FINITE_PRIVATE_API_KEY`, a cached key under `DEVFINITY_STATE_DIR`, then the
default cache at
`.local-state/devfinity/credentials/finite-private-upstream.key`.

If you have not cached the key locally yet, run this once from the monorepo
root:

```sh
just dev inference-key
```

Then generate artifacts:

```sh
just skills ab-test
```

To test one custom build prompt without editing `promptfooconfig.yaml`:

```sh
just skills ab-test-prompt 'Build a first screen for a browser dashboard that helps a renewable-energy operations team scan turbine health, open incidents, weather risk, and dispatch status.'
```

To verify the harness without API calls:

```sh
just skills ab-test-mock
```

The review page is written to:

```text
runs/latest/review/index.html
```

Serve it with the local edit/regenerate API:

```sh
just skills ab-serve
```

Then open:

```text
http://127.0.0.1:8787/review/index.html
```

You can also open the static file directly, but prompt/skill editing and
regeneration require the local server.

To open the static file:

```sh
cd finite-skills/ab-testing
pnpm run open
```

## What It Does

1. Promptfoo runs every brief in `promptfooconfig.yaml` against two provider
   variants.
2. Each provider gives the model the selected `SKILL.md` as installed-agent
   guidance, sends the same build prompt as the user task, and writes the
   resulting single-file HTML artifact under `runs/latest/artifacts`.
3. Playwright opens every generated HTML page at desktop and mobile viewports
   and captures screenshots.
4. `scripts/build-review.mjs` creates a side-by-side human review page with
   links to the generated HTML, raw model output, and exact prompt.

The review page is for judging rendered output. The skill text is saved only in
`prompt.txt` so runs are reproducible.

The review page stores winner picks and notes in browser local storage and can
export them as JSON.

When served through `pnpm run serve`, the review page also lets you edit the
build prompt, choose a different `SKILL.md` file for either variant, edit the
loaded skill text, and regenerate from the browser. Edited skills are saved
under `runs/editable/`; the source `../skills/.../SKILL.md` files are not
changed.

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

Provider defaults:

- `SKILL_AB_PROVIDER=auto`
- Finite Private is used when a local key is present.
- OpenAI is used only when no Finite Private key is found and `OPENAI_API_KEY`
  is set, or when `SKILL_AB_PROVIDER=openai` is set.

Finite Private settings:

- Key env vars: `SKILL_AB_FINITE_PRIVATE_KEY`,
  `FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY`, or `FINITE_PRIVATE_API_KEY`
- Key file override: `SKILL_AB_FINITE_PRIVATE_KEY_FILE`
- Devfinity state override: `DEVFINITY_STATE_DIR`
- Base URL: `SKILL_AB_FINITE_PRIVATE_BASE_URL`, `FINITE_PRIVATE_BASE_URL`, or
  `FC_RUNNER_FINITE_PRIVATE_BASE_URL`
- Default base URL: `https://kimi-k2-6.finite.containers.tinfoil.dev/v1`
- Default model: `glm-5-2`

Common settings:

- `SKILL_AB_MODEL`
- `SKILL_AB_MAX_OUTPUT_TOKENS=6000`
- `SKILL_AB_TIMEOUT_MS=120000`
- `SKILL_AB_MAX_CONCURRENCY`: forwarded to Promptfoo as `--max-concurrency`
- `SKILL_AB_REPAIR_HTML=0`: disables the automatic second pass that converts
  non-HTML model output into a reviewable HTML artifact

Override them as environment variables:

```sh
SKILL_AB_MODEL=glm-5-2 SKILL_AB_MAX_CONCURRENCY=1 pnpm run ab
```

OpenAI is still available as an explicit fallback:

```sh
SKILL_AB_PROVIDER=openai OPENAI_API_KEY=... SKILL_AB_MODEL=gpt-5-mini pnpm run ab
```

## Adding Cases

Add more entries under `tests` in `promptfooconfig.yaml`. Each case should
include:

- `caseId`: stable filesystem-safe identifier
- `title`: human-readable title
- `brief`: the design task given to both skill variants

Keep cases small enough that you can review every screenshot. For this harness,
human attention is the scoring mechanism.
