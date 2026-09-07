# ADR 0028: Static-only Sites platform service

## Status

Accepted. This supersedes the target architecture in ADR 0011, ADR 0014,
ADR 0015, ADR 0016, ADR 0019, and ADR 0022 wherever those decisions
depend on app/document output kinds, Kata app runners, Core-synced publish
grants, or a multi-output public Sites model. ADR 0027 remains authoritative
for Sites-owned email proofs and the remaining finite-identity dependency.

## Context

Finite Sites currently lives in `finite-mono`, but its production shape is
still coupled to the shared server infrastructure and to a public Project
Output model that grew to include static sites, rendered documents, PDFs, and
stateful apps. The app path introduced Kata runners and wake-on-request
questions that are no longer part of the product direction. Production has no
`kind = "app"` Sites state that must be preserved, so the next architecture can
make a hard static-only cut instead of carrying compatibility modes for a
failed experiment.

The goal is not to move Sites out of this repository. The goal is to make Sites
a separate platform service: independently deployable, independently backed up,
with its own API contract and data plane, while continuing to depend on
finite-identity only for facts it does not own. Under the auth kernel, Sites
authorization is product-local; identity services may expose facts such as
NIP-05 directory lookup, but Sites must not outsource request authorization to a
shared Core authority.

Static Sites behavior must remain recognizable to users and agents: project
creation, git remotes, committed deploy bytes, immutable versions, active
version pointers, visibility, sharing, generated `/llms.txt` when absent, SPA
fallback, and existing static serving semantics are the product contract we are
preserving. The architecture change should be validated as a second service
before the canonical production endpoints move.

As of ADR 0007, canonical legacy production Sites runs on finite-lat-2 with the
rest of the app plane. Routine finite-lat-2 deploys must not become the
static-only cutover by accident.

## Decision

Finite Sites becomes a static-only platform service in this repository. The
service is implemented in the existing `finite-sites` and `fsite-cli` crates,
but deployed separately from the current monolith/shared server host. The first
production-like target is one dedicated NixOS VPS/VM running
`finitesitesd.service`, with Sites state under `/var/lib/finite-sites`, a
simple unauthenticated healthcheck, service-consistent backups, and restore
drills. Deployments stay defined in `infra/`; nothing is built on the
production host.

The v2 validation service uses one control origin, `https://v2.finite.chat`,
for API and git smart HTTP, and one validation wildcard,
`https://{site}.v2.finite.chat/`, for served sites. The v2 API lives under
`/api/v2/*`; `GET /api/v2/healthz` is the healthcheck. Git remotes returned by
the server are authoritative and may be same-origin during validation, for
example `https://v2.finite.chat/{project}.git`. The existing
`FINITE_SITES_API` environment variable is enough for selected agent boxes and
operator testing to target the validation service. The public `fsite-latest`
release must not be advanced to a static-only incompatible CLI until the
canonical production endpoint is ready for that contract.

Sites v2 has no public concept of output kinds. A Project Repository may have
zero or one Project Site. Source-only Projects remain valid. Project Init stays
the canonical create/update entry point. The canonical config shape is:

```toml
[project]
slug = "my-project"

[site]
name = "my-project" # optional, defaults to project.slug
branch = "main"
path = "site"
spa = false
```

Legacy static config using `[outputs.*]` may be accepted only as deprecated
input while agents and examples are updated. It must contain exactly one static
site output; the legacy output id is ignored; app, document, PDF, multiple
outputs, `start`, `entry`, and retired output fields fail validation. New v2
responses and docs use Site vocabulary, not Output vocabulary. Public responses
return `site: null` for source-only Projects or one Project Site object with
name, URL, visibility, active version, branch, path, and SPA setting.

Static serving remains byte-serving, not build hosting. Agents and users build
before committing. Sites validates and serves committed bytes under the
configured path, publishes immutable Versions from deploy branch pushes, and
moves the active pointer atomically. There is no Vercel-style builder,
framework detection, lambda packaging, serverless function runtime, Kata
runner, containerd app runner, app supervisor, document renderer, PDF output
type, or wake-on-request machinery in the static-only service.

Authentication and authorization are Sites-owned. Daemon-local email proofs,
Sites Authorized Keys, Site sharing, Project collaboration, viewer sessions,
and internal viewer session exchange remain product-local Sites concepts. The
only finite-identity dependency is directory-style fact lookup such as NIP-05.
Sites calls identity/core for facts it does not own, but every Sites request is
authorized against Sites state.

The validation service starts essentially empty, except for reserved names and
collision-protection data imported by a simple operator process. There is no
general migration framework, dual-write path, or runtime compatibility flag.
Legacy production Sites continues to run the old deployed binary until cutover.
At cutover, operators may run a narrow one-off reconciliation for static sites
created during validation, then switch DNS/edge targets. After the canonical
service is live and verified, obsolete app/document/output code paths can be
deleted rather than retained as dormant architecture.

## Consequences

- The app Sites experiment is intentionally not preserved. `kind = "app"` and
  Kata runner behavior are removed from the future Sites contract.
- The v2 validation host needs its own DNS, wildcard certificate/edge routing,
  service secrets, backup schedule, and restore proof before it can carry real
  production traffic.
- Existing static users should see the same serving behavior after cutover, but
  static-only `fsite` validation builds may be incompatible with the current
  public production API while v2 is opt-in.
- Public API fields, CLI arguments, docs, and workflows move from Output
  vocabulary to Site vocabulary. Private Rust identifiers can be cleaned up as
  touched, but the public v2 contract must not expose output ids or output
  kinds.
- Historical ADRs remain in the repository as history. This ADR records the new
  target boundary instead of rewriting old decisions in place.

## Considered Options

- Move Sites out of `finite-mono`: rejected because the monorepo remains the
  first-party source of truth. Deployment and data-plane separation do not
  require repository separation.
- Use Vercel or a serverless host for v1: rejected for the static-only target.
  Static byte serving, git smart HTTP, Sites-local auth, and custom sharing are
  simpler to operate as one dedicated service first.
- Preserve app/document kinds behind compatibility modes: rejected because they
  keep the failed output-kind architecture alive and complicate the new service
  before it has proven the static path.
- Build a reusable migration subsystem: rejected because v2 can be validated on
  separate endpoints and the final reconciliation is a one-time operator event.
