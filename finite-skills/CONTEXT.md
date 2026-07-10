# Finite Skills

Finite Skills owns the platform-managed knowledge and workflows offered to
deployed agents without owning user-created skills or fleet lifecycle.

## Language

**Managed Skills Baseline**:
The platform-owned set of Finite-specific skills offered to every Agent Runtime.
_Avoid_: User skills, Hermes built-ins, fleet desired state

**Bundled Skills Baseline**:
The tested Managed Skills Baseline included with a new Agent Runtime.
_Avoid_: Network checkout, latest branch, user skill tree

**Published Skills Bundle**:
A versioned distribution of the Managed Skills Baseline eligible for explicit
sync.
_Avoid_: Editable source, floating branch, Runtime image

**Skills Sync**:
A user- or agent-initiated update of one Runtime's Managed Skills Baseline.
_Avoid_: Automatic rollout, Runtime upgrade, Runner replace, reboot

**User Skill**:
A skill owned by the user or agent in the Runtime's writable skill area.
_Avoid_: Managed Skills Baseline, platform bundle

**User Skill Override**:
A User Skill that intentionally takes precedence over a baseline skill.
_Avoid_: Managed edit, forked baseline

## Relationships

- Every new Agent Runtime starts with exactly one **Bundled Skills Baseline**.
- A **Published Skills Bundle** is derived from the same source as the
  **Bundled Skills Baseline**.
- An existing Agent Runtime changes its baseline only through an explicit
  **Skills Sync**.
- A **Skills Sync** changes neither the Runner nor the Runtime image.
- A **User Skill** is independent of every managed bundle.
- A **User Skill Override** survives baseline sync and Runtime restart.

## Example Dialogue

> **Dev:** "Should Core push a new skills revision to every running agent?"
> **Domain expert:** "No. New agents get the bundled baseline; existing agents update when the user or agent runs Skills Sync."

> **Dev:** "Does the Phala adapter need to know where skills come from?"
> **Domain expert:** "No. Skills Sync is runtime-local and Runner-neutral."

> **Dev:** "Can a sync replace a skill the user customized?"
> **Domain expert:** "No. That is a User Skill Override and remains user-owned."

## Flagged Ambiguities

- "Managed" means Finite authors the baseline; it does not mean Core controls
  a desired revision or rollout schedule.
- "Installed" previously meant present in an image, visible to Hermes, or
  listed in a repository. Use **Bundled Skills Baseline** or **Skills Sync**
  according to the actual state change.
- "Sync" is explicit and one-shot. It does not mean polling, automatic update,
  or fleet convergence.
