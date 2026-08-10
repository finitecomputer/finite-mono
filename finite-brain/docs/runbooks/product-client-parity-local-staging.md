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
  import, sync projection, and Session Lock seams.
- Finite VIP Brain invitations and mailbox-addressed Folder access use a
  mutation-free account-cohort preview before a separate explicit commit;
  external-email bootstrap invitations retain their existing client-owned key
  handoff.
- Personal Brain metadata renders every current account agent and Folder grant
  fanout uses the authoritative roster's `ready` agents (with singular metadata
  only as an old-server fallback). Human-anchored Organization agent
  authority is shown as delegated routine administration, never ownership or
  recovery authority.
- Session Lock clears in-memory keys, decrypted projections, drafts, prepared
  writes, import plans, invite secrets, and rendered plaintext; explicit resume
  reopens grants through the connected Member Identity.
- Regression checks reject Product Client use of durable browser storage for
  raw Folder Keys or decrypted content.
- Secure server routes enforce Nostr auth, replay rejection, rate limits, CORS
  allowlist behavior, request body limits, and encrypted-object boundaries.
- OKF import/export, graph/replay, and Brain Working Tree logic stay
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
  graph/replay, OKF import execution, invitation preflight, and explicit cohort
  commit.
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
- the seeded smoke Brain has populated Folders and at least 50 encrypted Pages;
- the Product Client opens all seeded Pages through Folder Key Grants;
- Page navigation rows, Graph View projection, workspace state, and
  access/share panel projection work against the fixture.
- the Product Client exposes explicit locked, resume, and lock states without
  persisting readable client state.
- the Finite VIP Brain and Folder forms expose the read-only human/account-agent
  cohort, exclusions, readiness, scope, capacity, roster revision, and expiry
  before enabling a separate **Send invitation** action;
- the client refuses a reduced recipient set until the user explicitly approves
  every displayed excluded NIP-05 identity, and refuses stale, expired, or
  capacity-blocked plans without silently falling back to a legacy write;
- a backend without the cohort preflight route produces an update-required
  message and sends no invitation request.

Then open the local Product Client:

```text
http://127.0.0.1:4015/client
```

Expected Product Client behavior:

- Shows NIP-07 availability and signer state.
- Starts locked and shows explicit **Resume session** feedback.
- Can load Brain metadata with a valid NIP-07 signer.
- Can open accessible Folder Key Grants into the in-memory session keyring.
- Can decrypt accessible Pages locally.
- Can prepare encrypted signed Page writes through secure object routes.
- Can pull sync and preserve unresolved dirty local edits.
- Can build Graph View and Replay from decrypted local Page indexes.
- Shows Obsidian-like Files, Search, Access, Page, and Graph surfaces with
  right-click Folder/Page actions.
- Can parse OKF bundles, plan conflicts, rewrite copied relative links, and
  upload imported Pages through encrypted object writes.
- Entering a `finite.vip` mailbox changes the primary action to **Preview
  recipients**. Previewing does not create an invitation, alter the pending
  invitation list, or send email.
- The preview names the human and each account agent separately, shows agents
  excluded or not ready without treating an offline runtime as departed, and
  keeps raw participant npubs out of normal copy.
- A reduced preview requires explicit approval before **Send invitation** is
  enabled. A stale roster/key/capacity response consumes the plan and requires
  a new preview. Success reports one shared-mailbox delivery status.
- Mailbox-addressed restricted-Folder access commits once through the atomic
  Folder account-access route with the human and included agents; it never
  decomposes the cohort into per-npub mutations.
- Removing mailbox-addressed restricted-Folder access first previews the
  friendly human/agent cohort and any independently retained principals, then
  removes cohort provenance and rotates the Folder key once through the atomic
  account-access route. Machine planning npubs stay out of confirmation copy.
- Removing one included account agent uses the targeted-principal Folder access
  route and one key rotation. The returned `accountAccessCohorts` participant
  keeps `relationship: account_agent` and records the Folder in
  `excludedFolderIds`; the human and sibling agents retain their access. A
  Managed Agent NIP-05 is never submitted as the human mailbox target.
- A ready Personal Brain agent does not receive a browser **Remove** action for
  a distinct ready sibling agent. That change requires short-lived, exact-scope
  Authenticated Human Intent from the human-authorized Chat/CLI transport; the
  Product Client neither prompts for nor fabricates it. Owner-human removal
  continues to use the existing rotation request without the optional proof.
- Personal Brains with `personalBrainAgents` show the complete account-managed
  roster and suppress the legacy single-agent replacement controls. Older
  metadata with only `personalAgent` remains readable.
- When the connected principal is listed in
  `humanAnchoredAgentAuthorities`, the Access surface identifies the acting
  agent and authorizing human. Routine controls follow the human's current role;
  ownership, recovery, and whole-Brain deletion stay human-only.
- **Lock session** hides protected content and clears keys, opened grants,
  decrypted Pages, local drafts, graph/search projections, prepared writes,
  import state, invite secrets, and rendered plaintext without deleting or
  changing the external signer.
- Page navigation/back-forward-cache suspension locks synchronously. A signed
  event whose `pubkey` differs from the connected Member Identity also hard
  locks before any protected request is sent.
- A locked session does not reopen grants until **Resume session** runs the
  normal encrypted-grant flow. Switching Brains applies the same clearing rule.
- An invitation fragment is removed from browser history immediately, held only
  as a one-shot in-memory pre-session capability, and imported after explicit
  **Resume session**. Lock, Brain switching, and failed Resume discard it.

Locked or inaccessible Folders must remain locked in the client. The server must
not return plaintext search results or accept plaintext OKF imports.

The first-party Product Client must not write raw Folder Keys or decrypted
content to Web Storage, IndexedDB, Cache Storage, cookies, or browser history.
It denies automatic plaintext egress such as remote embeddings, content-bearing
analytics, and unprompted external requests. Explicit controller exports are
allowed, and authorized third-party clients are outside FiniteBrain's
post-decryption enforcement boundary.

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

- The account-cohort controls in this client depend on ADR 0045 and the additive
  metadata/routes landing in the account-agent cohort backend change. Without
  those fields and routes, the new authority path remains inert and Finite VIP
  mailbox writes fail closed; do not treat this frontend branch alone as a
  supersession of ADR 0016.

- Do not add legacy route compatibility.
- Do not add old runtime migration shims to this Product Client parity PR.
- Do not move plaintext OKF import/search onto the server.
- Do not weaken encrypted object route requirements to ease old-client testing.
- Do not add durable browser plaintext/key caches as a restart convenience.
- Do not infer agent authority from Organization membership. Use only the
  explicit active `humanAnchoredAgentAuthorities` projection and re-check the
  authorizing human's current role.
- Do not collapse a human and account agent into one principal, key, grant, or
  audit identity.
