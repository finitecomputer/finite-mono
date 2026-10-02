# Finite Skills

Finite-managed baseline skills for deployed Hermes agents.

`finite-mono/finite-skills/skills` is the only editable source. Do not edit an
old component repository, a runtime checkout, or a Finite Sites mirror and try
to sync it back.

Every Runtime image bundles a tested snapshot from this tree. When a genuinely
new agent initializes, the common gateway launcher copies that baseline once to
its durable Agent Home and configures Hermes to discover it. Restarting or
replacing the Runtime image does not overwrite the installed baseline.

Existing agents update at their own pace with the explicit, agent-local
`finite skills sync` command. It adopts the tested `/runtime/finite-skills`
bundle from the Runtime image that is already running; it does not fetch from
GitHub or a platform service. There is no Core desired revision, automatic
updater, polling loop, Runtime Management Pipe command or status, or
Runner-managed skills checkout.

User-local skills stay in the normal writable Hermes skill directory. They are
durable user data, may intentionally override a baseline name, and must never be
rewritten or pruned by platform updates. Team/shared skills are a separate
future source, not content to mix into this global baseline.

## Editing And Validation

Add or change a deployed skill here first, keep all of its helpers inside the
skill directory, and use `${HERMES_SKILL_DIR}` for relocatable helper paths.
Run from the monorepo root:

```sh
just skills check
```

The Runtime image workflow also runs `scripts/check-sites-cli.py` against its
actual `fsite` binary and bundled skills. This checks the release version,
advertised workflows, config examples, and rejection of retired workflows.
It is an offline contract check; live publishing remains a rollout check.

For local human spot checks of web-design skill variants, use the Promptfoo +
Playwright harness in `ab-testing/`:

```sh
just skills ab-setup
just skills ab-test
```

Run `just dev inference-key` once if the local Finite Private upstream key has
not been cached yet, then run `just skills ab-test` for product-accurate local
Devfinity/Hermes artifacts. Use `just skills ab-test-prompt 'Build ...'` to
compare both skill variants on one custom build prompt. Run
`just skills ab-serve` to edit the prompt and both skill variants from the
browser and regenerate locally. The root `just` path enters the pinned dev
shell, which provides a Node version compatible with current Promptfoo. Use
`SKILL_AB_RUNNER=provider` for the faster direct Finite Private approximation,
or `SKILL_AB_PROVIDER=openai OPENAI_API_KEY=...` only when intentionally
testing against OpenAI instead of Finite Private.

The current checker is only a static floor. A Runtime image change must also
prove that a new Agent Home discovers the bundled baseline, restart or image
replacement leaves an already installed baseline and user-owned skills
unchanged, and explicit sync atomically adopts the image bundle while ordinary
failures restore the prior baseline.

## Current Delivery Gap

The current v2 Runtime image bundles this tree at `/runtime/finite-skills`,
seeds `/data/agent/managed-skills/finite/current` once for a new agent, and
exposes that durable directory through Hermes `skills.external_dirs`.

The corrected `fsite` 0.6.0 Finite Sites guidance reaches a newly initialized
Agent Home automatically. An existing agent keeps the revision it was seeded
with until the user or agent runs `finite skills sync` in a Runtime image that
contains the newer tested bundle. The command replaces only
`managed-skills/finite/current`; it never rewrites the user-owned Hermes skills
directory. Updated content at an existing skill path is available when the
agent rereads it. Changes to names or index descriptions require an authorized
gateway restart to clear the process cache; `/reload-skills` alone does not
refresh that index. Follow the verification steps below.

Component trees still contain historical/reference skill snapshots. They are
not deployment sources. The Finite Sites and FiniteBrain contract deltas have
been reconciled into this baseline; future component contract changes must land
here before promotion. The dashboard catalog also still has local-sibling and
GitHub fallback behavior instead of a release-bound catalog source.

## Correcting Guidance On Existing Agents

A merged skill correction reaches an existing agent only after all of these
steps. Stop at the first one that fails and record where the rollout stands.

1. Merge the correction to `main`.
2. Build and promote a Runtime image from a revision that contains it, per
   `infra/runbooks/runtime-image.md` sections 2 through 4. Record the bundled
   Finite Skills revision beside the image.
3. Move the agent's Runtime to that image with an explicit Runtime Upgrade
   (`infra/runbooks/runtime-image.md` section 4a). The upgrade preserves
   `/data`, so the old managed baseline is still in place afterwards.
4. Have the user ask the agent to run `finite skills sync`, or run it in the
   agent's Runtime with the user's agreement. It prints the adopted tree as
   `sha256:<digest>`.
5. Verify from the agent's Runtime:

   ```sh
   diff -rq -x __pycache__ /runtime/finite-skills /data/agent/managed-skills/finite/current
   ```

   No output means the managed baseline matches the image. Also hash any file
   the correction changed and compare it with `git show <revision>:<path> |
   sha256sum`.
6. Have the agent reread the corrected skill body before retesting in a new
   conversation. `skill_view` reads an existing path from disk. Hermes also
   caches its skills index in the gateway process: a new conversation or
   `/reload-skills` alone does not clear that cache. If skill names or index
   descriptions changed, arrange an authorized gateway restart and verify
   the new index. This correction retains the existing name and description.
7. Leave user-owned skills in `$HERMES_HOME/skills` as they are. When one
   contradicts the corrected guidance, give the user its name, path, and the
   conflicting sentence; the user decides whether to edit or remove it.

The updated `fbrain --skill` guide is available after step 3. Existing Brain
Working Tree `AGENTS.md` files regenerate on the next successful open or sync
with authoritative Brain metadata from the updated binary. Inspect the
regenerated file before claiming that its instructions have been updated.
