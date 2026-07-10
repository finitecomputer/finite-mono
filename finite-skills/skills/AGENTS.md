# Finite Managed Skills

This directory is the single git-owned source of truth for Finite-managed Hermes
skills.

## Naming

- Every Finite-managed skill must use a `-finite` suffix in both:
  - the directory name
  - the `name:` field in `SKILL.md`
- Exception: `grill-me` intentionally ships under its canonical upstream name
  and path so the local skill invocation and the managed baseline refer to the
  same skill contract.
- The existing unsuffixed `finitebrain` name is a temporary compatibility
  exception. Its deployable body is canonical here even while a component
  reference copy remains; settle the public name in a separate migration.
- Do not add Finite-managed skills with names that could collide with Hermes
  built-in or user-local skills under `~/.hermes/skills`.
- If a Finite-managed skill references another Finite-managed skill, use the
  suffixed name everywhere:
  - `related_skills`
  - `skill_view(...)`
  - inline docs and examples
  - seeded runtime guidance like `FINITE.md`, `SOUL.md`, and generated
    `AGENTS.md`

## Ownership

- This tree is platform-owned and is the only editable source for the deployed
  Managed Skills Baseline.
- Add or edit Finite-managed skill bodies here first. Component trees own API
  contracts and tests, not duplicate deployable `SKILL.md` bodies.
- Runtime images, dashboard catalogs, and Finite Sites distribution mirrors
  consume immutable promoted revisions; none of them consumes a floating branch
  or becomes a second source of truth.
- Hermes-local/user-created skills under `~/.hermes/skills` are not managed
  here and should not be rewritten or pruned by platform tooling.
- Treat this tree as a read-only baseline and reference library, not as the
  place to iterate on a machine-specific customization.
- When a platform behavior is really a shared contract, prefer moving it out of
  individual skills into a repo-owned contract file instead of duplicating it
  across skill bodies.

## Runtime Compatibility

- A skill that names a Finite CLI, service endpoint, credential flow, Runtime
  Capability, or filesystem contract must have a promotion test against that
  exact Product Release.
- Do not use the retired `finitec` publishing, repository, or skill-management
  command families, `/profile-assets`, raw Nostr, manual OAuth, DNS, proxy, or
  host-management fallbacks in the v2 baseline unless the current runtime
  contract explicitly provides them.
- Reference bundled helpers through `${HERMES_SKILL_DIR}`. Do not hardcode a
  checkout, image, profile-assets, or `current` symlink path.
- Skill prose and helpers are trusted executable product input. Keep dependency
  and credential requirements explicit, bounded, and testable.
- A compatible prose/helper revision may ship without a Runtime restart. Any
  revision requiring a new binary, service API, credential contract, or Runtime
  Capability waits for that dependency's Product Release.

## Forking Skills Locally

- If you want to customize a Finite-managed skill for one machine, copy it into
  `~/.hermes/skills/...` and edit the local copy there.
- Do not edit the mounted managed copy in place under
  `~/.finite/managed-skills/finite/current` or the content-addressed revision
  tree behind it.
- Prefer giving the local copy a new name while experimenting so you can still
  compare it against the shipped baseline.
- Only shadow the managed skill with the same name when you intentionally want
  that machine to override the baseline behavior.

## Goal

Keep the model simple:

- Finite owns this external baseline skill tree.
- Hermes and the user own the normal writable `~/.hermes/skills` tree.
- Baseline activation never rewrites user content; intentional name overrides
  remain visible and reversible.
