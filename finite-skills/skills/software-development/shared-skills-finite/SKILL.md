---
name: shared-skills-finite
description: Explain and safely handle requests to share, publish, update, install, pull, or sync team-authored Hermes skills on Finite without invoking retired skill-management commands or changing the Finite-managed baseline.
---

# Shared Skills On Finite

Do not pretend a team/shared skill distribution workflow exists. The current
SaaS baseline has two real skill ownership areas:

- Finite-managed baseline skills, authored in
  `finite-mono/finite-skills/skills` and seeded for a new Agent Home;
- user-local Hermes skills, authored in the user's normal writable skill area.

The explicit, one-shot managed-baseline command is available as
`finite skills sync`. It adopts only the tested `/runtime/finite-skills` bundle
from the currently running Runtime image. It is not a team source discovery or
team pull mechanism. There is no automatic rollout, background polling, Runner
operation, or Runtime reboot workflow for shared skills.

## Route The Request

### Local Experiment Or One Agent

Create or update a user-local skill under the normal Hermes skill directory.
Keep its helper scripts and references inside the skill folder, give an
experiment a distinct name, and test it with harmless inputs. Never edit the
mounted Finite-managed baseline in place.

### Finite-Managed Platform Skill

When working in the Finite monorepo as a platform developer, edit the canonical
skill under `finite-skills/skills`, follow `finite-skills/skills/AGENTS.md`, and
run:

```sh
just skills check
```

Do not edit a component-owned copy, Runtime checkout, deployed Agent Home, or
distribution mirror as the source of truth.

### Team Distribution Or Existing-Agent Sync

For an existing agent adopting the tested Finite-managed baseline already
bundled in its current Runtime image, run:

```sh
finite skills sync
```

The command reports the installed tree digest. If it adds or removes skill
names, ask the user to invoke Hermes `/reload-skills`; updated content at an
existing path is available without a Runtime reboot. For team-authored skills,
report the missing distribution capability plainly. Cloning source does not
install or activate it, and managed sync must never be used to overwrite a
user-local skill.

## Guardrails

- Skills are executable product input, not a secrets vault.
- Never commit `.env` files, API keys, OAuth tokens, cookies, session files,
  identity keys, or private credentials.
- Preserve user-local skills and intentional overrides.
- Do not couple skills delivery to Core desired state, a Runner adapter, image
  replacement, or reboot.
- Do not claim a managed baseline update reached an existing agent until
  `finite skills sync` succeeds in that agent's Runtime.
