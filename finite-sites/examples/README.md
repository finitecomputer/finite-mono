# Examples

Working demos for each hosting tier, smallest first. Each was published to
finite.chat as part of platform validation.

## Project Repository seed

`finitechat-native-mockup` is the Project-first validation example. The
Project init reads the committed `finite.toml`; the Project Repository source
contains the deployable mockup and its required config.

Grant `skyler@example.com` or another External Principal after init if that
email should clone and push.

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
git clone https://git.finite.chat/finitechat-native.git /tmp/finitechat-native
rsync -a --delete examples/finitechat-native-mockup/ /tmp/finitechat-native/
cd /tmp/finitechat-native
git add finite.toml index.html
git commit -m "Seed finitechat native mockup"
git push origin main
fsite project share finitechat-native mockup --public --yes-public --output json
```

Pushing `main` is the publish step. Finite Sites validates committed bytes
selected by `finite.toml` and creates the immutable Version; it does not run
builds.

## Static Site Outputs

- **finitechat-native-mockup** — Project-first validation example. Uses
  `examples/finitechat-native-mockup/finite.toml`.
- **hello-site** — plain files. Uses `examples/hello-site/finite.toml`.
- **spa-pushstate** — dependency-free single-page app using the history API.
  Uses `spa = true` in `examples/spa-pushstate/finite.toml` so deep links serve
  the shell.
- **react-bun-spa** — React 19 + React Router 7 bundled with Bun. Uses
  `examples/react-bun-spa/finite.toml` with `path = "dist"`:
  ```sh
  cd examples/react-bun-spa
  bun install && bun run build
  # commit dist/ as the configured Project Output path, then git push
  ```
  Bun's HTML entrypoint build (`bun build index.html --outdir=dist`)
  rewrites the script tag to the hashed bundle; `spa = true` makes router
  paths refresh-safe.

## Stateful App Outputs

App outputs are git-backed too. They commit an explicit runtime payload and
receive `PORT` plus `DATA_DIR` at runtime. Live mutable state must live under
`DATA_DIR`.

- **nextjs-demo** — idiomatic Next.js, `output: "standalone"`. Uses
  `examples/nextjs-demo/finite.toml`, which publishes the generated `bundle/`
  directory and starts `node server.js`.
- **fasthtml-demo** — Python FastHTML with PEP 723 inline dependencies,
  run as `uv run app.py` on `$PORT`. Uses
  `examples/fasthtml-demo/finite.toml`.

Generated app bundles and dependency directories are intentionally ignored in
this source repo. When publishing an app example, copy the generated runtime
payload into the Project Repository and commit it there.

## Document Outputs

- **docs-demo** — a small multi-page Markdown document. Uses
  `examples/docs-demo/finite.toml`. Rendered pages live under the document
  domain; `.md`, `/llms.txt`, and `/llms-full.txt` expose agent-friendly source
  and editing context.
