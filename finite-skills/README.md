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

The current checker is only a static floor. A Runtime image change must also
prove that a new Agent Home discovers the bundled baseline, restart or image
replacement leaves an already installed baseline and user-owned skills
unchanged, and explicit sync atomically adopts the image bundle while ordinary
failures restore the prior baseline.

## Current Delivery Gap

The current v2 Runtime image bundles this tree at `/runtime/finite-skills`,
seeds `/data/agent/managed-skills/finite/current` once for a new agent, and
exposes that durable directory through Hermes `skills.external_dirs`.

The corrected `fsite` 0.4.0 Finite Sites guidance reaches a newly initialized
Agent Home automatically. An existing agent keeps the revision it was seeded
with until the user or agent runs `finite skills sync` in a Runtime image that
contains the newer tested bundle. The command replaces only
`managed-skills/finite/current`; it never rewrites the user-owned Hermes skills
directory. New skill names require Hermes `/reload-skills`, while updated
content at an existing skill path is available from the new baseline without a
Runtime reboot.

Component trees still contain historical/reference skill snapshots. They are
not deployment sources. The Finite Sites and FiniteBrain contract deltas have
been reconciled into this baseline; future component contract changes must land
here before promotion. The dashboard catalog also still has local-sibling and
GitHub fallback behavior instead of a release-bound catalog source.
