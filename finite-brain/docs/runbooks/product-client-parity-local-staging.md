# Product Client Parity Local/Staging Runbook

Status: hard-cut v1 verification runbook.

This runbook verifies FiniteBrain Rust v1 Product Client parity without
production deployment, production migrations, live data operations, or legacy
compatibility bridges.

## Scope

This verifies:

- Rust workspace builds and tests.
- Product Client assets are served by the Rust app/server.
- Product Client JS contains the NIP-07, encrypted Page loop, graph/replay, OKF
  import, and sync projection seams.
- Secure server routes enforce Nostr auth, replay rejection, rate limits, CORS
  allowlist behavior, request body limits, and encrypted-object boundaries.
- OKF import/export, graph/replay, and Vault Working Tree logic stay
  client/local-agent owned.

This does not verify:

- Production deployment.
- Production configuration changes.
- Live data migration.
- Backwards compatibility with the old prototype runtime.
- Plaintext server import/search.

## Local Server

Start the app on a local port:

```sh
FINITE_BRAIN_ADDR=127.0.0.1:4015 cargo run -p finite-brain-app
```

In another shell:

```sh
curl -fsS http://127.0.0.1:4015/health
curl -fsS http://127.0.0.1:4015/client | rg 'FiniteBrain|obsidian-shell|Graph View|OKF'
curl -fsS http://127.0.0.1:4015/client/config.json
curl -fsS http://127.0.0.1:4015/client/app.js | rg 'buildAuthEventTemplate|buildPageWriteRequest|buildGraphProjection|buildReplayFrames|parseOkfBundle|prepareOkfImportWrites|accessBadgesForFolder'
curl -fsS http://127.0.0.1:4015/client/app.css | rg 'obsidian-shell|graph|access-inspector|okf'
```

Expected result:

- `/health` returns `status: ok`.
- `/client` serves the Product Client shell.
- `/client/config.json` reports the configured public base URL and Nostr auth
  kind.
- `/client/app.js` contains the trusted-client seams for auth, crypto, sync,
  graph/replay, and OKF import execution.
- `/client/app.css` contains Product Client styling.

## Required Gates

Run these before marking Product Client parity ready for staging review:

```sh
node --check crates/finite-brain-server/src/product-client.js
node crates/finite-brain-server/src/product-client.test.js
node --check scripts/seed-smoke-doc-pages.mjs
node --check scripts/verify-obsidian-product-client.mjs
node scripts/seed-smoke-doc-pages.mjs
node scripts/verify-obsidian-product-client.mjs
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
git diff --check
```

Focused hardening evidence:

```sh
cargo test -p finite-brain-server protected_routes -- --nocapture
cargo test -p finite-brain-server cors_preflight_is_allowlist_driven -- --nocapture
```

## Browser Smoke

Seed the docs-rich smoke fixture and run the repeatable prototype smoke before
manual browser inspection:

```sh
node scripts/seed-smoke-doc-pages.mjs
node scripts/verify-obsidian-product-client.mjs
```

The verifier checks that:

- the static HTML/CSS/JS still expose the Obsidian shell, file sidebar,
  search panel, context menu, graph pane, and Access inspector surfaces;
- the seeded smoke Vault has populated Folders and at least 50 encrypted Pages;
- the Product Client opens all seeded Pages through Folder Key Grants;
- Page navigation rows, Graph View projection, workspace state, and
  access/share panel projection work against the fixture.

Then open the local Product Client:

```text
http://127.0.0.1:4015/client
```

Expected Product Client behavior:

- Shows NIP-07 availability and signer state.
- Can load Vault metadata with a valid NIP-07 signer.
- Can open accessible Folder Key Grants into the in-memory session keyring.
- Can decrypt accessible Pages locally.
- Can prepare encrypted signed Page writes through secure object routes.
- Can pull sync and preserve unresolved dirty local edits.
- Can build Graph View and Replay from decrypted local Page indexes.
- Shows Obsidian-like Files, Search, Access, Page, and Graph surfaces with
  right-click Folder/Page actions.
- Can parse OKF bundles, plan conflicts, rewrite copied relative links, and
  upload imported Pages through encrypted object writes.

Locked or inaccessible Folders must remain locked in the client. The server must
not return plaintext search results or accept plaintext OKF imports.

## Staging Notes

For a staging server:

- Set the public base URL to the externally visible staging origin so Nostr auth
  URL validation and the default CORS origin match the browser URL.
- If the Product Client is served from a separate origin, configure an explicit
  CORS allowlist through `ServerState::with_cors_allowed_origins`.
- Keep protected-route rate limits explicit. The default is 120 requests per 60
  seconds per signer/method/path.
- Treat replay cache and rate limits as in-process protections. A horizontally
  scaled deployment needs an edge/shared policy before public production
  traffic.
- Keep `Smoke UI` development-only. Product usage should go through `/client`.

## Hard-Cut Boundary

Portable v1 is a hard cut:

- Do not add legacy route compatibility.
- Do not add old runtime migration shims to this Product Client parity PR.
- Do not move plaintext OKF import/search onto the server.
- Do not weaken encrypted object route requirements to ease old-client testing.
