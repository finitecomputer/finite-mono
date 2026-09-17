# Finite Skills

- **Managed Skills Baseline**: Finite-owned skill source, bundled as one tested
  revision in an Agent Runtime. This repository is its editable authority.
- **Skills Sync**: explicit runtime-local adoption of the available baseline.
  It does not restart compute or establish a Core-controlled update schedule.
- **User Skill / Override**: user-owned content that may take precedence over a
  baseline skill and must survive sync and Runtime restart.

New agents receive the bundled baseline once. Existing agents update through
`finite skills sync`; Core and Runner do not poll, push or activate revisions.
