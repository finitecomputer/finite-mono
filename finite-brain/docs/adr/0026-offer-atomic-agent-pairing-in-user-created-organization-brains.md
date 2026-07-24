# Offer Atomic Agent Pairing In User-Created Organization Brains

Status: accepted 2026-07-21. Extends ADR-0025's Organization Brain bootstrap
model and supersedes its exclusion of automatic Product Client agent pairing.

When a human creates an Organization Brain in the Product Client, the creation
flow visibly offers to add the currently selected, identity-resolved agent as
an initial admin, with that choice on by default. If selected, Brain creates
the empty Brain and both admin memberships atomically; if agent resolution or
any part of bootstrap fails, no Brain or partial relationship is created. The
human may turn the choice off to create a human-only Organization Brain.

This makes user-first and agent-first Organization Brain setup converge without
silently granting access: agent-first bootstrap includes the authenticated
requester under ADR-0025, while user-first bootstrap includes the selected
agent only through the creation screen's explicit, reversible choice. Adding
the agent later was rejected as the default because partial failure could leave
the user's expected collaborator without access; unconditional enrollment was
rejected because Organization Brains must still support human-only creation.
