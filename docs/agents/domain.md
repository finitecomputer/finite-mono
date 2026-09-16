# Existing domain documentation

Start from the relevant Linear issue and the owning code, tests, and executable
configuration. For cross-component work, use `CONTEXT-MAP.md` when helpful to
locate the owners.

Consult existing component `CONTEXT.md`, ADRs, and runbooks when the change
touches their terminology, compatibility, security, or recovery boundaries.
If a retained contract conflicts with the requested change or implementation,
name the conflict and resolve it explicitly in the Linear issue.

Record new terminology, design decisions, and plans in Linear. Missing context
documents do not need replacements. Retain operational instructions and
contracts still consumed by code, tests, or production workflows until their
replacement is verified.
