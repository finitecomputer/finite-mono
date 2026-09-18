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

## Runtime implementation constraints

The runtime-local `finite` utility serves explicit agent-owned workflows such
as `finite skills sync`. Product features use their own services, APIs, CLIs
and skills, or typed Finite Chat requests to `finite-agentd` for local actions.
Core does not grow product feature schemas, runtime file editing or orchestrator
access to support them; `finite-agentd` never owns compute lifecycle.

Hermes version facts derive from the root flake pin across image, smoke and
release paths. The image build stamps the evaluated version into
`deploy/finite-computer/images/runtime.Dockerfile`. Baseline CLIs come from
`.#agent-runtime-toolchains`, defined in
`deploy/finite-computer/images/agent-runtime-toolchains.nix`; the image copies
that closure and the derivation's `bins` passthru owns exposed CLI names.

`finite-agentd` supervises the production Finite Chat bridge, the sole holder
of the inbound Chat sync stream. Hermes and `finite-agentd` consume separate
durable loopback inboxes. Reconnect uses bounded backoff rather than Python
polling or CLI-per-message subprocesses.
