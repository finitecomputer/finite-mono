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

Real runs default to the Devfinity local Finite agent, backed by Finite
Private. The harness accepts a skill-specific key override, then follows the
same local key convention as devfinity:
`FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY`, then the general
`FINITE_PRIVATE_API_KEY`, a cached key under `DEVFINITY_STATE_DIR`, then the
default cache at
`.local-state/devfinity/credentials/finite-private-upstream.key`.

If you have not cached the key locally yet, run this once from the monorepo
root:

```sh
just dev inference-key
```

Then generate artifacts with the product-accurate local Finite/Hermes agent:

```sh
just skills ab-test
```

The default Devfinity runner uses the Apple Container SaaS profile, so it needs
the same host prerequisites as `just dev saas-smoke` (`container system start`
on a supported Apple silicon macOS host). Set
`SKILL_AB_DEVFINITY_DOCKER_RUNTIME=1` to use Devfinity's Docker runtime profile
for disposable local runs.

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
2. The selected runner sends the same build prompt to both variants and writes
   the resulting single-file HTML artifact under `runs/latest/artifacts`.
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

For an accurate product skill test, use the `Devfinity local Finite agent`
runner and turn off `Mock`. That runner starts a disposable Devfinity SaaS
stack for each variant, installs a managed-skills tree containing exactly one
selected skill, restarts Hermes inside that local runtime, sends the build task
through the Hosted Web chat path, and copies back the generated HTML from the
runtime workspace.

The Devfinity prompt does not include either skill file, either skill path, the
variant label, or any A/B wording. Isolation is provided by the runtime's
managed-skills tree and disposable Devfinity state, not by prompt injection.

The `Isolated Codex proxy` runner is useful when you want a faster isolated
agent subprocess, but it is not the Finite product runtime. The
`Direct Finite Private` runner is only a model-call approximation: it pastes
the selected skill into a direct provider prompt and bypasses Hermes, chat,
tools, managed-skills discovery, and runtime workspace behavior. Mock mode is
only for checking the harness UI.

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

Runner settings:

- `SKILL_AB_RUNNER=devfinity`: product-accurate local Finite/Hermes agent via
  Devfinity.
- `SKILL_AB_RUNNER=agent`: isolated Codex proxy with one configured skill.
- `SKILL_AB_RUNNER=provider`: direct Finite Private/OpenAI model call.
- `SKILL_AB_DEVFINITY_DOCKER_RUNTIME=1`: use Devfinity's Docker runtime profile
  instead of the default Apple Container profile.
- `SKILL_AB_DEVFINITY_TIMEOUT_MS=1800000`
- `SKILL_AB_AGENT_MODEL`: optional model override for isolated Codex agents.
- `SKILL_AB_AGENT_TIMEOUT_MS=600000`

Direct-provider defaults:

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
- `SKILL_AB_MAX_CONCURRENCY`: forwarded to Promptfoo as `--max-concurrency`.
  Devfinity runs default to `1`.
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
