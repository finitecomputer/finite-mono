# ADR 0028: Static-only Sites platform service

## Status

Accepted. This is the current service and Project Repository contract.
[ADR 0027](0027-daemon-local-email-proofs.md) defines Sites-owned email proofs;
[ADR 0029](0029-account-session-viewer-bridge.md) defines account viewer access.

## Service boundary

Finite Sites is an independently deployed and backed-up static hosting service
in `finite-mono`. One Fly Machine runs the CI-built, digest-pinned image with
persistent state at `/var/lib/finite-sites`. Fly terminates TLS and routes to
`finitesitesd`. Deployment, backup, recovery and content redirect maintenance
are defined in [the Sites runbook](../../../infra/runbooks/deploy-sites.md).

The control origin is `https://finite.site` for API and Git smart HTTP; served
Sites use `https://{site}.finite.site/`. The API lives under `/api/v2/*`, with
`GET /api/v2/healthz` for health. The production service is the sole publishing
authority. Isolated recovery drills use disposable targets.

The service owns registry state, Git repositories, blobs, authorization,
sharing, viewer sessions and audit history. Directory services provide facts
such as NIP-05 resolution; every Sites request is authorized against Sites
state. Sites availability and recovery do not depend on a shared Core database
as the source of truth for Sites permissions.

## Project Repositories and publishing

A Project Repository is the editable source of truth and has zero or one
Project Site. A source-only Project remains cloneable, editable and visible in
Project Status/List without a viewer URL or active Version. Project Init is
the replay-safe setup operation; adding `[site]` to a source-only Project and
replaying Init adds its Site. Project Slug and Site Name are separate identities.

```toml
[project]
slug = "my-project"

[site]
name = "my-project" # optional, defaults to project.slug
branch = "main"
path = "site"
spa = false
```

Agents inspect `fsite describe workflow publish-static-site --output json`,
edit `finite.toml`, validate with Project Init `--dry-run`, then commit and
push. Sites serves committed bytes under the configured path; agents own
builds. There is no direct bundle upload surface, app runtime, document/PDF
renderer, framework detection or multi-output model. Static files, including
pre-rendered documents, use the same Site contract.

Git Remotes use standard smart HTTP through `git-http-backend`, canonically
`https://finite.site/{project}.git`. Use the server-returned remote, including
on isolated deployments. `fsite auth git PROJECT --store` writes a scoped,
revocable credential to Git's credential helper. Bare repositories live under
`DATA_DIR/git/projects/{project_id}.git`; URLs use Project Slugs.

Project visibility controls repository read/clone/fetch independently of Site
visibility. Public-read repositories never grant push access. Pushes remain
authenticated and collaborator-gated. Collaborators edit the whole source
repository; Site Shares grant only served read access.

Git post-receive hooks record durable ref-change events before client success.
The daemon reconciles them after receive-pack and at startup. Deploy Branch
pushes create immutable Versions and atomically advance the active pointer;
other branches update source history without publication. Replay must not
create duplicate Versions. Push audit records retain actor, delegation when
present and Git credential attribution. Visibility and sharing are separate
mutations, never side effects of a push.

Generated `/llms.txt` provides Project Repository editing instructions only
when the project did not publish that path. SPA fallback is explicit. Mutable
Site URLs return `Cache-Control: no-store`; viewers are authorized on every
read.

## Authorization

`fsite auth register` explicitly creates or replays a self-sourced publish
grant for the local Publishing Key. Operator `allow`, `disallow` and `allowed`
commands manage the manual grant path. An active, unexpired grant is required
for Project creation and Site allocation. Revocation does not by itself delete
content or stop already-published Sites from serving; Site disable is separate.
Publishing limits are defined in `finitesites-proto/src/limits.rs`.

Standalone publishers verify a mailbox with `fsite auth sites-key request` /
`add` and supply `--owner-email` to Project Init. Self-registration alone is not
mailbox proof. Hosted publication may carry a verified requester assertion.
Mailbox ownership and authorized keys are Sites-local records. Agents sign as
their own Principals; a Sites key authorization never links a human and agent
identity or grants Brain access. Account requester assertions and viewer
exchange use bounded service contracts; they do not move permission authority
out of Sites.

## Compatibility and recovery

The parser accepts deprecated `[outputs.*]` configuration only when it describes
exactly one static Site. The output id is ignored. App/document/PDF kinds,
multiple outputs and retired runtime/rendering fields fail validation. Current
responses expose `site: null` or one Project Site, never public output IDs or
kinds. Retained repositories may still depend on that input compatibility.

Previous content URLs redirect through exact reviewed mappings. Old-host
cookies, tokens and Git credentials do not transfer. Startup schema migrations,
old-writer fixtures and historical snapshot readers remain recovery contracts;
the completed cutover is not evidence that every supported Recovery Set can
be read without them. Preserve those readers until equivalent existing-state
and mixed-version recovery is proved.

The Sites Recovery Set includes the registry, blobs, repositories, permissions
and cookie key. Restore onto an empty target using independently held recovery
credentials. Image rollback must read current state and preserve accepted
writes. Deleting or retiring compute does not authorize purging user data.
