# SearXNG Tinfoil Prototype Runbook

This runbook owns the SearXNG-first Tinfoil path.

## Boundary

This is a SearXNG-only prototype. Firecrawl stays out of scope because the
working Docker deployment is a heavier browser, queue, and database stack.

The current prototype bundle lives at:

```text
tinfoil/searxng-public/
```

It is designed to be copied to a separate public GitHub repository. Tinfoil
requires a public root `tinfoil-config.yml`; the private `finite-search` repo
cannot be the deployment repo.

## Current Design

- Public repo root contains `tinfoil-config.yml`.
- Public repo builds a small image from the pinned upstream SearXNG image.
- `settings.yml` is baked into that image so JSON search and the narrowed
  engine set match the `lat2` proof.
- The Tinfoil config uses `cvm-version: 0.10.4` because SearXNG needs outbound
  egress, and current public Tinfoil egress examples use that CVM version.
- The top-level enclave shape is `8 CPU / 16384 MiB`, matching Tinfoil's
  published `small_0d_new` hardware profile shape.
- The only public shim routes are `/search` and `/search/*`.
- The healthcheck uses local `/healthz`, not a real search query.
- `SEARXNG_SECRET` is declared as a Tinfoil secret.
- The container attaches to a `public` network with `egress: open`.

## Verified Locally

Before creating or deploying the public repo, run:

```bash
scripts/smoke-tinfoil-searxng-bundle.sh
```

This proves the image can start and return non-empty JSON search results.

## Deploy Steps

1. Create a public repo, probably
   `finitecomputer/finite-searxng-tinfoil`.
2. Copy `tinfoil/searxng-public/` into that repo root.
3. Replace `ghcr.io/OWNER/REPO:placeholder` in `tinfoil-config.yml`.
4. Push the public repo.
5. In the Tinfoil dashboard, add `SEARXNG_SECRET`.
6. Run the public repo's `Tinfoil Release` workflow with `v0.0.1`.
7. Deploy the release in Tinfoil staging first.
8. Smoke through `tinfoil-proxy`, not plain `curl`.

## Deployment Attempt: 2026-07-01

Completed:

- Created public repo:
  `https://github.com/finitecomputer/finite-searxng-tinfoil`
- Pushed SearXNG Tinfoil config and release workflows.
- Ran local public-repo Docker smoke successfully.
- Ran the public repo `Tinfoil Release` workflow for `v0.0.1`.
- The workflow built and pushed GHCR image:
  `ghcr.io/finitecomputer/finite-searxng-tinfoil@sha256:4221fc420bc524fbc2797b5fdc421f86e4abc5d2036d4521bdbf5893d79b3b70`
- The publish workflow succeeded and uploaded:
  - `tinfoil-deployment.json`
  - `tinfoil.hash`

Initial blocker:

- The local Tinfoil CLI is installed at `~/.local/bin/tinfoil`.
- `tinfoil whoami`, `tinfoil secret list`, and `tinfoil container create`
  all fail with:

```text
not logged in: run `tinfoil login` or set TINFOIL_API_KEY
```

Next command once an org admin key is available:

```bash
~/.local/bin/tinfoil login
printf '%s' '<random-secret>' |
  ~/.local/bin/tinfoil secret create SEARXNG_SECRET --value-file -
~/.local/bin/tinfoil container create finite-searxng \
  --repo finitecomputer/finite-searxng-tinfoil \
  --tag v0.0.1 \
  --secret SEARXNG_SECRET \
  --staging
```

The public repo also has a manual `Tinfoil Deploy - Staging` GitHub Actions
workflow. It needs a repository or organization `TINFOIL_API_KEY` secret. If the
Tinfoil org does not already have `SEARXNG_SECRET`, add `SEARXNG_SECRET_VALUE`
as a GitHub secret for the first run.

## Deployment Attempt: 2026-07-01 With Tinfoil Access

Completed:

- Tinfoil API auth worked.
- Created Tinfoil secret `SEARXNG_SECRET`.
- Deployed `finite-searxng` from
  `finitecomputer/finite-searxng-tinfoil@v0.0.1`.
- Published `v0.0.2` with `cvm-version: 0.10.0` to test whether the verifier
  failure was specific to `0.10.1`.
- Relaunched `finite-searxng` to `v0.0.2`.
- Current container state:
  - status: `ready`
  - tag: `v0.0.2`
  - domain: `finite-searxng.finite.containers.tinfoil.dev`
  - host: `control.inf9.tinfoil.sh`
  - hardware: `TDX` / `H200`
- Direct public smoke passed:

```bash
curl -fsS \
  'https://finite-searxng.finite.containers.tinfoil.dev/search?q=open+source&format=json' |
  jq '.results | length'
```

The direct smoke returned 154 results and no unresponsive engines.

Not complete:

- The attested client path fails. Both `tinfoil container connect` and the
  standalone `tinfoil-proxy` fail with:

```text
failed to verify enclave: verifyHardware: failed to verify hardware measurements: no matching hardware platform found
```

- The existing production `kimi-k2-6` container verifies successfully on the
  same `control.inf9.tinfoil.sh` TDX/H200 host, so this is not a general local
  proxy or host outage.
- Changing SearXNG from `cvm-version: 0.10.1` to `0.10.0` did not fix it.
- Treat this as a Tinfoil verifier/platform issue to raise with Tinfoil before
  calling the deployment production-ready.

## Resource-Shape Experiment: 2026-07-01

Because the known-good `kimi-k2-6` container verifies on the same
`control.inf9.tinfoil.sh` TDX/H200 host, the next test was whether SearXNG's
small CPU-only platform shape was missing from Tinfoil's published hardware
measurements.

Completed:

- Published `v0.0.3` with top-level resources changed from `2 CPU / 8192 MiB`
  to `4 CPU / 16384 MiB`.
- Marked `v0.0.3` as the GitHub latest release because Tinfoil's verifier
  resolves the repo's latest release, not a caller-provided tag.
- Created staging experiment container `finite-searxng-medium`.
- Direct public smoke passed against:
  `https://finite-searxng-medium.finite.containers.tinfoil.dev/search?q=open+source&format=json`

Result:

- The source/code measurement now matches `v0.0.3`, but hardware verification
  still fails before full attestation succeeds.
- `finite-searxng-medium` runtime hardware registers:

```text
MRTD:
7357a10d2e2724dffe68813e3cc4cfcde6814d749f2fb62e3953e54f6e0b50a219786afe2cd478f684b52c61837e1114

RTMR0:
492006d8554a37287c46a04d4ac6c3339a463453d3c355756af39f0150e37424ccc98d0c2821732b40670393a5182e58
```

- That MRTD/RTMR0 pair is not present in
  `tinfoilsh/hardware-measurements@v0.0.35`.
- The known-good `kimi-k2-6` container on the same host matches
  `extra_large_1d_new`.

Conclusion:

- The attested path appears blocked because the two tested resource shapes do
  not match any exact Tinfoil-published hardware profile. The public
  `tinfoilsh/hardware-measurements@v0.0.35` profiles include `mini_0d` at
  `4 CPU / 4096 MiB` and `small_0d_new` at `8 CPU / 16384 MiB`, but not the
  tested `2 CPU / 8192 MiB` or `4 CPU / 16384 MiB` pairs.
- Tinfoil's own `confidential-websearch` and `confidential-doc-upload` repos
  use `cvm-version: 0.10.4`, `8 CPU / 16384 MiB`, and open egress.
- Next test was to deploy `v0.0.4` with `cvm-version: 0.10.4` and
  `8 CPU / 16384 MiB`, then smoke through `tinfoil-proxy`.

## Candidate Hardware-Profile Fix: 2026-07-01

Published `v0.0.4` to test the current public Tinfoil CPU-only egress shape:

```text
cvm-version: 0.10.4
cpus: 8
memory: 16384
```

This is the closest public match for SearXNG because:

- `tinfoilsh/confidential-websearch` uses `cvm-version: 0.10.4`,
  `8 CPU / 16384 MiB`, and `egress: open`.
- `tinfoilsh/confidential-doc-upload` uses the same top-level shape and open
  egress.
- `tinfoilsh/hardware-measurements@v0.0.35` includes `small_0d_new` with
  `8 CPU / 16384 MiB`.

Release result:

- Published release: `v0.0.4`
- Image digest:
  `ghcr.io/finitecomputer/finite-searxng-tinfoil@sha256:a0d2f4a6c1701e50922e666476fd7cf5707a98d5184927c36e1c7f8b7f81e9a6`
- Release assets: `tinfoil-deployment.json`, `tinfoil.hash`
- GitHub latest release points at `v0.0.4`, which matters because Tinfoil's
  verifier resolves the repo latest release.
- Deployed to `finite-searxng-medium` on 2026-07-02.
- Current state:
  - tag: `v0.0.4`
  - status: `ready`
  - mode: non-staging
  - resources: `8 CPU / 16384 MiB`
  - domain: `finite-searxng-medium.finite.containers.tinfoil.dev`
- Direct public smoke passed with 155 results and 0 unresponsive engines.
- Verified proxy smoke passed through local port `3394` with 155 results.
- Standalone verifier matched hardware profile
  `small_0d_new@92c6b94f64e6867989d758b1c3682d1bbd775b3fc4cee5936c50c98dfc8f5e3e`
  and reported `Measurements match`.
- Conclusion: the old hardware measurement caveat is fixed for the
  `v0.0.4` experiment shape.

## Canonical Container and Access Gate: 2026-07-02

Completed:

- Relaunched canonical `finite-searxng` from `v0.0.2` to `v0.0.4`.
- `finite-searxng` now reports:
  - tag: `v0.0.4`
  - status: `ready`
  - mode: non-staging
  - resources: `8 CPU / 16384 MiB`
  - domain: `finite-searxng.finite.containers.tinfoil.dev`
- Direct public smoke passed with 157 results and 0 unresponsive engines.
- Verified proxy smoke passed through local port `3395` with 154 results.
- Standalone verifier matched `small_0d_new` and reported `Measurements match`.
- Left `finite-searxng-medium` on `v0.0.4` as the fallback experiment
  container.

Access-control implementation:

- Added a measured in-container bearer-token proxy.
- Tinfoil shim now routes `/search` to the proxy on port `8081`.
- The proxy forwards authorized requests to local SearXNG on port `8080`.
- `FINITE_SEARCH_TOKEN` is a required Tinfoil secret.
- Clients must send:

```text
Authorization: Bearer <FINITE_SEARCH_TOKEN>
```

Local gated smoke passed:

```text
anonymous /search -> 401
authorized /search -> non-empty JSON results
```

Deployment result:

- Published release: `v0.0.5`
- Image digest:
  `ghcr.io/finitecomputer/finite-searxng-tinfoil@sha256:3171c5914536eec1629bfb8e4f23a80451a8e2fc7b4c67f1215f4c5e0ab7df3e`
- Release assets: `tinfoil-deployment.json`, `tinfoil.hash`
- GitHub latest release points at `v0.0.5`.
- Created Tinfoil secret `FINITE_SEARCH_TOKEN` with a generated development
  token.
- Relaunched canonical `finite-searxng` to `v0.0.5`.
- Current state:
  - tag: `v0.0.5`
  - status: `ready`
  - mode: non-staging
  - resources: `8 CPU / 16384 MiB`
  - domain: `finite-searxng.finite.containers.tinfoil.dev`
  - secrets: `SEARXNG_SECRET`, `FINITE_SEARCH_TOKEN`
- Direct gate smoke passed:
  - anonymous `/search`: `401`
  - bearer-token `/search`: 155 results
- Verified proxy gate smoke passed through local port `3396`:
  - anonymous `/search`: `401`
  - bearer-token `/search`: 155 results
- Standalone verifier matched `small_0d_new` and reported
  `Measurements match`.
- `finite-searxng-medium` remains on raw-but-verified `v0.0.4` as the fallback
  experiment container.

## Hermes Consumer Proof: 2026-07-02

The canonical `finite-searxng` container was tested from the Hermes web tool
path, not only with direct curl.

Preparation:

- Replaced the development `FINITE_SEARCH_TOKEN` with a fresh generated value.
- Relaunched `finite-searxng` on the same `v0.0.5` tag.
- Waited for `update_status` to clear and the container to report `ready`.

Proof:

```text
direct anonymous /search -> 401
direct bearer-token /search -> 87 raw results
Hermes SearXNG provider without token -> HTTP 401
Hermes SearXNG provider with token -> success true, 3 normalized results
Hermes web_search_tool with searxng backend -> success true, 3 results
```

First normalized `web_search_tool` title for the test query `finite computer`:

```text
Finite-state machine - Wikipedia
```

That first direct-token proof required a local Hermes provider patch, so it is
historical evidence only. The preferred runtime shape is the no-Hermes-patch
proxy chain below.

Preferred no-Hermes-patch proof:

```text
Hermes web_search
  -> scripts/searxng-token-proxy.py on localhost
  -> standalone tinfoil-proxy on localhost
  -> finite-searxng.finite.containers.tinfoil.dev
```

The Hermes provider was restored to stock behavior before this proof. Hermes
received only `SEARXNG_URL=http://127.0.0.1:18999`; `SEARXNG_TOKEN` and
`FINITE_SEARCH_TOKEN` were unset in the Hermes process.

```text
direct public anonymous /search -> 401
standalone tinfoil-proxy anonymous /search -> 401
local token proxy /search -> 96 raw results
Hermes token env present -> false
stock Hermes SearXNG provider -> success true, 3 normalized results
stock Hermes web_search_tool -> success true, 3 results
first title -> Finite-state machine - Wikipedia
```

Use standalone `tinfoil-proxy` for this path. In the local test,
`tinfoil container connect` failed forwarded `/search` requests with
`Request.RequestURI can't be set in client requests`.

## Verified Smoke

Once deployed:

```bash
tinfoil-proxy \
  -e https://<container>.<org>.containers.tinfoil.dev \
  -r finitecomputer/finite-searxng-tinfoil \
  -p 3301
```

Then from this repo:

```bash
SEARXNG_URL=http://127.0.0.1:3301 scripts/smoke-searxng.sh
```

## Remaining Human Gate

Rotate the development Tinfoil admin key and replace the generated
`FINITE_SEARCH_TOKEN` with a team-owned secret before wiring this into a
long-lived client runtime.
