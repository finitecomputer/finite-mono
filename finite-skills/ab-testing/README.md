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
When running from a Git worktree, it also checks the same Devfinity key cache
in the other worktrees listed by Git, which lets Codex worktrees use the key
cached in your main local checkout.

If you have not cached the key locally yet, run this once from the monorepo
root:

```sh
just dev inference-key
```

Start the local Devfinity stack once:

```sh
just dev up --headless
```

Then, in another terminal, generate artifacts with the product-accurate local
Finite/Hermes agent:

```sh
just skills ab-test
```

The default Devfinity runner is now a client of that already-running stack. The
stack profile is whatever you started with `just dev up`.

To test one custom build prompt without editing `promptfooconfig.yaml`:

```sh
just skills ab-test-prompt 'Build a first screen for a browser dashboard that helps a renewable-energy operations team scan turbine health, open incidents, weather risk, and dispatch status.'
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

When served through `pnpm run serve`, the review page lets you edit the build
prompt, choose a different `SKILL.md` file for either variant, edit the loaded
skill text, and click Generate. Generate runs the configured real runner from
the browser UI; it does not use synthetic previews. The browser server defaults
to `SKILL_AB_REVIEW_RUNNER=provider` with
`SKILL_AB_REVIEW_PROVIDER=finite-private` so local spot checks work against
Finite Private without starting a Devfinity stack. Start the server with
`SKILL_AB_REVIEW_RUNNER=devfinity` when you want the product-accurate local
Finite/Hermes agent path. Edited skills are saved under `runs/editable/`; the
source `../skills/.../SKILL.md` files are not changed.

For the most product-accurate skill test, run the browser server with
`SKILL_AB_REVIEW_RUNNER=devfinity` or run the CLI with
`SKILL_AB_RUNNER=devfinity`. The default Devfinity mode calls
`devfinity agent-run` against an already-running stack. For each variant, the
harness stages a source skill directory with the edited `SKILL.md`, asks
Devfinity to create a short-lived local runtime, copies that directory into a
one-skill managed bundle inside that runtime, restarts Hermes, sends the build
task through the Hosted Web chat path, and captures the generated single-file
HTML from the agent's final response. If the runtime also writes the diagnostic
HTML path, the harness copies that file instead. After capture, the driver asks
Core to stop the runtime and removes the local container. Set
`SKILL_AB_DEVFINITY_KEEP_RUNTIME=1` when debugging a generated runtime.

Client mode still serializes Devfinity agent-run calls with a local lock even
when Promptfoo concurrency is higher. The shared server stack has one local
runtime port, so the clean-room boundary is a fresh short-lived runtime per
variant rather than parallel runtimes in the same stack.

Set `SKILL_AB_DEVFINITY_MODE=disposable` to use the older behavior where each
variant starts its own disposable Devfinity SaaS stack. In that mode, each run
receives its own Devfinity port offset, Apple container name prefix,
host-network probe container name, and local runtime image tag so parallel runs
and retries do not share local ports, dashboard build state, or runtime
container names. The harness records every disposable `--state-dir` under
`runs/`, runs `devfinity --state-dir ... cleanup` in provider `finally` and
signal paths, and sweeps previously tracked temp-root state on startup before
serving or generating another run.

The Devfinity prompt does not include either skill file, either skill path, the
variant label, or any A/B wording. In client mode, separation is provided by a
fresh local runtime, a staged one-skill managed bundle, and a fresh chat topic
per variant. Disposable mode adds a separate Devfinity state root per variant.

The `Isolated Codex proxy` runner is useful when you want a faster isolated
agent subprocess, but it is not the Finite product runtime. The
`Direct Finite Private` runner is only a model-call approximation: it pastes
the selected skill into a direct provider prompt and bypasses Hermes, chat,
tools, managed-skills discovery, and runtime workspace behavior.

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
- `SKILL_AB_DEVFINITY_MODE=client`: default; reuse the already-running
  Devfinity stack through `devfinity agent-run`.
- `SKILL_AB_DEVFINITY_MODE=disposable`: legacy mode; start and clean up a
  disposable Devfinity stack per variant.
- `SKILL_AB_DEVFINITY_STATE_DIR=.local-state/devfinity`: state root for client
  mode. A `runs/default` path from Devfinity's generated env is also accepted.
- `SKILL_AB_DEVFINITY_DOCKER_RUNTIME=1`: in disposable mode, use Devfinity's
  Docker runtime profile instead of the default Apple Container profile.
- `SKILL_AB_DEVFINITY_TIMEOUT_MS=1800000`
- `SKILL_AB_DEVFINITY_REPLY_TIMEOUT_MS=1200000`: wait up to 20 minutes for
  the in-runtime Hermes chat turn to produce a final response.
- `SKILL_AB_DEVFINITY_READINESS_TIMEOUT_MS=180000`: wait up to 3 minutes for
  hosted chat binding recovery, connected chat state, and owner claim.
- `SKILL_AB_DEVFINITY_KEEP_RUNTIME=1`: keep the short-lived local runtime after
  the run for debugging instead of stopping and deleting its container.
- `SKILL_AB_DEVFINITY_LOCK_TIMEOUT_MS=1800000`: maximum wait for the client-mode
  agent-run lock.
- `SKILL_AB_DEVFINITY_STATE_ROOT`: in disposable mode, override the temp root
  used for per-variant Devfinity state.
- `SKILL_AB_DEVFINITY_CLEANUP_TIMEOUT_MS=120000`: per-state-dir cleanup cap.
- `SKILL_AB_AGENT_MODEL`: optional model override for isolated Codex agents.
- `SKILL_AB_AGENT_TIMEOUT_MS=600000`

Direct-provider modes:

- `SKILL_AB_PROVIDER=finite-private`: default; uses the local Finite Private
  key and the deployed Finite Private streaming chat-completions endpoint.
  The current shim disables DeepSeek thinking for artifact generation because
  the active rollout otherwise spends the response budget on reasoning.
- `SKILL_AB_PROVIDER=openai`: explicit toggle for OpenAI's Responses API.
- There is no automatic fallback between providers; choose the provider you
  want to test.

Finite Private settings:

- Key env vars: `SKILL_AB_FINITE_PRIVATE_KEY`,
  `FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY`, or `FINITE_PRIVATE_API_KEY`
- Key file override: `SKILL_AB_FINITE_PRIVATE_KEY_FILE`
- Devfinity state override: `DEVFINITY_STATE_DIR`
- Base URL: `SKILL_AB_FINITE_PRIVATE_BASE_URL`, `FINITE_PRIVATE_BASE_URL`, or
  `FC_RUNNER_FINITE_PRIVATE_BASE_URL`
- Default base URL: `https://kimi-k2-6.finite.containers.tinfoil.dev/v1`
- Default model: `deepseek-v4-flash-0731`. This tracks the currently deployed
  Finite Private rollout and may need to change with the next model rollout.
- `SKILL_AB_FINITE_PRIVATE_TIMEOUT_MS=1200000`: wait up to 20 minutes for the
  current streaming model path before failing a provider call.
- `SKILL_AB_FINITE_PRIVATE_ATTEMPTS=3`: retry transient upstream errors before
  failing the provider run.

Common settings:

- `SKILL_AB_MODEL`
- `SKILL_AB_MAX_OUTPUT_TOKENS=5000`: default artifact budget. Raise it for
  richer pages once the current Finite Private rollout is no longer hitting
  long-stream termination under concurrent direct-provider calls.
- `SKILL_AB_TIMEOUT_MS=120000`
- `SKILL_AB_MAX_CONCURRENCY`: forwarded to Promptfoo as `--max-concurrency`.
  Devfinity runs default to `2`.
- `SKILL_AB_REPAIR_HTML=0`: disables the automatic second pass that converts
  non-HTML model output into a reviewable HTML artifact

Override them as environment variables:

```sh
SKILL_AB_MODEL=deepseek-v4-flash-0731 SKILL_AB_MAX_CONCURRENCY=2 pnpm run ab
```

OpenAI is available as an explicit mode:

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
