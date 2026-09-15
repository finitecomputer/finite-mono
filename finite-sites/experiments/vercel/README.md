# Finite Sites on Vercel — single-project experiment

**Question:** can one Vercel project serve independently published private Sites,
while Finite retains authentication, sharing, and immediate request-time revocation?

This is disposable synthetic infrastructure on `alex/sites-vercel-experiment`.
Production ADR 0028, the Rust daemon, `fsite`, customer state, and production DNS
are unchanged. See [RESULTS.md](RESULTS.md) for measured evidence and omissions.

## Shape

```text
committed Git deploy tree → publisher → private Vercel Blob
                                  └→ managed Neon version catalog / active pointer

Alpha hostname ─┐
Beta hostname  ─┼→ ONE Vercel project: validated host → current Finite grant → file
Control host   ─┘                     authentication / sharing / publishing APIs
```

- **One project:** `finite-sites-poc`, reusing the former control project's ID.
- **Two Site origins:** `finite-sites-poc-one-alpha.vercel.app` and
  `finite-sites-poc-one-beta.vercel.app`, attached to that same project.
- **Control origin:** `finite-sites-poc.vercel.app`. Tenant hosts cannot invoke
  its control handlers. Control requests require explicit operator/issuer tokens.
- **One private Blob store:** `finite-sites-poc-content`. Files are immutable,
  keyed by Site, Version, and content hash. A file upload is verified before its
  database row is marked uploaded. Direct unauthenticated Blob reads fail.
- **One existing scratch Neon database:** `finite-sites-poc-db`. The `poc2_*`
  tables hold Sites, grants, single-use handoffs, viewer sessions, versions, and
  file manifests. The old `poc_*` tables are historical scratch data, unused.
- **Publish:** upload a complete version, mark ready, then compare-and-swap the
  Site's Active Version pointer. Failed uploads never replace active content;
  stale publishers cannot silently overwrite a newer publication.
- **Rollback:** select a ready version. Permissions and platform code stay current.
  Vercel deployment rollback is a separate operation on shared platform code.
- Every HTML/asset/HEAD/conditional/range request checks current grants. Viewer
  responses use `private, no-store`. Blob caching happens behind this gate.
- Host-only `__Host-` cookies bind sessions to one Site hostname. Unknown hosts,
  generated deployment hosts, extra wildcard labels, and forged tenant headers
  cannot select content. There is no tenant code in the application deployment.

`app/` is the shared service. `publish.mjs` publishes content without calling
Vercel deployment APIs. `deploy.mjs` deploys only platform code. Resource IDs and
secret locations live in `infra/experiments/sites-vercel.json`.

## Try it

```sh
scripts/with-dev-env npm --prefix finite-sites/experiments/vercel run demo
```

Open http://127.0.0.1:4319. Each Site has **Grant access**, **Open private site**,
**Revoke access**, and version-switch buttons. Alpha starts at v2 and Beta at v1.
Revoke Alpha, then reload both its page and asset: denied. Beta stays accessible
if independently granted. Version switches require a refresh of the Site tab.
The console uses `viewer@example.invalid`; credentials stay on the local server.

## Reproduce

Use existing authenticated Vercel CLI access and the declared scratch resources.
Do not initialize this schema in any production database. Node dependencies run
inside the repository's pinned Nix environment.

```sh
scripts/with-dev-env npm ci --prefix finite-sites/experiments/vercel
scripts/with-dev-env npm ci --prefix finite-sites/experiments/vercel/app
scripts/with-dev-env npm exec --yes --package=vercel@59.17.0 -- vercel link --yes --project finite-sites-poc --cwd finite-sites/experiments/vercel/app --scope alexlwn123-s-team
scripts/with-dev-env npm exec --yes --package=vercel@59.17.0 -- vercel env pull .env.local --yes --cwd finite-sites/experiments/vercel/app
scripts/with-dev-env node finite-sites/experiments/vercel/operator.mjs init
scripts/with-dev-env node finite-sites/experiments/vercel/deploy.mjs
scripts/with-dev-env node finite-sites/experiments/vercel/seed.mjs
```

`init` creates/reuses ignored owner-only operator secrets. `deploy` sets them on
this scratch project; do not run it concurrently from multiple operator machines.
The database and private Blob connections already exist. For a fresh isolated
installation, create those scratch resources first; do not reuse production ones.

Publish any committed static fixture:

```sh
scripts/with-dev-env node finite-sites/experiments/vercel/publish.mjs alpha REPOSITORY COMMIT site
```

Limits: 200 files, 1 MiB per file, 8 MiB per version. The publisher never runs
customer builds; hidden files, symlinks, unsafe paths, and platform helper paths
are rejected. A version needs `index.html`.

## Verify

Run these sequentially; the live proof changes synthetic grants and versions.
Keep the demo running for the browser test; use an existing Playwright Chromium
or set `POC_BROWSER_PATH`. No browser or system dependency is installed by tests.

```sh
scripts/with-dev-env npm --prefix finite-sites/experiments/vercel test
scripts/with-dev-env node finite-sites/experiments/vercel/live-test.mjs
scripts/with-dev-env node finite-sites/experiments/vercel/storage-test.mjs
scripts/with-dev-env node finite-sites/experiments/vercel/browser-test.mjs
```

## Wildcard configuration

The router supports one-label `{site}.{POC_SITE_BASE_DOMAIN}` routing. The live
proof currently uses explicit Vercel-provided hostnames to avoid production DNS
changes. Configure `siteBaseDomain` and attach a wildcard domain/certificate to
this same project to exercise live wildcard DNS. Exact alias overrides are in
`config.json`; existing registered Site hostnames are immutable in this proof.
Use new synthetic Site names when testing a new suffix. Actual wildcard TLS
issuance/delegation remains unverified. See [ROUTING-RESEARCH.md](ROUTING-RESEARCH.md).
