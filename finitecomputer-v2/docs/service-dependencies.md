# Service ownership

| Boundary | Authority |
| --- | --- |
| Accounts, Projects, entitlements, Runtime placement and lifecycle intent | Core |
| Provider compute and lifecycle execution | Runner |
| Device identity, MLS and chat delivery | Finite Chat |
| Static publishing, Git repositories and viewer permissions | Sites at `finite.site` |
| Encrypted knowledge, Folder permissions and grants | Brain |
| Public NIP-05 resolution and local identity primitives | Identity |
| Managed skill source | `finite-skills/`; explicit runtime-local sync |
| Runtime-local supervision and typed agent actions | `finite-agentd` |

These are sibling components in one workspace. Consumers use their public
contracts, never copies of product code, shared permission databases or Runtime
Management Pipe feature commands. Preserve issued Finite Private grants, key
hashes, reservations, settlements and audit history across deployments.

The Runtime image is built once from a source revision, smoke-tested and
promoted by digest. New Agents receive its skills baseline once; existing
Agents update skills explicitly. Account identity, agent identity, product
permission and compute ownership remain distinct.
