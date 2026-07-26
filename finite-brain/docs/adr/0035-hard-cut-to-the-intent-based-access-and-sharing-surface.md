# Hard-Cut To The Intent-Based Access And Sharing Surface

Status: accepted

FiniteBrain will replace the overlapping `access` mutations, `permissions`,
`share`, plural `collaborators`, and plural `invites` CLI commands and the
corresponding legacy HTTP resources in one hard cut rather than retaining
deprecated aliases or compatibility routes. The canonical interface will use
read-only `access`, desired-state `collaborator`, invitation-oriented `invite`,
cross-Brain `mount`, and low-level `admin` namespaces, with the CLI and HTTP API
using the same domain language. This deliberately trades legacy compatibility
for one unambiguous interface that users, agents, first-party clients, skills,
and documentation can learn without carrying two competing models.

The HTTP hard cut also replaces the blanket `/_admin` prefix. Normal signed
product resources and workflows live under `/v1`, while `/v1/admin` is reserved
for low-level member, role, and Folder-access mutations. There are no
compatibility routes, redirects, or dual-write clients.

Normal hosted-agent CLI use follows a no-plumbing contract. The binary and
Runtime provide production server, signing-origin, Finite Home, config, and
Working Tree defaults; commands infer the current Brain and Folder from an
unambiguous Brain Working Tree. Users and agents provide the semantic target or
destination, while `--server`, `--config-dir`, `--brain`, `--folder`, and path
overrides remain available for ambiguity resolution, local development, and
advanced automation. Ambiguous context fails closed with actionable choices
rather than guessing. `fbrain open personal` resolves the user's unique
Personal Brain, and `fbrain doctor` verifies the same real signed API path used
by ordinary commands rather than reporting health from transport alone.
