# Bare Repos And Skills Hosting

Status: requirements note, updated for ADR 0028.

Date: 2026-07-02

## Problem Statement

Finite Computer is cutting `finitec repo` and `finitec publish` from the new
self-serve runtime shape. Finite Sites should cover both useful behaviors:

- Project Repositories replace machine-owned `finitec repo` workflows.
- Project Sites replace `finitec publish` website workflows.
- A Finite-managed skills Distribution Mirror can live in Finite Sites instead
  of making GitHub a production runtime dependency.

The product vocabulary supports this direction: a Project Repository is the
source primitive and may exist before a public-facing site exists.

## Product Shape

### Bare Project Repository

A Bare Project Repository is a normal Project Repository with no Project Site
yet. It has:

- a Project Slug;
- owner and collaborator permissions;
- a Git Remote;
- Git Credentials;
- Project Status and Project List entries;
- audit records for ref updates;
- no viewer-facing URL and no active Version.

Agent-facing shape:

```sh
fsite describe workflow register-and-publish --output json
fsite auth register --output json
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
fsite auth git PROJECT --store --output json
git clone https://v2.finite.chat/PROJECT.git
git push origin main
```

The `finite.toml` for a Bare Project Repository is valid with only a project
section:

```toml
[project]
slug = "finite-skills"
```

### Add A Site Later

A Bare Project Repository can gain one Project Site later without changing its
Git Remote or Project Slug. The explicit mutation is to add `[site]` to
`finite.toml` and replay Project Init:

```sh
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
```

Hard constraints:

- Adding a site is idempotent when the existing site matches the config.
- Replaying with incompatible existing site settings fails deterministically.
- Pushing a branch before a site exists records Git history but creates no
  Version.
- Pushing a Deploy Branch after a site is added reconciles normally.
- Existing site config changes remain rejected until an explicit update design
  exists.

### Skills Distribution Mirror With Browsable Site

The canonical editable source is `finite-mono/finite-skills`. Release
automation can publish immutable Finite Skills Revisions into a `finite-skills`
Project Repository Distribution Mirror. Runtimes fetch an exact promoted
revision through Finite Sites, while humans and agents can browse a Project
Site generated from that same revision.

Runtime read policy decision:

- The Finite-owned baseline Distribution Mirror uses public read-only Project
  Visibility.
- Public read-only means unauthenticated `git clone` and `git fetch` are
  allowed for that selected Project Repository.
- The initial mutation surface is the operator command
  `finitesitesd project-visibility --data DIR PROJECT public-read`.
- Public read-only never permits `git push`; maintainer writes still require
  normal Project Collaborator auth and scoped Git Credentials.
- Customer, user, and team Managed Skills Repositories stay private by default.
  They use the normal Project Repository auth path today, and may later use
  Core-granted read credentials for hosted Agent Runtimes.
- Site visibility remains separate. A browsable static site can be public,
  shared, or private without changing whether runtime Git fetches are public.
- Mirror writes come only from release automation publishing a promoted
  monorepo revision. Runtime activation never follows `main`.

The clean final shape:

```toml
[project]
slug = "finite-skills"

[site]
name = "finite-skills"
branch = "main"
path = "skills"
```

The browsable shape is static files committed under the configured site path.
The monorepo remains the editable source of truth; the Project Repository and
generated HTML are distribution and browsing views.

## Requirements

- `ProjectConfig::validate` accepts no site.
- `ProjectConfig::to_toml_string` preserves a `[project]`-only config.
- `ProjectInitResponse.site` may be null.
- `fsite project init` help explains that a Project Repository may start
  without a site.
- `fsite project status` renders an absent site without implying failure.
- `fsite project status` and `fsite project list` include Project Visibility.
- `fsite project list` includes Bare Project Repositories.
- `fsite auth git PROJECT --store --output json` works for Bare Project
  Repositories.
- Git clone/fetch/push works for Bare Project Repositories.
- Git push to a branch with no matching site does not create a Version and
  does not produce a deploy failure state.
- Bare Git Remotes set `HEAD` to `refs/heads/main` so empty clones do not warn
  about a nonexistent default ref.
- Managed-skills fixtures can serve a browsable static site without making
  generated HTML the install source.

## Tests

Coverage required before product code depends on this:

- `[project]`-only config parses, validates, encodes, and round-trips.
- `project init` creates a Bare Project Repository and replays safely.
- Conflicting owner cannot initialize an existing Bare Project Repository slug.
- `auth git` works for a Bare Project Repository.
- Real git clone/push works for a Bare Project Repository.
- Pushing to a Bare Project Repository records refs but creates no Version.
- Adding a first site to a Bare Project Repository succeeds and replays.
- Replaying incompatible site config fails.
- After site add, a real git push to the Deploy Branch creates a Version.
- Project Status and Project List include source-only repositories cleanly.
- Managed-skills fixture with public Project Visibility can be cloned and
  fetched without credentials.
- Managed-skills fixture with private Project Visibility rejects anonymous
  clone and fetch.
- Anonymous push to a public-read Managed Skills Repository is rejected.
- Authenticated collaborator push to a public-read Managed Skills Repository
  still works.
- Managed-skills fixture can serve a browsable site without making generated
  HTML the install source.

## Non-Goals

- Do not add a second `repo` product beside Project Repositories.
- Do not infer Project Sites from arbitrary pushed files.
- Do not make generated website bytes the source of truth for skills.
- Do not edit the Finite Sites mirror directly or reconcile it back into the
  monorepo.
- Do not make all Project Repositories public-read just because Finite-owned
  baseline skills are public-read.
- Do not reintroduce `finitec repo` or `finitec publish` compatibility paths.
- Do not require GitHub for runtime managed-skill distribution once Finite
  Sites can serve the mirror.
