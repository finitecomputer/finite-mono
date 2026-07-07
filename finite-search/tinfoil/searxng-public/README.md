# finite SearXNG Tinfoil Prototype

This directory is a public-repo-ready bundle for the first SearXNG-only
Tinfoil Containers prototype.

Do not deploy it from `finite-search` directly. Tinfoil requires
`tinfoil-config.yml` to live at the root of a public GitHub repo so the enclave
measurement can be verified by clients.

## What This Runs

- One measured container running SearXNG plus a local bearer-token proxy.
- JSON search enabled.
- The same narrowed engine set proven on `lat2`.
- Open outbound egress so SearXNG can reach public search engines.
- Tinfoil shim routes only `/search` and `/search/*` to the auth proxy.
- Healthcheck uses the proxy's local `/healthz`, not a real search query.
- `SEARXNG_SECRET` is a Tinfoil secret, not a public env var.
- `FINITE_SEARCH_TOKEN` is a Tinfoil secret required as
  `Authorization: Bearer <token>` on `/search`.

This is still a prototype, but raw public SearXNG exposure is no longer the
intended access pattern. The canonical endpoint should be called through a
verified Tinfoil client and the bearer-token gate.

## Public Repo Setup

The current public repo is:

```text
finitecomputer/finite-searxng-tinfoil
```

For a fresh repo, copy the contents of this directory to that repo root:

```bash
rsync -av tinfoil/searxng-public/ ../finite-searxng-tinfoil/
```

Then edit `tinfoil-config.yml`:

```yaml
image: "ghcr.io/OWNER/REPO:placeholder"
```

Replace `OWNER/REPO` with the public repo path, for example:

```yaml
image: "ghcr.io/finitecomputer/finite-searxng-tinfoil:placeholder"
```

Commit and push the public repo.

## Local Smoke Before Tinfoil

From `finite-search`:

```bash
scripts/smoke-tinfoil-searxng-bundle.sh
```

That builds the local image and proves:

```text
anonymous /search -> 401
authorized /search?q=open+source&format=json -> non-empty JSON results
```

## Release

The latest published release is:

```text
v0.0.5
ghcr.io/finitecomputer/finite-searxng-tinfoil@sha256:3171c5914536eec1629bfb8e4f23a80451a8e2fc7b4c67f1215f4c5e0ab7df3e
```

`v0.0.5` keeps the verified `small_0d_new` hardware shape and adds the
bearer-token proxy plus the `FINITE_SEARCH_TOKEN` secret. It is published,
marked as the GitHub latest release, and deployed on canonical `finite-searxng`.

The previous raw-but-verified fallback release is:

```text
v0.0.4
ghcr.io/finitecomputer/finite-searxng-tinfoil@sha256:a0d2f4a6c1701e50922e666476fd7cf5707a98d5184927c36e1c7f8b7f81e9a6
```

For future releases, run the `Tinfoil Release` workflow with a new version:

```bash
gh workflow run tinfoil-release.yml -f version=v0.0.5
```

The workflow:

1. Builds and pushes `ghcr.io/<owner>/<repo>`.
2. Rewrites `tinfoil-config.yml` with the pinned image digest.
3. Creates the release tag.
4. Dispatches the publish workflow that measures and attests the image.

## Tinfoil Deploy

In the Tinfoil dashboard:

1. Add a `SEARXNG_SECRET` secret with a random value.
2. Add a `FINITE_SEARCH_TOKEN` secret with the bearer token clients must send.
3. Create a new container from the public repo and release tag.
4. Deploy in staging first.
5. Smoke through the verified Tinfoil proxy, not plain public `curl`:

```bash
tinfoil container connect finite-searxng \
  -p 3301
```

Then, in another shell:

```bash
curl -fsS \
  -H 'Authorization: Bearer <FINITE_SEARCH_TOKEN>' \
  'http://127.0.0.1:3301/search?q=open+source&format=json' |
  jq '.results | length'
```

CLI equivalent once an org admin key is available:

```bash
tinfoil login --api-key admin_...
export SEARXNG_SECRET_VALUE="$(openssl rand -hex 32)"
export FINITE_SEARCH_TOKEN_VALUE="$(openssl rand -hex 32)"
scripts/deploy-staging.sh v0.0.5
unset SEARXNG_SECRET_VALUE
unset FINITE_SEARCH_TOKEN_VALUE
```

Manual form:

```bash
tinfoil login
printf '%s' '<random-secret>' |
  tinfoil secret create SEARXNG_SECRET --value-file -
printf '%s' '<random-token>' |
  tinfoil secret create FINITE_SEARCH_TOKEN --value-file -
tinfoil container create finite-searxng \
  --repo finitecomputer/finite-searxng-tinfoil \
  --tag v0.0.5 \
  --secret SEARXNG_SECRET \
  --secret FINITE_SEARCH_TOKEN \
  --staging
```

GitHub Actions equivalent:

1. Add `TINFOIL_API_KEY` as a repository or organization secret.
2. If `SEARXNG_SECRET` does not already exist in Tinfoil, also add
   `SEARXNG_SECRET_VALUE` as a secret.
3. If `FINITE_SEARCH_TOKEN` does not already exist in Tinfoil, also add
   `FINITE_SEARCH_TOKEN_VALUE` as a secret.
4. Run the `Tinfoil Deploy - Staging` workflow with tag `v0.0.5`.

## Current Deployment

`finite-searxng` is live at:

```text
https://finite-searxng.finite.containers.tinfoil.dev
```

Current state:

```text
tag: v0.0.5
status: ready
mode: non-staging
resources: 8 CPU / 16384 MiB
secrets: SEARXNG_SECRET, FINITE_SEARCH_TOKEN
```

Direct public gate smoke passed:

```text
anonymous /search -> 401
authorized /search -> 155 results
```

Verified proxy smoke passed:

```bash
tinfoil-proxy \
  -e finite-searxng.finite.containers.tinfoil.dev \
  -r finitecomputer/finite-searxng-tinfoil \
  -p 3396

SEARXNG_URL=http://127.0.0.1:3396 \
  SEARXNG_TOKEN='<FINITE_SEARCH_TOKEN>' \
  scripts/smoke-searxng.sh
```

The verified proxy smoke returned 155 results and anonymous proxy access
returned 401.

`finite-searxng-medium` is also deployed as an experiment container at:

```text
https://finite-searxng-medium.finite.containers.tinfoil.dev
```

Its current state:

```text
tag: v0.0.4
status: ready
mode: non-staging
resources: 8 CPU / 16384 MiB
```

Direct public smoke passed for the medium experiment:

```bash
curl -fsS \
  'https://finite-searxng-medium.finite.containers.tinfoil.dev/search?q=open+source&format=json' |
  jq '{result_count: (.results | length), unresponsive: (.unresponsive_engines // [] | length)}'
```

Verified proxy smoke also passed:

```bash
tinfoil-proxy \
  -e finite-searxng-medium.finite.containers.tinfoil.dev \
  -r finitecomputer/finite-searxng-tinfoil \
  -p 3394

SEARXNG_URL=http://127.0.0.1:3394 scripts/smoke-searxng.sh
```

The verified smoke returned 155 results. This confirms the old hardware
measurement caveat is fixed for the `v0.0.4` experiment shape.

Earlier `v0.0.2` and `v0.0.3` attempts failed the verified proxy path with:

```text
failed to verify enclave: verifyHardware: failed to verify hardware measurements: no matching hardware platform found
```

`v0.0.2` changed `cvm-version` from `0.10.1` to `0.10.0` to test whether the
networking CVM version was the issue. The app still runs, but attested proxy
verification still fails.

`v0.0.3` increased top-level resources from `2 CPU / 8192 MiB` to
`4 CPU / 16384 MiB` to test whether a different CPU-only platform profile would
match Tinfoil's hardware measurements. It did not.

Deeper follow-up found that `4 CPU / 16384 MiB` is not one of the exact
published hardware profile shapes in `tinfoilsh/hardware-measurements@v0.0.35`.
The public profiles include `mini_0d` at `4 CPU / 4096 MiB` and `small_0d_new`
at `8 CPU / 16384 MiB`; Tinfoil's own `confidential-websearch` and
`confidential-doc-upload` repos also use `cvm-version: 0.10.4` with
`8 CPU / 16384 MiB` and open egress. `v0.0.4` uses that shape and verifies.

Current verifier evidence from `go run ./verifier/examples/verifier`:

```text
finite-searxng-medium runtime MRTD:
7357a10d2e2724dffe68813e3cc4cfcde6814d749f2fb62e3953e54f6e0b50a219786afe2cd478f684b52c61837e1114

finite-searxng-medium runtime RTMR0:
a2749c840579faca6adf0c9c3ab69f277556cda67f8a6b3553c2c7fbf00e9706ec77a6f6960d802433b339ff8b72eefb

matched hardware:
small_0d_new@92c6b94f64e6867989d758b1c3682d1bbd775b3fc4cee5936c50c98dfc8f5e3e

result:
TLS public key fingerprint matches
Measurements match
```

Also note that the current verifier resolves the repo's latest release; keep the
GitHub "latest" release aligned with whichever deployed tag is being tested.

## Production Gate

The selected access-control story is a measured bearer-token proxy in front of
`/search`. Before broader production use, rotate the generated development token
into a team-owned secret and decide where the calling runtime should store it.
