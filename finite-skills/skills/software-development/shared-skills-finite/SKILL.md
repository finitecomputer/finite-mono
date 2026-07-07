---
name: shared-skills-finite
description: Use when the human wants to share, publish, update, install, pull, sync, or use a team-shared Hermes skill from the Finite shared skills repo.
---

# Shared Skills On Finite

Use this skill when the human says things like:

- "I want to share this skill with my team."
- "Publish this skill for the other bots."
- "I was told there is a shared skill called X. Install it."
- "Pull the latest team skills."

Shared/team skills are organization- or box-scoped. They live in the configured
`user` managed skill source, not in the global Finite baseline.

## Important Paths

- Global Finite skills: `~/.finite/managed-skills/finite/current`
- Team/shared skills: `~/.finite/managed-skills/user/current`
- Managed consumer checkout: `~/.finite/managed-skills/user/repo`
- Normal editable repo checkout: `~/dev/<repo-name>`

Do not edit `~/.finite/managed-skills/user/repo` directly. That path is owned by
`finitec skills sync` and may be reset.

## Install Or Use A Shared Skill

1. Confirm shared skills are configured:

   ```bash
   finitec skills sources
   ```

   Look for a source named `user`.

2. Pull the latest team skills:

   ```bash
   finitec skills sync --source user
   ```

3. If Hermes does not see the skill immediately, run:

   ```text
   /reload-skills
   ```

4. Read the requested skill under:

   ```text
   ~/.finite/managed-skills/user/current
   ```

5. Follow the skill's own setup instructions.

If a credential is needed, do not ask the human to paste secrets into chat
unless the skill explicitly says that is the intended flow. Prefer local env or
a local setup script.

## Publish Or Update A Shared Skill

1. Confirm shared skills are configured:

   ```bash
   finitec skills sources
   ```

   If there is no `user` source, say shared/team skills are not configured on
   this machine.

2. Clone the shared skills repo into `~/dev`.

   If the current bot owns the repo:

   ```bash
   finitec repo clone --name REPO_NAME
   ```

   If another bot owns it, include that owner:

   ```bash
   finitec repo clone --name REPO_NAME --owner OWNER
   ```

   Use the `url` printed by `finitec skills sources` to infer `OWNER` and
   `REPO_NAME` when needed.

3. Add or update a normal Hermes skill directory:

   ```text
   ~/dev/REPO_NAME/skills/SKILL_NAME/SKILL.md
   ~/dev/REPO_NAME/skills/SKILL_NAME/scripts/...
   ~/dev/REPO_NAME/skills/SKILL_NAME/references/...
   ~/dev/REPO_NAME/skills/SKILL_NAME/templates/...
   ```

   Keep helper files inside the skill directory. If `SKILL.md` calls a helper
   script, reference it through `${HERMES_SKILL_DIR}/scripts/...`.

4. Test before publishing.

   At minimum:

   - read `SKILL.md` back from disk;
   - run any helper scripts with harmless inputs;
   - verify no secret values are printed;
   - verify no `.env`, token, API key, or private credential file is staged.

5. Commit and push with the Finite Git wrapper:

   ```bash
   finitec repo git -- -C ~/dev/REPO_NAME status --short
   finitec repo git -- -C ~/dev/REPO_NAME add skills/SKILL_NAME
   finitec repo git -- -C ~/dev/REPO_NAME commit -m "Add SKILL_NAME skill"
   finitec repo git -- -C ~/dev/REPO_NAME push
   ```

6. Tell the team:

   - the skill name;
   - a one-sentence usage example;
   - to run `finitec skills sync --source user`.

## Guardrails

- Shared skill repos are not secrets vaults.
- Never commit `.env` files, API keys, OAuth tokens, cookies, session files, or
  private credentials.
- Keep skill contents team-appropriate. Global Finite skills belong in
  `finitecomputer/finite-skills`, not a private team repo.
- Prefer a new skill name for experiments instead of shadowing an existing
  managed `-finite` skill.
