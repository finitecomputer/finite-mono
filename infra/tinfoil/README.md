# Tinfoil satellite repos

Tinfoil confidential-compute deploys are measured against a
`tinfoil-config.yml` at the root of a public GitHub repo. There is one config per
repo. That means each enclave keeps a thin public "satellite" repo even
though finite-mono itself is public: the measurement is per-repo-root, so
multiple enclaves cannot share this repo.

Mono's job is to produce and pin the satellites' inputs; the satellites' job
is to be measurable.

The current model/container/alias map and retired lab state are recorded in
[`model-inventory.md`](model-inventory.md).

## Satellites and deployment

- `finitecomputer/confidential-finite-private`: current Finite Private config.
- `finitecomputer/confidential-kimi-k2-6`: retained model/rollback configurations.
- `finitecomputer/finite-searxng-tinfoil`: independently owned search satellite.
- `finitecomputer/tinfoil-agent-runtime-canary`: runtime canary satellite.

Build limiter images in mono's `Service Images` workflow, select the exact
reviewed digest, and publish the measured satellite release. Follow
[Finite Private deployment and rollback](confidential-finite-private/README.md).
Run `scripts/finite-status` around every authorized rollout. The ops wrapper
requires the exact tag in `FINITE_PRIVATE_RELAUNCH_APPROVED` for mutation.

Usage-api admission requires Core's JSON health response. An HTML 200 or
redirected outage page is not admission health; do not change admission mode
without proving its configured API route and credentials.

## Secrets

Tinfoil sealed secrets (`FINITE_USAGE_API_SERVICE_KEY`, `VLLM_INTERNAL_API_KEY`,
`VLLM_API_KEY`) are set through the Tinfoil deployment surface, never in git.
