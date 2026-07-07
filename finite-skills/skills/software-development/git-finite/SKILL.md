---
name: git-finite
description: Use when the user wants to create, clone, inspect, share, or push private repos on Finite's built-in Git host, or hand an outside developer an HTTPS clone URL plus token-based access.
---

# Git on Finite

Use this skill for the Finite Git host and shared Git collaboration. Prefer `finitec repo ...` over ad hoc token scraping or hand-built authenticated Git commands.

## First principles

- Repos are private by default.
- The machine-local Git identity and token are already injected into `~/.hermes/.env` when the machine has Git access.
- `finitec repo` is the supported wrapper for machine-owned repo operations.
- Raw Git over HTTPS is still the underlying transport. `finitec repo git` injects the machine's HTTPS auth header for you.
- Shared/team skills use the `shared-skills-finite` skill. This skill covers the lower-level Git host commands.

## Core workflow

### List repos

```bash
finitec repo list
```

Repo listings include the current access mode plus any pending collaborator emails that have not logged into the Git web UI yet.

### Create a repo

```bash
finitec repo create --name notes-app
```

Repos are empty by default so the first clone and push stay predictable.

### Clone a repo

```bash
finitec repo clone --name notes-app
```

Default destination: `~/dev/REPO_NAME`

### Work with normal Git commands

```bash
finitec repo git -- -C ~/dev/notes-app status --short --branch
finitec repo git -- -C ~/dev/notes-app add .
finitec repo git -- -C ~/dev/notes-app commit -m "Add draft"
finitec repo git -- -C ~/dev/notes-app push -u origin HEAD:main
```

Before the first commit in a new checkout, set a local identity if Git asks for one:

```bash
git -C ~/dev/notes-app config user.name "$FC_GITEA_USERNAME"
git -C ~/dev/notes-app config user.email "${FC_GITEA_USERNAME}@gitea.local.invalid"
```

## Shared Skills

If the human is trying to publish, update, pull, or install a team-shared Hermes
skill, use `shared-skills-finite` instead of this lower-level Git workflow. The
old `finitec repo bootstrap-shared-skills` and `~/shared-skills` flow is retired.

## Repo permissions

Use `finitec repo permissions` instead of sending people into Gitea settings:

```bash
finitec repo permissions --name notes-app view
finitec repo permissions --name notes-app emails --email austin@finite.vip
finitec repo permissions --name notes-app org --org-domain finite.vip
finitec repo permissions --name notes-app self
finitec repo permissions --name notes-app public --confirm-public "MAKE PUBLIC"
```

- `self` keeps the repo private to the machine owner.
- `emails` grants write access to explicit collaborator emails after those users have logged into the Git web UI once.
- `org` grants write access to everyone whose Gitea account email matches the org domain.
- `public` makes the repo anonymously cloneable. Pushes still require normal auth.

## Outside developers

- Outside developers should use the repo's HTTPS clone URL plus a Gitea personal access token.
- If a collaborator email is still pending, ask them to sign into the Git web UI once so Finite can map that email to a Gitea account.
- After that, give them the HTTPS clone URL and tell them to authenticate with a Gitea PAT.

## Guardrails

- Do not expose Git credentials directly when `finitec repo ...` can do the job.
- Do not make repos public unless the human explicitly asks for that.
- Prefer `finitec repo git -- -C PATH ...` over inventing manual `Authorization` headers.
- If a shared skill is only an experiment, use a new skill name rather than shadowing a managed `-finite` skill immediately.
