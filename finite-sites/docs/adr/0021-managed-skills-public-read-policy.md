# Managed Skills Public Read Policy

Finite-owned baseline skills should be easy for hosted runtimes, local agents,
and curious external agents to inspect and sync. They should not depend on
GitHub as the canonical source once Finite Sites can host source repositories.

This ADR decides distribution read policy only. Root
[`ADR 0002`](../../../docs/adr/0002-managed-skills-are-hot-swappable-product-revisions.md)
supersedes its old source-ownership assumption: `finite-mono/finite-skills` is
the only editable source, and the Finite Sites Project Repository is a
release-automation-owned Distribution Mirror of immutable revisions.

Decision:

- The Finite-owned baseline Distribution Mirror, expected to be
  `finite-skills`, uses public read-only Project Visibility.
- Public read-only Project Visibility permits unauthenticated `git clone` and
  `git fetch` for selected Project Repositories.
- Public read-only Project Visibility never permits `git push`. Writes still
  require Project Collaborator permission and a scoped Git Credential.
- Customer, user, and team Managed Skills Repositories remain private by
  default and use normal Project Repository auth.
- Output visibility remains independent. A browsable docs output can be public
  while the repository is private, or a repository can be public-read while an
  output is private.

Why:

- Baseline skills are part of the runtime substrate and should be inspectable.
- Runtime boot and skill activation should not require per-agent credentials
  for Finite-owned public baseline material.
- Public read-only Git gives agents an obvious primitive they already
  understand: clone the source repository.
- Private-by-default Project Repositories remain the right default for user
  data, customer skills, drafts, and team-specific agent behavior.

Implementation shape:

- Add Project Visibility as repository read policy separate from Project
  Output visibility.
- Git smart HTTP may allow unauthenticated `git-upload-pack` only when the
  Project Repository has public read-only Project Visibility.
- Git smart HTTP must keep `git-receive-pack` authenticated and
  collaborator-gated for every Project Repository.
- `fsite project status` should show Project Visibility so agents can explain
  why a clone does or does not need credentials.
- Release automation publishes exact promoted revisions from the monorepo to
  `https://git.finite.chat/finite-skills.git`. Runtimes fetch only an immutable
  revision selected by Core and verify its manifest/digest; they never follow
  `main` or treat the mirror as writable source.
- Private managed skills for hosted runtimes can later use Core-granted read
  credentials or runtime Principal credentials without changing this decision.

Considered options:

- Keep GitHub or a separate Project Repository as the editable baseline source.
  Rejected because it would split first-party authorship from the monorepo and
  recreate drift.
- Require every runtime to mint Git Credentials for baseline skills. This is
  safer by default but adds bootstrap fragility for material that is intended
  to be public and inspectable.
- Grant a Finite-managed runtime Principal to every skills repository. This is
  useful later for private customer/team skills, but it is unnecessary for the
  public baseline and would make the first runtime path harder to reason
  about.
- Make all Project Repositories public-read. Rejected because Project
  Repositories are source-first and often contain data, drafts, and logic that
  should stay private even when an output is public.

Consequences:

- The product has one repository read primitive, Project Visibility, not a
  special skills-only bypass.
- Agents and release tests can inspect exact baseline revisions with plain git.
- Public read does not grant mutable-branch authority: Product Release
  manifests and verified revision digests define what a runtime may activate.
- Abuse and confidentiality controls remain centered on private-by-default
  Project Repositories, explicit Project Visibility, collaborator auth,
  revocation, and emergency operator actions.
