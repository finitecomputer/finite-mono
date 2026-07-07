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

- This tree is platform-owned and shipped through the runtime baseline.
- Add or edit shared skill bodies in this repo first. The `finitecomputer` repo
  should only reference them through `nix/agent-runtime/skills/registry.json`
  and runtime guidance.
- Hermes-local/user-created skills under `~/.hermes/skills` are not managed
  here and should not be rewritten or pruned by platform tooling.
- Treat this tree as a read-only baseline and reference library, not as the
  place to iterate on a machine-specific customization.
- When a platform behavior is really a shared contract, prefer moving it out of
  individual skills into a repo-owned contract file instead of duplicating it
  across skill bodies.

## Forking Skills Locally

- If you want to customize a Finite-managed skill for one machine, copy it into
  `~/.hermes/skills/...` and edit the local copy there.
- Do not edit the mounted managed copy in place under
  `/profile-assets/hermes-local/managed-skills/...`.
- Prefer giving the local copy a new name while experimenting so you can still
  compare it against the shipped baseline.
- Only shadow the managed skill with the same name when you intentionally want
  that machine to override the baseline behavior.

## Goal

Keep the model simple:

- Finite owns this external baseline skill tree.
- Hermes and the user own the normal writable `~/.hermes/skills` tree.
- The two should never fight over the same skill name.
