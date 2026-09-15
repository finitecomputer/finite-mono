# Finite Sites on Vercel — wildcard prototype

One Vercel project serves all Sites through `*.sites-poc.lwn.lol`. Every Site
request passes through the Finite authorization gate before private content is
read from Vercel Blob. Neon stores ownership, shares, immutable version manifests,
and active version pointers. Publishing and rollback do not deploy the platform.

This is synthetic infrastructure on `alex/sites-vercel-experiment`. Production
Sites, account login, customer data and ADR 0028 are unchanged. See
[HIGH-FIDELITY.md](HIGH-FIDELITY.md) for the exact proof boundary.

## Current demo

Open **https://admin.sites-poc.lwn.lol/** for a read-only, database-backed list
of active published wildcard Sites. This synthetic catalog publicly shows only
names and links; disabled and unpublished Sites are omitted. It has no admin
actions or sign-in controls. Site content keeps its existing authorization gate.

| Site | URL | Purpose |
| --- | --- | --- |
| Wild-alpha | https://wild-alpha.sites-poc.lwn.lol | Private viewing, version switching and revocation |
| Wild-beta | https://wild-beta.sites-poc.lwn.lol | Independent sibling Site and browser isolation |
| Gamma | https://gamma.sites-poc.lwn.lol | Native Finite identity, managed Git publication and recovery |

The control API stays at `https://finite-sites-poc.vercel.app`; it is infrastructure,
not a served Site. The router accepts this exact control hostname, the reserved
`admin` wildcard host, or a valid one-label Site under the wildcard. No per-Site
aliases or alias override exist.

Start the local console, then open http://127.0.0.1:4319/:

```sh
scripts/with-dev-env npm --prefix finite-sites/experiments/vercel run demo
```

For Wild-alpha/Beta, choose **Grant access → Open private site**. Switch versions
and refresh a Site tab; the other Site stays unchanged. Revoke access and refresh
to see the gate deny the existing session. Gamma has native viewer grant/login/
revoke controls. All browser Site links use the wildcard.

The email viewer and native identities are isolated test principals. Production
WorkOS/dashboard account login is not connected. Operator credentials remain on
the local machine and never enter the console's HTML or JavaScript.

## Setup and publishing

Dependencies and toolchains use the repository Nix environment. This checkout
already has the linked project, private Blob connection, Neon integration and
ignored operator state. Do not reconstruct secrets or owner identities.

```sh
scripts/with-dev-env npm ci --prefix finite-sites/experiments/vercel
scripts/with-dev-env npm ci --prefix finite-sites/experiments/vercel/app
scripts/with-dev-env node finite-sites/experiments/vercel/seed.mjs
```

The seed publishes two versions of Wild-alpha/Beta from committed synthetic Git
bytes and leaves both on version 2. Gamma publishes through ordinary pushes to
the private `alexlwn123/finite-sites-poc-source` repository. Its Actions publisher
credential can update only Gamma; each ready version includes a complete source
Git bundle. Its Site hostname is returned by the control API.

The reusable fixture setup and recovery checks use the real Rust signer:

```sh
scripts/with-dev-env cargo build -p fsite-cli --bin fsite --example sites_poc_sign
scripts/with-dev-env node finite-sites/experiments/vercel/fidelity-setup.mjs
```

Setup reuses isolated ignored identities and cannot replace an existing owner.
`recovery.mjs` exports Gamma and restores into fresh database/Blob namespaces.
Viewer sessions expire on restore; source, content and grants survive. This is
an empty logical target drill, not provider-loss or scheduled-backup proof.

## Experiment branch only

Automatic platform deployments are restricted to `alex/sites-vercel-experiment`.
The Vercel project has preview deployments disabled and a project-level Ignored
Build Step that skips every other Git ref, including an absent ref:

```sh
[ "$VERCEL_GIT_COMMIT_REF" != "alex/sites-vercel-experiment" ]
```

Vercel interprets exit 0 as skip and exit 1 as build. This project-level guard
also applies to branches that do not contain the experiment's `vercel.json`.
That file additionally disables all Git branch patterns except the exact
experiment branch, using `git.deploymentEnabled`.

The project is connected to `finitecomputer/finite-mono`, with this exact branch
as its **production branch**, and `finite-sites/experiments/vercel/app` as its
root directory. Keep preview deployments disabled; do not enable repository-wide
PR previews. Commit and push this branch to deploy shared platform changes.
`deploy.mjs` is for unlinked bootstrap only and refuses a linked project before
making changes. Individual Site publication continues to update private content
and its active version, without a platform deployment.
See `evidence/branch-restriction.json` for the verified project settings.

## Verification

Run mutation suites serially when they share a fixture. Browser tests need the
local console and an existing Playwright Chromium installation (or
`POC_BROWSER_PATH`). No system dependencies are installed by the tests.

```sh
scripts/with-dev-env npm --prefix finite-sites/experiments/vercel test
scripts/with-dev-env node finite-sites/experiments/vercel/wildcard-test.mjs
scripts/with-dev-env node finite-sites/experiments/vercel/live-test.mjs
scripts/with-dev-env node finite-sites/experiments/vercel/storage-test.mjs
scripts/with-dev-env node finite-sites/experiments/vercel/browser-test.mjs
scripts/with-dev-env node finite-sites/experiments/vercel/native-test.mjs
scripts/with-dev-env node finite-sites/experiments/vercel/native-browser-test.mjs
scripts/with-dev-env node finite-sites/experiments/vercel/recovery-test.mjs
```

`evidence/wildcard-*` records actual DNS, trusted HTTPS, browser isolation,
private serving, publication and rollback checks. `evidence/fidelity-*` records
native identity, managed Git and recovery checks. The wildcard test proves a
random Site can be published without adding a Vercel domain or deployment;
its random probe Site is disabled afterward. Certificate renewal remains untested.

## Resources and cleanup

[Resource inventory](../../../infra/experiments/sites-vercel.json) names the
single Vercel project, scratch database, private Blob store, source repository,
secret locations, and wildcard. Remove only those declared scratch resources
when retiring the experiment; preserve the parent `lwn.lol` zone and unrelated DNS.

The old non-wildcard aliases, console panels, routing overrides and historical
screenshots are removed. Old Alpha/Beta synthetic rows are disabled; their bytes
remain private offline scratch state. Gamma keeps its versions and managed Git
binding at its wildcard hostname. Prior screenshots and reports are in Git history.
