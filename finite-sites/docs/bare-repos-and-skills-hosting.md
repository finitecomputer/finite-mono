# Bare Repos And Skills Hosting

Status: requirements note.

Date: 2026-07-02

## Problem Statement

Finite Computer is cutting `finitec repo` and `finitec publish` from the new
self-serve runtime shape. Finite Sites should cover both useful behaviors:

- Project Repositories replace machine-owned `finitec repo` workflows.
- Project Outputs replace `finitec publish` website workflows.
- A Finite-managed skills Distribution Mirror can live in Finite Sites instead
  of making GitHub a production runtime dependency.

The product vocabulary supports this direction: a Project Repository is the
source primitive and may exist before a public-facing output exists.

## Product Shape

### Bare Project Repository

A Bare Project Repository is a normal Project Repository with no Project
Outputs yet. It has:

- a Project Slug;
- owner and collaborator permissions;
- a Git Remote;
- Git Credentials;
- Project Status and Project List entries;
- audit records for ref updates;
- no viewer-facing URL and no active Version.

It is a repository first, not a private site with no files and not a failed
publish.

Agent-facing shape:

```sh
fsite describe workflow register-and-publish --output json
fsite auth register --output json
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
fsite auth git PROJECT --store --output json
git clone https://git.finite.chat/PROJECT.git
git push origin main
```

The `finite.toml` for a Bare Project Repository is valid with only a project
section:

```toml
[project]
slug = "finite-skills"
```

### Add Outputs Later

A Bare Project Repository can gain Project Outputs later without changing its
Git Remote or Project Slug. The explicit mutation is to add output entries to
`finite.toml` and replay Project Init:

```sh
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
```

Hard constraints:

- Adding an output is idempotent when the existing output matches the config.
- Replaying with incompatible existing output settings fails deterministically.
- Pushing a branch before an output exists records Git history but creates no
  Version.
- Pushing a Deploy Branch after an output is added reconciles normally.
- Existing output config changes remain rejected until an explicit update
  design exists.

### Skills Distribution Mirror With Browsable Output

The canonical editable source is `finite-mono/finite-skills`. Release automation
can publish immutable Finite Skills Revisions into a `finite-skills` Project
Repository Distribution Mirror. Runtimes fetch an exact promoted revision
through Finite Sites, while humans and agents can browse a Project Output
generated from that same revision.

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
- Output visibility remains separate. A browsable docs output can be public,
  shared, or private without changing whether runtime Git fetches are public.
- Mirror writes come only from release automation publishing a promoted
  monorepo revision. Runtime activation never follows `main`.

The clean final shape:

```toml
[project]
slug = "finite-skills"

[outputs.docs]
kind = "document"
document_name = "finite-skills"
branch = "main"
path = "skills"
```

Document Output is the preferred browsable shape for Markdown-backed skills
and docs. The monorepo remains the editable source of truth; the Project
Repository and generated HTML are distribution and browsing views.

## Requirements

### Bare Repository Requirements

- `ProjectConfig::validate` accepts zero outputs.
- `ProjectConfig::to_toml_string` preserves a `[project]`-only config.
- `ProjectInitResponse.outputs` may be empty.
- `fsite project init` help explains that a Project Repository may start
  without outputs.
- `fsite project status` renders an empty output list without implying
  failure.
- `fsite project status` and `fsite project list` include Project Visibility.
- `fsite project list` includes Bare Project Repositories.
- `fsite auth git PROJECT --store --output json` works for Bare Project
  Repositories.
- Git clone/fetch/push works for Bare Project Repositories.
- Git push to a branch with no matching output does not create a Version and
  does not produce a deploy failure state.
- Bare Git Remotes set `HEAD` to `refs/heads/main` so empty clones do not
  warn about a nonexistent default ref.

### Managed Skills Distribution Requirements

- Finite Sites can represent promoted `finite-skills` revisions as a Project
  Repository Distribution Mirror.
- Runtime agents can fetch an exact Finite-owned baseline revision from Finite
  Sites without a GitHub dependency or per-agent credential.
- A runtime never consumes a mutable branch head, and a fetched revision is not
  active until its Product Release manifest and artifact digest verify.
- Maintainer write access stays restricted to approved Principals or Agent
  Keys.
- Public read-only Project Visibility is opt-in for selected Finite-owned
  baseline repositories. It is not the default Project Visibility.
- `finitesitesd project-visibility PROJECT public-read` is idempotent and
  replay-safe. `private` turns anonymous clone/fetch off again.
- Customer, user, and team Managed Skills Repositories remain private unless a
  user deliberately changes Project Visibility.
- The repository can expose a browsable output for humans and agents.
- The browsable output must not become the install source. Runtime sync reads
  Git; the website is documentation.
- Release automation is the only writer to the distribution mirror; normal
  authoring lands in `finite-mono/finite-skills` first.

### Document Output Requirements

Document Output v0 supports `kind = "document"` with a Markdown directory or
single Markdown file selected by `finite.toml`. For skills, the useful v0
document features are:

- recursive Markdown routes under `skills/`;
- directory index pages from `_index.md` or generated navigation;
- code fences and frontmatter rendered predictably;
- best-effort internal and wikilink rendering without making prose quality a
  deploy blocker;
- a generated root index if the source tree does not provide one.

## Tests

Coverage required before product code depends on this:

- `[project]`-only config parses, validates, encodes, and round-trips.
- `project init` creates a Bare Project Repository and replays safely.
- Conflicting owner cannot initialize an existing Bare Project Repository slug.
- `auth git` works for a Bare Project Repository.
- Real git clone/push works for a Bare Project Repository.
- Pushing to a Bare Project Repository records refs but creates no Version.
- Adding a first output to a Bare Project Repository succeeds and replays.
- Replaying incompatible output config fails.
- After output add, a real git push to the Deploy Branch creates a Version.
- Project Status and Project List include empty-output repositories cleanly.
- Managed-skills fixture with public Project Visibility can be cloned and
  fetched without credentials.
- A promoted revision can be fetched by exact immutable id and matches the
  release manifest digest; a moved or mutable branch head is never accepted as
  activation authority.
- Managed-skills fixture with private Project Visibility rejects anonymous
  clone and fetch.
- Anonymous push to a public-read Managed Skills Repository is rejected.
- Authenticated collaborator push to a public-read Managed Skills Repository
  still works.
- Project Visibility migration preserves private repositories, maps legacy
  ambiguous `public` project rows to `public-read`, and clamps legacy `shared`
  project rows to `private`.
- Managed-skills fixture can serve a browsable output without making generated
  HTML the install source.

## Non-Goals

- Do not add a second `repo` product beside Project Repositories.
- Do not infer Project Outputs from arbitrary pushed files.
- Do not make generated website bytes the source of truth for skills.
- Do not edit the Finite Sites mirror directly or reconcile it back into the
  monorepo.
- Do not make all Project Repositories public-read just because Finite-owned
  baseline skills are public-read.
- Do not reintroduce `finitec repo` or `finitec publish` compatibility paths.
- Do not require GitHub for runtime managed-skill distribution once Finite
  Sites can serve the mirror.
