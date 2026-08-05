# ADR 0006: Finite Private specialization is platform-managed

Status: accepted, 2026-07-26.

The canonical Finite Private profile includes the active Finite Private
Specialization Profile as one product contract. Its current main model is
DeepSeek V4 Flash 0731, but specialization identity and ownership are not named
after that model. An eligible Agent Runtime must never become ready with
specialization merely desired or absent. This is a narrow exception to ADR
0003's general rule that user drift blocks automatic configuration ownership:
while this canonical profile is selected, `finite-agentd` may replace a
conflicting `auxiliary.vision` value with the Finite-managed specialization
and own only the required `video` membership in
`platform_toolsets.finitechat`. It must preserve the exact semantic pre-image
of those owned fields and restore it when the managed profile is removed.
Existing toolset entries and all other Hermes configuration remain user-owned.
Failed activation and removal transactions restore the byte-identical whole
file; ordinary removal preserves unrelated edits made while the profile was
active.

We rejected preserving a conflicting vision value while reporting the runtime
as successfully using canonical Finite Private because that turns one named
product profile into box- and history-dependent behavior. We also rejected
silently launching without the specialization when Runner configuration is
missing; a Runner that cannot provide the complete profile is not
Specialization-Ready and must not accept that work.
