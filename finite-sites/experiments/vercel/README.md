# Finite Sites on Vercel — disposable experiment

Question: can Vercel own independent static site deployments while Finite keeps
private sharing, per-request authorization, and revocation across content rollback?

This is an experiment on `alex/sites-vercel-experiment`, not a replacement Sites
release or approval to cut over. Production ADR 0028 remains unchanged. Resources
and secret locations are declared in `infra/experiments/sites-vercel.json`.

## Shape

```text
committed Git deploy tree → experiment publisher → Vercel project per site
                                                    │ every file request
                                                    ▼
synthetic verified-email issuer → Finite control API → managed Neon Postgres
```

- `publish.mjs` selects files from a specific Git commit. It does not execute
  customer code or upload their untracked files. It injects platform-owned
  routing middleware and creates an independent Vercel deployment.
- `site-template/middleware.js` covers all paths and methods. Its only public
  helper is the platform login page; redemption requires a valid one-use proof.
  Customer content cannot override the gate/config. Site identity comes from
  trusted deployment configuration, never an inbound user/tenant header.
- `control/api/control.js` owns site registration, grants, trusted email-session
  issuance, redemption, disabling, and authorization. It runs natively on Vercel.
- Postgres contains site records, hashed gate credentials, grants, hashed
  one-use handoffs, and hashed opaque viewer sessions. Every content request
  checks current grants. No grants or viewer identities are embedded in content.
- Vercel owns static bytes, independent deployment history, TLS, and aliases.
  Platform Deployment Protection stays enabled on generated deployment URLs.
  Tests use `vercel curl` to prove Finite's gate behind that additional layer.

No Fly machine, persistent volume, SQLite registry, custom blob store, site
container, or customer build service is needed by this experiment.

## Try it on this machine

From the monorepo root, using its pinned Nix environment:

```sh
scripts/with-dev-env npm --prefix finite-sites/experiments/vercel run demo
```

Open http://127.0.0.1:4319. Click **Grant access**, **Open private site**, then
**Revoke access**. Refresh the opened site and its private asset. Both should
be denied. The console binds only loopback, checks Host and Origin, and keeps
operator credentials server-side. Stop it with Ctrl-C.

The live control overview is https://finite-sites-poc-control.vercel.app.
The private test sites are https://finite-sites-poc-alpha.vercel.app and
https://finite-sites-poc-beta.vercel.app. Direct unauthenticated visits are
expected to be denied.

## Reproduce

Vercel CLI authentication, the declared scratch projects, and the Marketplace
database are required. Dependencies must run through `scripts/with-dev-env`.
Provision a scratch database before initialization; never point this at a
production database. The control project's ignored `.env.local` is populated by
`vercel env pull`. The operator credentials are generated into an ignored
owner-only `.local-state/operator.env` file and configured as Vercel env vars.

```sh
scripts/with-dev-env npm ci --prefix finite-sites/experiments/vercel
scripts/with-dev-env npm ci --prefix finite-sites/experiments/vercel/control
scripts/with-dev-env npm ci --prefix finite-sites/experiments/vercel/site-template
scripts/with-dev-env node finite-sites/experiments/vercel/operator.mjs init
scripts/with-dev-env node finite-sites/experiments/vercel/operator.mjs push-control-env
scripts/with-dev-env node finite-sites/experiments/vercel/fixtures.mjs
```

`fixtures.mjs` prints two synthetic commits. Publish them in order:

```sh
scripts/with-dev-env node finite-sites/experiments/vercel/publish.mjs alpha REPOSITORY COMMIT_1
scripts/with-dev-env node finite-sites/experiments/vercel/publish.mjs beta REPOSITORY COMMIT_1
scripts/with-dev-env node finite-sites/experiments/vercel/publish.mjs alpha REPOSITORY COMMIT_2
```

The publisher requires `site/index.html`, rejects symlinks/submodules/hidden
files (except `.well-known`), and bounds the experiment to 200 files / 8 MiB.
The deploy path is its fourth argument. This is a separate operator adapter;
the production `fsite` binary, Git remotes, and `finite.toml` parser are untouched.

## Evidence and checks

Run **sequentially**: live checks mutate the disposable viewer grants.

```sh
scripts/with-dev-env npm --prefix finite-sites/experiments/vercel test
scripts/with-dev-env node finite-sites/experiments/vercel/probe.mjs alpha 2 V1_DEPLOYMENT_URL V2_DEPLOYMENT_URL
scripts/with-dev-env node finite-sites/experiments/vercel/rollback-test.mjs
# Keep the local demo running for the real browser flow:
scripts/with-dev-env npm --prefix finite-sites/experiments/vercel run browser-test
```

The browser test uses an existing Playwright Chromium; set `POC_BROWSER_PATH`
to an installed Chromium executable if needed. It does not install a browser.
The Node/database suite uses actual managed Postgres, with isolated test rows
removed afterward. No in-memory authorization substitute is used.

The live probe verifies anonymous and revoked HTML/assets, repeated authorized
reads, forged headers, HEAD and conditional requests, one-use proof redemption,
and deployment URL coverage. The rollback test verifies independent sites,
cross-site session denial, revocation through rollback, then promotes v2 again.
Deployment alias propagation is measured separately from authorization: a
content version may take a short period to converge after the CLI completes.

## What remains intentionally unproven

- **Production identity integration.** `POC_ISSUER_TOKEN` authorizes a synthetic
  verified-email assertion. It models the existing internal viewer-session
  trust boundary, but does not implement live WorkOS sign-in, guest email
  delivery, native-principal grants, account bridge, or existing cookie formats.
- **Source hosting and recovery.** The fixture Git repository remains local.
  Managed Git, collaborator credentials, source-only Projects, database backups,
  and a complete empty-target recovery drill are separate decisions.
- **Production publishing semantics.** No durable deployment job reconciliation,
  concurrent publication arbitration, managed version catalog, or complete
  fsite compatibility. Repeating a publish can create another deployment of the
  same commit; a CLI failure after deployment must be reconciled in Vercel.
- **Custom domains and migration.** Only Vercel-provided domains were used. No
  `finite.site` / `finite.chat` DNS, customer state, old URLs, or fleet CLI changed.
- **Public transitions and all serving semantics.** Public/private transitions,
  SPA fallback, generated `llms.txt`, source-map policy, iframe/partitioned
  cookies, download behavior, and arbitrary file sizes need their own checks.
- **Operations and economics.** Region/latency tuning, quotas, rate limiting,
  audit retention, session cleanup, billing limits, and independent recovery
  remain production work. A control API outage fails content access closed.
- **Gate code upgrades.** Content rollback retains old gate code. Revocation
  works because both deployed gate versions use the same current authority;
  future incompatible auth changes require a versioned contract or retiring
  old deployments. Vercel deployment retention is not a source archive.

No customer migration should inherit these experimental omissions silently.
Keep the live resources disposable and delete them when the experiment ends.
