# Examples

Working static-site demos, smallest first. Each example uses canonical
`[site]` configuration and publishes through a Project Repository.

## Project Repository Seed

`finitechat-native-mockup` is the Project-first validation example. The Project
init reads the committed `finite.toml`; the Project Repository source contains
the deployable mockup and its required config.

```sh
fsite project init \
  --dry-run \
  --output json \
  --config examples/finitechat-native-mockup/finite.toml

fsite project init \
  --output json \
  --config examples/finitechat-native-mockup/finite.toml

fsite project grant finitechat-native --email skyler@example.com --send-invite --output json
fsite auth login skyler@example.com
fsite auth redeem skyler@example.com TOKEN_FROM_EMAIL
fsite auth git finitechat-native --email skyler@example.com --output json
git clone https://v2.finite.chat/finitechat-native.git /tmp/finitechat-native
rsync -a --delete examples/finitechat-native-mockup/ /tmp/finitechat-native/
cd /tmp/finitechat-native
git add finite.toml index.html
git commit -m "Seed finitechat native mockup"
git push origin main
fsite project share finitechat-native --public --yes-public --output json
```

Pushing `main` is the publish step. Finite Sites validates committed bytes
selected by `finite.toml` and creates the immutable Version; it does not run
builds.

## Static Sites

- **finitechat-native-mockup**: Project-first validation example.
- **hello-site**: plain files.
- **spa-pushstate**: dependency-free single-page app using the history API.
  It sets `spa = true` so deep links serve the shell.
- **react-bun-spa**: React 19 + React Router 7 bundled with Bun. It uses
  `path = "dist"`:

```sh
cd examples/react-bun-spa
bun install
bun run build
# commit dist/ as the configured Project Site path, then git push
```

Bun's HTML entrypoint build (`bun build index.html --outdir=dist`) rewrites
the script tag to the hashed bundle; `spa = true` makes router paths
refresh-safe.
