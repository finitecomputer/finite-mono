---
name: git-finite
description: Use Finite Sites Project Repositories for source-only or published Git projects on Finite. Use when creating, listing, cloning, editing, granting access to, or pushing a Finite-hosted repository with fsite and ordinary Git.
---

# Git On Finite

The supported Finite-hosted Git surface is a Finite Sites Project Repository.
Use `fsite` for Project and credential operations, then ordinary Git for the
working tree. Do not scrape credentials or call a retired repository wrapper.

Read `finite-sites-publishing-finite` when the Project serves a site, document,
or stateful app. A source-only repository uses the same model with no declared
Project Output.

## Create A Source-Only Project

Create `finite.toml`:

```toml
[project]
slug = "notes-project"
```

Validate, create, and inspect it:

```sh
fsite auth register --output json
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
fsite project status notes-project --output json
```

Add a Project Output later by adding an `[outputs.<id>]` section and replaying
`project init` with the same Project Slug.

## List And Clone

```sh
fsite project list --output json
fsite auth git notes-project --store --output json
git clone https://git.finite.chat/notes-project.git
```

Prefer `--store`; do not print Git Credential passwords into chat or logs.

## Work With Git

```sh
git -C notes-project status --short --branch
git -C notes-project add .
git -C notes-project commit -m "Add notes"
git -C notes-project push -u origin HEAD:main
```

Use a repository-local Git author identity when Git requires one. Never infer
or expose a credential as author metadata.

## Collaborators

Use the Project owner identity to grant or revoke edit access:

```sh
fsite project grant notes-project --email editor@example.com --send-invite --output json
fsite project revoke notes-project --email editor@example.com --output json
```

An email-only collaborator verifies the email, mints a scoped credential, and
then uses ordinary Git:

```sh
fsite auth login editor@example.com
fsite auth redeem editor@example.com TOKEN_FROM_EMAIL --output json
fsite auth git notes-project --email editor@example.com --store --output json
```

Project Repository edit access is separate from viewer access to a served
Project Output. Do not make a repository or output public merely to collaborate
or preview. Public-read repository policy for selected Finite-owned baselines
is an operator concern, not a general user command.

## Guardrails

- Use Project-scoped credentials and ordinary Git.
- Keep `.env*`, `.finite/`, private keys, tokens, and build caches out of Git.
- Never reconstruct source from a served website; clone its Project Repository.
- Do not claim team skill installation from a Project Repository. Skill
  distribution has a separate product contract.
