# Roadmap

ADR 0028 is the current target architecture for Finite Sites. Older tiered
hosting plans for document outputs, stateful app outputs, and Kata app runners
are historical context only.

## Current Target

Finite Sites is a static-only platform service:

- one independently deployed `finitesitesd` service;
- one control/API origin;
- one wildcard static-site origin;
- Project Repositories for source and collaboration;
- zero-or-one Project Site per Project;
- git pushes to the configured Deploy Branch create immutable static Versions;
- Sites-owned auth, sharing, viewer sessions, email proofs, and project
  collaborators;
- finite-identity/core used only for facts Sites does not own, such as NIP-05
  directory lookup.

## Deployment

The destination is one Fly Machine with a persistent volume and CI-built image.
The [Sites runbook](../../infra/runbooks/deploy-sites.md) owns deployment,
backup/restore qualification and cutover. API and Git use `https://finite.site`;
served Sites use `https://{site}.finite.site/`.

Validate restored static Sites and existing access before moving traffic.
There is no dual-write migration framework.

## Product Scope

The active product contract is:

- `fsite describe workflow publish-static-site --output json`;
- `fsite project init --config finite.toml [--dry-run] --output json`;
- `fsite auth git PROJECT --store --output json`;
- `fsite project grant/revoke PROJECT ...`;
- `fsite project share PROJECT ...`;
- `fsite project status/list`;
- `fsite view URL_OR_NAME`.

Finite Sites does not run builds. Agents build locally or in their own runtime,
commit the static deploy bytes under `[site].path`, and push git.

## Out Of Scope

These are not part of Finite Sites:

- app kinds;
- document/PDF output kinds;
- serverless functions;
- framework detection or builders;
- Kata/containerd app runners;
- wake-on-request app supervisors;
- direct upload publishing;
- multi-output projects.

Future dynamic compute belongs in a separate product boundary, not as another
Sites kind.
