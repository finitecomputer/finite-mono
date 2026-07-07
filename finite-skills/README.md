# Finite Skills

Shared Finite baseline skills for Hermes agents.

Managed runtimes sync this repository into
`~/.finite/managed-skills/finite/current` and point Hermes
`skills.external_dirs` at that checkout.

User-local skills should stay in each machine's normal Hermes skill directory.
Team/shared skills belong in the configured `user` managed skill source, not in
this global baseline repo.
