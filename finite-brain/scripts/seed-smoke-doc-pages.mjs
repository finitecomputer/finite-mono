#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const repoRoot = path.resolve(new URL("..", import.meta.url).pathname);
const dbPath = process.env.FINITE_BRAIN_DB || "/tmp/finite-brain-smoke-test.sqlite3";
const keyManifestPath =
  process.env.FINITE_BRAIN_SMOKE_KEYS || "/tmp/finite-brain-smoke-brain-keys.json";
const brainId = process.env.FINITE_BRAIN_SMOKE_BRAIN || "smoke";
const createdAtUnix = 1782320400;
const createdAtIso = new Date(createdAtUnix * 1000).toISOString();

function element() {
  return {
    children: [],
    className: "",
    disabled: false,
    textContent: "",
    value: "",
    addEventListener() {},
    appendChild(child) {
      this.children.push(child);
    },
    replaceChildren() {
      this.children = [];
    },
  };
}

function loadProductClient() {
  const elements = new Map();
  const context = {
    TextDecoder,
    TextEncoder,
    Uint8Array,
    atob: (value) => Buffer.from(value, "base64").toString("binary"),
    btoa: (value) => Buffer.from(value, "binary").toString("base64"),
    console,
    crypto: crypto.webcrypto,
    document: {
      createElement: element,
      getElementById(id) {
        if (!elements.has(id)) elements.set(id, element());
        return elements.get(id);
      },
    },
    window: {
      __FINITE_BRAIN_DISABLE_AUTOSTART__: true,
    },
  };
  context.globalThis = context;
  const source = fs.readFileSync(
    path.join(repoRoot, "crates/finite-brain-server/src/product-client.js"),
    "utf8"
  );
  vm.runInNewContext(source, context, { filename: "product-client.js" });
  return context.window.FiniteBrainProductClient;
}

function sqliteValue(sql) {
  return execFileSync("sqlite3", [dbPath, sql], { encoding: "utf8" }).trim();
}

function sqliteExec(sql) {
  execFileSync("sqlite3", [dbPath], { input: sql, encoding: "utf8" });
}

function sqlQuote(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

function eventIdFor(page) {
  return crypto
    .createHash("sha256")
    .update(`finite-brain-smoke-doc-page:${brainId}:${page.folderId}:${page.objectId}`)
    .digest("hex");
}

function fakeSignedEvent(template, page, authorNpub) {
  return {
    ...template,
    id: eventIdFor(page),
    pubkey: crypto.createHash("sha256").update(authorNpub).digest("hex"),
    sig: crypto
      .createHash("sha256")
      .update(`signature-placeholder:${brainId}:${page.folderId}:${page.objectId}`)
      .digest("hex")
      .repeat(2),
  };
}

const pages = [
  {
    folderId: "general",
    objectId: "fb_smoke_home_0001",
    path: "index.md",
    title: "FiniteBrain Smoke Brain",
    text: `# FiniteBrain Smoke Brain

This smoke brain is a small, real-content FiniteBrain workspace for local testing.

It is organized around the Portable v1 model:

- Brains are the top-level privacy container.
- Folders are independent access and crypto boundaries.
- Pages and Assets are encrypted Folder Objects.
- Asset Source Notes make non-Markdown source files usable by agents.
- The Product Client opens Folder Key Grants, decrypts readable Pages, and builds local views.

Start with [[Brain Model]], [[Product Client]], [[Asset Source Notes]], [[Folder Keys]], and [[Sync Append Log]].
`,
  },
  {
    folderId: "general",
    objectId: "fb_asset_source_notes_0001",
    path: "asset-source-notes.md",
    title: "Asset Source Notes",
    text: `# Asset Source Notes

FiniteBrain keeps the LLM wiki Markdown-first while preserving original evidence.

The rule is simple:

- put non-Markdown source files under the containing Folder's raw/assets/ path;
- store those files as encrypted Assets, not plaintext server blobs;
- create a Markdown Source Note in the same Folder for each Asset;
- record provenance, content type, hash or extraction status, and useful notes;
- cite Source Notes from synthesized wiki/ pages instead of citing blob bytes directly.

This gives users a durable evidence trail and gives agents something readable to search, link, summarize, and verify.

Related pages: [[OKF and Agent Wiki]], [[Working Tree Projection]], and [[Graph View Visibility]].
`,
  },
  {
    folderId: "general",
    objectId: "fb_brain_model_0001",
    path: "brain-model.md",
    title: "Brain Model",
    text: `# Brain Model

A Brain is the top-level container for a personal or organization knowledge space.

New Personal and Organization Brains start empty. Folders and content appear
only after an explicit user action or authorized product workflow.

This smoke/demo Brain deliberately adds Folders like general, agent-wiki,
graph-smoke, and brain-ops so local testing has enough content to inspect.

Folder access is binary in Portable v1. A member either has access to a Folder or they do not. There are no read-only or editor roles yet.

Related pages: [[Folder Keys]], [[Brain Invites]], and [[Shared Folder Mounts]].
`,
  },
  {
    folderId: "general",
    objectId: "fb_folder_keys_0001",
    path: "folder-keys.md",
    title: "Folder Keys",
    text: `# Folder Keys

Every Folder has its own Folder Key.

There is no Brain Root Key. Parent Folder access does not imply Child Folder access, and Child Folder access does not imply Parent Folder access.

The Product Client opens Folder Key Grants into an in-memory session keyring. Once a Folder Key is open, the client can decrypt the current encrypted objects for that Folder.

This is why graph, search, OKF export, and agent working-tree projection all follow the same visibility boundary.
`,
  },
  {
    folderId: "general",
    objectId: "fb_product_client_0001",
    path: "product-client.md",
    title: "Product Client",
    text: `# Product Client

The Product Client is the trusted browser workflow for FiniteBrain.

It owns the normal loop:

- connect a NIP-07 signer;
- prepare signed Nostr HTTP authorization;
- load Brain metadata;
- open Folder Key Grants;
- decrypt readable Pages;
- edit and encrypt Page writes;
- build graph, search, replay, and OKF views from local plaintext.

The Smoke UI remains a development harness. The Product Client is the actual direction.

See [[First Party Product Client ADR]] and [[Product Client Runbook]].
`,
  },
  {
    folderId: "general",
    objectId: "fb_portable_v1_0001",
    path: "portable-v1.md",
    title: "Portable v1",
    text: `# Portable v1

Portable v1 is the hard-cut Rust implementation target.

It covers:

- Brain, Folder, and Page domain rules;
- Folder Object encryption;
- Folder Key Grants;
- signed object revisions and tombstones;
- sync bootstrap and incremental pulls;
- shared Folder invitations and mounts;
- OKF import/export;
- working-tree projection for agents.

Hard cut means no legacy runtime compatibility shims. Old data should come through explicit import paths, not hidden compatibility routes.
`,
  },
  {
    folderId: "general",
    objectId: "fb_sync_log_0001",
    path: "sync-append-log.md",
    title: "Sync Append Log",
    text: `# Sync Append Log

FiniteBrain sync is built from ordered records.

The server keeps:

- a Brain Record Index with monotonic sequence numbers;
- current encrypted object projection for fast bootstrap;
- duplicate event detection;
- tombstones for deletes;
- cursor behavior for incremental pulls.

Clients bootstrap from current state, then pull later records by cursor. If the cursor expires, the client reboots from a fresh bootstrap.
`,
  },
  {
    folderId: "general",
    objectId: "fb_graph_replay_0001",
    path: "graph-replay.md",
    title: "Graph Replay",
    text: `# Graph Replay

Graph Replay is a Product Client projection.

The server does not store plaintext graph nodes, links, backlinks, or replay frames. The client derives those from Pages it can decrypt.

This means replay visibility follows Folder Key access:

- inaccessible Pages do not appear;
- newly opened Folder Keys can add nodes;
- removed access should remove those nodes after refresh/rebootstrap.

See [[Graph View Visibility]] and [[Replay Frames]].
`,
  },
  {
    folderId: "general",
    objectId: "fb_sharing_model_0001",
    path: "sharing-model.md",
    title: "Sharing Model",
    text: `# Sharing Model

FiniteBrain sharing has two related flows.

Brain invitations add a specific npub as a member of one Brain.

Shared Folder invitations let a source Folder appear inside another organization without copying ownership. This is closer to shared channels than email attachments.

The crypto primitive stays the same: recipients need Folder Key Grants for the source Folder.

Read [[Brain Invites]], [[Shared Folder Mounts]], and [[Mounted Folder Routing]].
`,
  },
  {
    folderId: "general",
    objectId: "fb_okf_agentwiki_0001",
    path: "okf-and-agent-wiki.md",
    title: "OKF and Agent Wiki",
    text: `# OKF and Agent Wiki

OKF is the readable import/export format for accessible content.

The agent wiki layer is the local working-tree shape agents use after the client decrypts Pages and Assets:

- AGENTS.md for operating instructions;
- _index.md for folder summaries;
- raw/ for captured source material;
- raw/assets/ for non-Markdown Assets;
- wiki/ for durable synthesized pages;
- inventory/, datasets/, and output/ for workflow material.

The server remains blind to readable OKF and wiki content unless a trusted client encrypts it back into Folder Objects.
`,
  },
  {
    folderId: "general",
    objectId: "fb_security_notes_0001",
    path: "security-notes.md",
    title: "Security Notes",
    text: `# Security Notes

Current hardening themes:

- Nostr HTTP auth validates method, URL, timestamp, payload hash, event id, and signature.
- Folder Object AAD binds ciphertext to brainId, folderId, objectId, and keyVersion.
- Payload limits, CORS allowlists, rate limits, and replay checks are server concerns.
- Plaintext Page content belongs in trusted clients, not server-side search or import routes.

Prototype boundary: the browser keyring is in-memory, so opened Folder Keys are not durable across sessions.
`,
  },
  {
    folderId: "general",
    objectId: "fb_smoke_runbook_0001",
    path: "smoke-testing-runbook.md",
    title: "Smoke Testing Runbook",
    text: `# Smoke Testing Runbook

Local smoke path:

- start finite-brain-app with FINITE_BRAIN_DB pointing at the smoke SQLite file;
- open /client;
- connect the NIP-07 signer;
- load the smoke Brain;
- open accessible Folder Key Grants;
- click each Folder and Page;
- render Graph View from decrypted Pages;
- test a Page write inside an accessible Folder.

Expected result: all seeded Folders have Pages, and locked states are explained instead of looking empty.
`,
  },
  {
    folderId: "docs",
    objectId: "fb_docs_context_map_0001",
    path: "context-map.md",
    title: "FiniteBrain Context Map",
    text: `# FiniteBrain Context Map

FiniteBrain Rust v1 is organized around a small product hierarchy: Brain -> Folder -> Page.

The current workspace keeps the implementation split into four crates:

- finite-brain-core owns domain validation, folder object crypto, portability helpers, and deterministic rules.
- finite-brain-store owns SQLite schema, migrations, sync append-log storage, and current-state projection.
- finite-brain-server owns HTTP routes, request validation, and protected route policy.
- finite-brain-app wires configuration, SQLite state, the Product Client, and the development smoke UI.

Useful entry points:

- [[Brain Model]] explains the product object hierarchy.
- [[Folder Keys]] explains why readable content stays client-side.
- [[Product Client]] explains the browser workflow used for smoke testing.
`,
  },
  {
    folderId: "docs",
    objectId: "fb_docs_readiness_0001",
    path: "readiness-matrix.md",
    title: "Portable v1 Readiness Matrix",
    text: `# Portable v1 Readiness Matrix

The current Rust implementation is intentionally SQLite-backed from day one.

Readiness checks cover:

- bootstrap and metadata visibility;
- protected route authorization;
- encrypted object writes and sync bootstrap;
- cursor expiry and rebootstrap behavior;
- filtered encrypted export;
- local Product Client static asset serving;
- local smoke UI route coverage.

The important smoke-test idea is simple: the server can store and order encrypted state, but readable Page content only exists inside a trusted client after Folder Keys are opened.

See also [[Sync Append Log]], [[Security Notes]], and [[Portable v1]].
`,
  },
  {
    folderId: "architecture",
    objectId: "fb_arch_workspace_0001",
    path: "rust-workspace.md",
    title: "Rust Workspace Architecture",
    text: `# Rust Workspace Architecture

FiniteBrain is a Rust workspace, not a single catch-all application crate.

The crate boundary is a design tool:

- core is pure logic and crypto policy;
- store is SQLite durability and transactional behavior;
- server is the route and validation shell;
- app is the runtime binary.

Reusable Nostr primitives live in finite-nostr so other Finite repos can share NIP helpers without inheriting Brain or Folder concepts.

This keeps the rebuild production-shaped while still letting the Product Client move quickly.
`,
  },
  {
    folderId: "architecture",
    objectId: "fb_arch_server_store_0001",
    path: "server-store-boundary.md",
    title: "Server and Store Boundary",
    text: `# Server and Store Boundary

The server validates protected requests, checks Brain membership, checks Folder visibility, and delegates durable state changes to the store.

The store owns:

- schema migrations;
- folder hierarchy persistence;
- Folder Key Grant metadata;
- sync append-log insertion;
- current encrypted object projection;
- idempotency and duplicate event handling;
- SQLite backup and rebuild behavior.

This split keeps route handlers thin and lets storage invariants stay testable without a browser.
`,
  },
  {
    folderId: "crypto",
    objectId: "fb_crypto_folder_objects_0001",
    path: "folder-object-crypto.md",
    title: "Folder Object Crypto",
    text: `# Folder Object Crypto

FiniteBrain Page content is encrypted as Folder Objects.

The current envelope uses AES-256-GCM with associated data that binds ciphertext to:

- brainId;
- folderId;
- objectId;
- keyVersion.

That AAD is why a payload encrypted for one Folder cannot be silently replayed as a different Folder Object.

The server stores envelopes and validates sync records, but it does not need plaintext Page content.
`,
  },
  {
    folderId: "crypto",
    objectId: "fb_crypto_grants_0001",
    path: "folder-key-grants.md",
    title: "Folder Key Grants",
    text: `# Folder Key Grants

Folder Keys are the practical access boundary for readable Pages.

In production shape, a Folder Key Grant is wrapped for a specific recipient using Nostr/NIP primitives. In this local smoke fixture, development grants are intentionally readable so the browser Product Client can auto-open seeded data for testing.

The important invariant stays the same:

- the server can know grant metadata;
- the client opens grants;
- only clients with the right Folder Key can decrypt Page content.
`,
  },
  {
    folderId: "sync",
    objectId: "fb_sync_projection_0001",
    path: "sync-current-projection.md",
    title: "Sync Current Projection",
    text: `# Sync Current Projection

FiniteBrain sync has two related shapes:

- an append-only Brain Record Index;
- a current encrypted object projection.

Clients bootstrap from the current projection, then pull later records by sequence. Duplicate event ids are ignored. Stale base revisions are rejected by the store.

If a cursor expires, the client discards the incremental cursor, runs bootstrap again, rebuilds local projection, and then resumes from the bootstrap latest sequence.
`,
  },
  {
    folderId: "sync",
    objectId: "fb_sync_conflicts_0001",
    path: "sync-conflicts.md",
    title: "Sync Conflict Policy",
    text: `# Sync Conflict Policy

The prototype conflict rule is deliberately small:

- creates start at revision 1;
- updates include baseRevision;
- the server rejects stale baseRevision writes;
- clients keep unresolved local drafts when a newer server revision appears.

This is enough to smoke test the encrypted object lifecycle without inventing a collaborative editor too early.

See [[Sync Append Log]] for the broader model.
`,
  },
  {
    folderId: "sharing",
    objectId: "fb_sharing_invites_0001",
    path: "brain-invites.md",
    title: "Brain Invites",
    text: `# Brain Invites

Brain Invitations are npub-bound and single use.

They are intentionally different from Folder shares.

An organization invite answers: can this npub join this Brain?
A Folder share answers: can this npub or destination Brain see this one Folder?

An invitation has a lifecycle:

- pending;
- accepted;
- revoked;
- expired.

Accepting an invite makes the recipient a Brain member and grants the initial Folder access selected by the admin. The invite is bound to one npub, so forwarding the link should not let a different identity claim it.

This keeps organization membership separate from Folder-level sharing.

Related pages: [[Invite Lifecycle]], [[Binary Folder Access]], and [[Shared Folder Mounts]].
`,
  },
  {
    folderId: "sharing",
    objectId: "fb_sharing_mounts_0001",
    path: "shared-folder-mounts.md",
    title: "Shared Folder Mounts",
    text: `# Shared Folder Mounts

Mounted shared folders are source-backed projections, not copies.

That means:

- the source Brain keeps owning the Folder;
- writes route back to the source Folder;
- destination organization members need source access and Folder Key Grants;
- revocation changes what the destination can continue to read.

This is the Slack shared-channel style middle ground: organizations remain distinct, but a shared Folder can appear inside another organization.
`,
  },
  {
    folderId: "portability",
    objectId: "fb_port_okf_export_0001",
    path: "okf-export.md",
    title: "OKF Export Shape",
    text: `# OKF Export Shape

FiniteBrain portability uses an OKF-style bundle for readable export.

The export shape includes:

- okf-brain.json metadata;
- Markdown Pages;
- link rewriting for present Pages;
- omissions for inaccessible Folders;
- deterministic conflict behavior on import.

Unreadable or inaccessible Folders are not exported as plaintext. They remain encrypted server state.

This makes OKF useful as a portability and agent handoff format without weakening the main crypto model. A client can export what it can decrypt, explain what it omitted, and later import the bundle by encrypting new Folder Objects through the normal Page write path.

Related pages: [[OKF Import Conflicts]], [[Working Tree Projection]], and [[Backup and Restore]].
`,
  },
  {
    folderId: "portability",
    objectId: "fb_port_working_tree_0001",
    path: "working-tree-projection.md",
    title: "Working Tree Projection",
    text: `# Working Tree Projection

The working-tree projection turns accessible decrypted Pages and Assets into a local folder/files view.

Conventions include:

- AGENTS.md for agent instructions;
- _index.md for folder summaries;
- raw/ for captured source material;
- raw/assets/ for non-Markdown Assets;
- wiki/ for durable synthesized pages;
- inventory/, datasets/, and output/ for agent workflows.

The projection is a client-side convenience. Authoritative server sync remains encrypted and ordered through the Brain Record Index.
`,
  },
  {
    folderId: "agent-wiki",
    objectId: "fb_agent_discovery_0001",
    path: "agent-discovery.md",
    title: "Agent Discovery Rules",
    text: `# Agent Discovery Rules

Agents should discover readable FiniteBrain context through the client-side decrypted projection.

Good discovery rules:

- start with AGENTS.md when present;
- read _index.md for folder intent;
- prefer curated wiki/ material before raw dumps;
- avoid assuming inaccessible Folders are empty;
- keep generated reports separate from source notes.

This lets agents work with useful local plaintext without asking the server to index or inspect private Page content.
`,
  },
  {
    folderId: "agent-wiki",
    objectId: "fb_agent_reports_0001",
    path: "generated-reports.md",
    title: "Generated Reports",
    text: `# Generated Reports

Generated reports are useful smoke-test content because they exercise search, graph links, and folder organization.

Expected report areas:

- wiki/ for durable synthesized pages;
- output/ for one-off run artifacts;
- raw/ for source material that should remain easy to audit.
- raw/assets/ for non-Markdown Assets that need Source Notes.

Reports should link back to source Pages like [[Rust Workspace Architecture]] and [[Folder Object Crypto]].
`,
  },
  {
    folderId: "graph-smoke",
    objectId: "fb_graph_links_0001",
    path: "graph-link-fixture.md",
    title: "Graph Link Fixture",
    text: `# Graph Link Fixture

This Page exists to exercise the graph view.

Links:

- [[FiniteBrain Smoke Brain]]
- [[Brain Model]]
- [[Folder Keys]]
- [[Sync Append Log]]
- [[Shared Folder Mounts]]
- [[OKF Export Shape]]

The graph should show only Pages the active client can decrypt.

This note intentionally links across several Folders so the graph view has visible cross-folder edges. If a Folder Key is missing, the graph should shrink instead of showing server-side plaintext guesses.

Good smoke checks:

- filter by "sync" and confirm the sync pages remain connected;
- open Restricted Lab and confirm restricted links appear only when readable;
- switch to replay and confirm frames are derived from readable Page changes.
`,
  },
  {
    folderId: "graph-smoke",
    objectId: "fb_graph_replay_fixture_0001",
    path: "replay-fixture.md",
    title: "Replay Fixture",
    text: `# Replay Fixture

Graph replay is derived from local decrypted Page history.

For now, this fixture is static demo content. It still helps test that graph surfaces are built from readable Pages and Folder Keys, not from server-side plaintext indexing.

Replay should feel like watching the client rebuild its local understanding:

- a Page appears after its Folder Object decrypts;
- links appear after the Page text is indexed;
- filtered-out or inaccessible Pages do not create readable nodes;
- a rebootstrap can rebuild the same graph from current state.

Useful related notes:

- [[Graph Replay]]
- [[Sync Current Projection]]
- [[Product Client]]
`,
  },
  {
    folderId: "brain-ops",
    objectId: "fb_ops_smoke_admin_0001",
    path: "smoke-admin-checklist.md",
    title: "Smoke Admin Checklist",
    text: `# Smoke Admin Checklist

This admin-only Folder is for operational smoke checks.

Before handing the local client back to a human:

- verify /health returns ok;
- verify /client serves the Product Client;
- open accessible Folder Key Grants;
- pull sync bootstrap;
- click folders and Pages in the Brain Reader;
- confirm restricted content is visible only when the right Folder Key is open.

This Page is intentionally admin-only to keep the access model visible during demos.
`,
  },
  {
    folderId: "restricted-lab",
    objectId: "fb_restricted_rotation_0001",
    path: "restricted-rotation.md",
    title: "Restricted Rotation Notes",
    text: `# Restricted Rotation Notes

Restricted Folder access is binary in this prototype.

When access is removed, the expected secure path is:

- rotate the Folder Key;
- re-encrypt live objects;
- issue new grants only to remaining recipients;
- keep old encrypted records as historical sync records;
- update the current projection to the rotated revision.

This fixture is readable in the smoke setup because the seeded admin receives the Restricted Lab Folder Key.
`,
  },
  {
    folderId: "docs",
    objectId: "fb_docs_hard_cut_0001",
    path: "hard-cut-boundary.md",
    title: "Hard Cut Boundary",
    text: `# Hard Cut Boundary

FiniteBrain Rust v1 is a hard cut from the previous prototype runtime.

That means:

- no legacy route compatibility as a product promise;
- no old runtime migration shims hidden inside the new client;
- explicit import/export paths for portable data;
- Product Client language based on Brain, Folder, and Page.

The hard cut keeps the Rust implementation small enough to reason about while still preserving the useful product ideas from v1.
`,
  },
  {
    folderId: "docs",
    objectId: "fb_docs_product_runbook_0001",
    path: "product-client-runbook.md",
    title: "Product Client Runbook",
    text: `# Product Client Runbook

The Product Client parity runbook verifies that the trusted browser workflow can perform the core spine.

Required checks:

- serve /client and static assets;
- load /client/config.json;
- connect through NIP-07;
- sign protected route requests;
- open Folder Key Grants;
- decrypt readable Pages;
- prepare encrypted Page writes;
- build graph, replay, OKF, and sync projections locally.

This smoke brain is intentionally docs-heavy so those flows have enough content to inspect.
`,
  },
  {
    folderId: "architecture",
    objectId: "fb_arch_finite_nostr_0001",
    path: "finite-nostr-boundary.md",
    title: "finite-nostr Boundary",
    text: `# finite-nostr Boundary

finite-nostr is for reusable Nostr primitives.

Good candidates:

- NIP-19 identity encoding;
- Nostr event serialization and verification;
- NIP-44 encryption adapters;
- NIP-59 wrapping helpers;
- NIP-98-style HTTP authorization helpers.

Not candidates:

- Brain policy;
- Folder access rules;
- Folder Object AAD;
- OKF import behavior;
- Product Client state.

FiniteBrain uses Nostr, but FiniteBrain policy belongs in finite-brain.
`,
  },
  {
    folderId: "architecture",
    objectId: "fb_arch_sqlite_day_one_0001",
    path: "sqlite-from-day-one.md",
    title: "SQLite From Day One",
    text: `# SQLite From Day One

SQLite is the authoritative state store for the Rust implementation.

The reason is simple: sync, grants, invites, mounts, backups, and recovery are durability problems from the start.

The store owns:

- schema migrations;
- transaction boundaries;
- current projection rebuilds;
- duplicate event handling;
- restart behavior;
- backup and consistency checks.

In-memory stores can help pure unit tests, but SQLite is the reference path.
`,
  },
  {
    folderId: "crypto",
    objectId: "fb_crypto_http_auth_0001",
    path: "nostr-http-auth.md",
    title: "Nostr HTTP Auth",
    text: `# Nostr HTTP Auth

Protected FiniteBrain routes use signed Nostr authorization events.

The event binds:

- request method;
- absolute request URL;
- request body hash when a body is present;
- timestamp within the configured skew window;
- signer identity.

The server verifies the event id and Schnorr signature, then derives the actor npub from the signer.

This avoids trusting caller-supplied user ids for protected Brain operations.
`,
  },
  {
    folderId: "crypto",
    objectId: "fb_crypto_signed_revisions_0001",
    path: "signed-revisions.md",
    title: "Signed Revisions",
    text: `# Signed Revisions

Folder Object creates, updates, moves, and deletes are signed events.

The signature covers the intent of the revision, while the encrypted payload holds the private Page content. That split lets the server validate ordering and authorship without reading the Page.

Create/update/move writes include:

- brainId;
- folderId;
- objectId;
- operation;
- revision;
- baseRevision;
- keyVersion;
- ciphertext hash;
- author npub.

Deletes create tombstones instead of silently removing history.

The server validates the signed event against the submitted encrypted payload before appending the record.

This keeps the sync log audit-friendly: clients can tell who attempted a write, which object changed, and whether the encrypted bytes match the signed revision metadata.
`,
  },
  {
    folderId: "crypto",
    objectId: "fb_crypto_canonical_vectors_0001",
    path: "canonical-test-vectors.md",
    title: "Canonical Test Vectors",
    text: `# Canonical Test Vectors

Portable v1 treats serialization as part of the cryptographic boundary.

Compatibility fixtures should cover:

- auth event serialization;
- encrypted Folder Object envelopes;
- Folder Key Grant plaintext;
- ciphertext hashes;
- base64 encoding;
- Nostr event ids;
- revision and tombstone payloads;
- duplicate sync submissions;
- stale baseRevision conflicts.

The goal is to make another implementation fail loudly if it hashes or serializes a security-critical object differently.
`,
  },
  {
    folderId: "sync",
    objectId: "fb_sync_cursor_rebootstrap_0001",
    path: "cursor-rebootstrap.md",
    title: "Cursor Rebootstrap",
    text: `# Cursor Rebootstrap

Sync cursors are a convenience, not a permanent truth.

If a client cursor expires, the expected behavior is:

- stop incremental pull;
- request sync bootstrap again;
- rebuild the current encrypted projection;
- reopen/decrypt readable Pages with the session keyring;
- resume incremental pulls from the new latest sequence.

This keeps the client correct even when old event windows compact away.
`,
  },
  {
    folderId: "sync",
    objectId: "fb_sync_duplicate_events_0001",
    path: "duplicate-events.md",
    title: "Duplicate Events",
    text: `# Duplicate Events

Duplicate event handling keeps retry behavior boring.

If a client submits the same signed record twice, the store should detect the existing event id and return the existing sequence instead of appending a second logical change.

This matters for flaky browsers, refreshes, and reconnects.

Idempotency belongs near storage because it depends on durable event ids and transaction boundaries.
`,
  },
  {
    folderId: "sync",
    objectId: "fb_sync_base_revision_0001",
    path: "base-revision-conflicts.md",
    title: "Base Revision Conflicts",
    text: `# Base Revision Conflicts

Updates include baseRevision.

If the client thinks a Page is at revision 1 but the server is already at revision 2, the server rejects the stale write.

The Product Client should keep the local draft unresolved, fetch the newer server version, and let the user or future merge helper decide what to do.

This is intentionally simpler than real-time collaborative editing.
`,
  },
  {
    folderId: "sharing",
    objectId: "fb_sharing_invite_lifecycle_0001",
    path: "invite-lifecycle.md",
    title: "Invite Lifecycle",
    text: `# Invite Lifecycle

Brain invites are singleton and npub-bound.

They should behave like a traditional one-person invite link in a business app: only the intended recipient can accept it, it cannot be reused after success, and an admin can revoke it before acceptance.

States:

- pending;
- accepted;
- revoked;
- expired.

An accepted invite cannot be accepted again by another npub. Revoking a pending invite prevents future acceptance, but it does not silently remove already-created membership from an accepted invite.

Folder access after joining still depends on the initial Folder choices and grants.

That final point matters for demos: being a Brain member does not automatically mean every restricted Folder opens.
`,
  },
  {
    folderId: "sharing",
    objectId: "fb_sharing_mounted_routing_0001",
    path: "mounted-folder-routing.md",
    title: "Mounted Folder Routing",
    text: `# Mounted Folder Routing

A mounted shared Folder appears in a destination Brain but remains owned by the source Brain.

Client behavior:

- display the mount where the destination org expects it;
- read source metadata and source encrypted objects;
- open source Folder Key Grants;
- send writes back to the source Brain and Folder;
- remove or lock the mount when access is revoked.

This avoids copying private content between organizations while still supporting shared-channel style collaboration.
`,
  },
  {
    folderId: "sharing",
    objectId: "fb_sharing_binary_access_0001",
    path: "binary-folder-access.md",
    title: "Binary Folder Access",
    text: `# Binary Folder Access

FiniteBrain currently keeps Folder access binary.

Binary means the product only answers one question for a Folder: does this user have access? It does not yet separate viewer, commenter, editor, or owner roles.

For a given Folder:

- allowed means read and write encrypted content;
- not allowed means see only permitted metadata;
- admins have access to every organization Folder;
- restricted access lists name additional members.

Role-based editor/viewer permissions are intentionally deferred until the product really needs them.

The upside is that Folder Keys and product language stay aligned. If a client can open the Folder Key, the Folder is available. If it cannot, the UI should say access is missing rather than pretending the Folder is empty.
`,
  },
  {
    folderId: "portability",
    objectId: "fb_port_path_rules_0001",
    path: "path-and-naming-rules.md",
    title: "Path and Naming Rules",
    text: `# Path and Naming Rules

FiniteBrain has two path layers.

Folder hierarchy paths are server-visible metadata.

Page paths live inside encrypted Folder Object plaintext.

Rules:

- paths are UTF-8 and normalized to Unicode NFC;
- comparisons are case-sensitive;
- Page paths must be relative safe paths;
- reserved root names include .finitebrain, _admin, and .git;
- Folder ids and object ids are stable opaque identifiers;
- moving a Folder changes metadata only.
`,
  },
  {
    folderId: "portability",
    objectId: "fb_port_import_conflicts_0001",
    path: "okf-import-conflicts.md",
    title: "OKF Import Conflicts",
    text: `# OKF Import Conflicts

OKF import planning should be explicit before writing encrypted objects.

Conflict behavior should answer:

- does this Page already exist in the destination Folder?
- should import skip, overwrite, or copy with a new path?
- which links need rewriting?
- which inaccessible folders were omitted from export?
- which destination Folder Keys must be open before upload?

The server should receive encrypted writes, not readable OKF payloads.
`,
  },
  {
    folderId: "portability",
    objectId: "fb_port_backup_restore_0001",
    path: "backup-restore.md",
    title: "Backup and Restore",
    text: `# Backup and Restore

Portable v1 backup is mostly a server-state concern.

The backup shape should preserve:

- SQLite metadata tables;
- Brain Record Index order;
- current encrypted object projection;
- Folder Key Grant metadata;
- invitations, share links, mounts, and access state.

Restore ordering matters: metadata and grants must exist before clients can decrypt current objects after rebootstrap.
`,
  },
  {
    folderId: "agent-wiki",
    objectId: "fb_agent_agents_md_0001",
    path: "agents-md-contract.md",
    title: "AGENTS.md Contract",
    text: `# AGENTS.md Contract

The working tree can create AGENTS.md files for agent-readable operating context.

A useful AGENTS.md should tell an agent:

- what this Folder is for;
- which local conventions matter;
- where raw source material lives;
- where Assets and Source Notes live;
- where synthesized wiki pages and generated output should go;
- what not to infer from inaccessible Folders.

This is client-side generated context over decrypted Pages, not server-side policy.
`,
  },
  {
    folderId: "agent-wiki",
    objectId: "fb_agent_wiki_conventions_0001",
    path: "wiki-folder-conventions.md",
    title: "Wiki Folder Conventions",
    text: `# Wiki Folder Conventions

Agent-facing wiki folders use a predictable shape.

Common paths:

- _index.md for a human and agent summary;
- raw/ for captured source material;
- raw/assets/ for non-Markdown Assets;
- wiki/ for durable synthesized pages;
- inventory/ for source candidates, open questions, and next actions;
- datasets/ for manifests, schemas, samples, and query recipes;
- output/ for run artifacts and reports.

Every Asset should have a Markdown Source Note in the same Folder before an agent cites it from synthesized work.

These paths are conventions inside an accessible working tree. The encrypted server state remains Folder Objects and sync records.
`,
  },
  {
    folderId: "graph-smoke",
    objectId: "fb_graph_visibility_0001",
    path: "graph-visibility.md",
    title: "Graph View Visibility",
    text: `# Graph View Visibility

Graph View must be built from decrypted Pages.

Visibility rules:

- all nodes come from readable Folder Objects;
- links to unreadable Pages may be shown as unresolved labels or omitted;
- tags and backlinks are local client indexes;
- server-visible object metadata is not enough to build a readable graph.

This smoke folder links across the brain so graph filtering can be tested.
`,
  },
  {
    folderId: "graph-smoke",
    objectId: "fb_graph_replay_frames_0001",
    path: "replay-frames.md",
    title: "Replay Frames",
    text: `# Replay Frames

Replay frames are a local view over applied sync history and decrypted Page indexes.

Useful frame events:

- Page created;
- Page updated;
- Page moved;
- Page deleted;
- Folder Key opened;
- Folder access lost after refresh.

Replay is not a separate authoritative event model. It is a Product Client visualization.

A good replay implementation should be boringly reproducible. Given the same current encrypted projection, opened Folder Keys, and applied sync records, the client should rebuild the same visible frames.

That makes replay a debugging tool too: when a Page is missing from graph view, the replay can show whether the Folder Key never opened, the object failed to decrypt, or the graph index did not see the link.
`,
  },
  {
    folderId: "restricted-lab",
    objectId: "fb_restricted_lab_0001",
    path: "restricted-lab.md",
    title: "Restricted Lab",
    text: `# Restricted Lab

This Folder is restricted to make access behavior visible during smoke testing.

It should demonstrate:

- restricted metadata can appear in the Brain tree;
- Page content only opens when the current user has a Folder Key Grant;
- removing access requires key rotation;
- stale local plaintext must not become server-visible.

In the local fixture, the seeded admin can read it so the Product Client has something real to show.
`,
  },
  {
    folderId: "restricted-lab",
    objectId: "fb_restricted_access_example_0001",
    path: "restricted-access-example.md",
    title: "Restricted Access Example",
    text: `# Restricted Access Example

A traditional app might say: You do not have access to this folder.

FiniteBrain should explain the same thing in product terms:

- this Folder is restricted;
- your account does not have a current Folder Key Grant;
- an admin can grant access or repair setup;
- if access was removed, old keys may no longer open current content.

That framing is much clearer than saying the Folder is broken.
`,
  },
  {
    folderId: "brain-ops",
    objectId: "fb_ops_bootstrap_0001",
    path: "bootstrap-seed-expectations.md",
    title: "Bootstrap Seed Expectations",
    text: `# Bootstrap Seed Expectations

Smoke/demo bootstrap should be reproducible.

Required expectations:

- create every demo Folder with a current Folder Key;
- create Folder Key Grants for every seeded Folder;
- seed encrypted Pages and Assets through the same crypto helper path the client uses;
- include Source Notes when a seed adds non-Markdown Assets;
- avoid hidden browser keyring state;
- keep test-only content clearly marked or omit it from reusable fixtures.

This page exists so future seed changes have a checklist.
`,
  },
  {
    folderId: "brain-ops",
    objectId: "fb_ops_hardening_0001",
    path: "hardening-watchlist.md",
    title: "Hardening Watchlist",
    text: `# Hardening Watchlist

Before production, keep pressure on:

- replay resistance;
- clock skew;
- nonce uniqueness;
- CORS and CSRF boundaries;
- payload size limits;
- rate limits;
- local plaintext lifetime;
- NIP-07 trust boundary;
- backup restore consistency;
- migration tests.

The smoke client should make these boundaries visible without pretending the prototype is already production-hardened.

The goal is not to scare users with crypto internals. The goal is to keep engineering honest while the UI says simple things like "access missing", "key opened", "write rejected because the page changed", or "server cannot read this page".

This Folder is admin-only so operational notes can exist in the same Brain without leaking into ordinary member views.
`,
  },
];

async function main() {
  if (!fs.existsSync(dbPath)) throw new Error(`SQLite DB not found: ${dbPath}`);
  if (!fs.existsSync(keyManifestPath)) {
    throw new Error(`Folder key manifest not found: ${keyManifestPath}`);
  }

  const manifest = JSON.parse(fs.readFileSync(keyManifestPath, "utf8"));
  const client = loadProductClient();
  const keyring = client.createSessionKeyring();
  const adminNpub = manifest.seededAdminNpub || "npub-smoke-admin";

  for (const [folderId, folderKey] of Object.entries(manifest.folderKeys || {})) {
    await client.openFolderKeyGrantPlaintext(keyring, {
      version: "finite-folder-key-grant-v1",
      brainId,
      folderId,
      keyVersion: 1,
      issuerNpub: adminNpub,
      recipientNpub: adminNpub,
      folderKey,
      issuedAt: createdAtIso,
    });
  }

  const objectIds = pages.map((page) => page.objectId);
  const quotedIds = objectIds.map(sqlQuote).join(", ");
  const statements = ["BEGIN;"];
  try {
    statements.push(
      `DELETE FROM current_encrypted_brain_objects WHERE brain_id = ${sqlQuote(
        brainId
      )} AND object_id IN (${quotedIds});`
    );
    statements.push(
      `DELETE FROM brain_record_index WHERE brain_id = ${sqlQuote(
        brainId
      )} AND object_id IN (${quotedIds});`
    );

    let sequence = Number(
      sqliteValue(
        `SELECT COALESCE(MAX(sequence), 0)
         FROM brain_record_index
         WHERE brain_id = ${sqlQuote(brainId)};`
      )
    );

    for (const [index, page] of pages.entries()) {
      const nonceBytes = crypto
        .createHash("sha256")
        .update(`finite-brain-smoke-doc-page-nonce:${brainId}:${page.folderId}:${page.objectId}`)
        .digest()
        .subarray(0, 12);
      const write = await client.buildPageWriteRequest(keyring, {
        authorNpub: adminNpub,
        baseRevision: null,
        createdAtUnix: createdAtUnix + index,
        folderId: page.folderId,
        keyVersion: 1,
        nonceBytes,
        objectId: page.objectId,
        plaintext: page.text,
        signEvent: (event) => fakeSignedEvent(event, page, adminNpub),
        brainId,
      });
      const payloadJson = JSON.stringify({
        recordType: "folder_object_revision",
        folderId: page.folderId,
        objectId: page.objectId,
        baseRevision: null,
        keyVersion: write.keyVersion,
        cipher: write.cipher,
        ciphertext: write.ciphertext,
        revisionEvent: write.revisionEvent,
      });
      const acceptedAt = new Date((createdAtUnix + index) * 1000).toISOString();
      sequence += 1;
      statements.push(
        `INSERT INTO brain_record_index (
          brain_id, sequence, record_event_id, record_type, folder_id, object_id,
          revision, actor_npub, client_created_at, payload_json, accepted_at,
          record_event_kind
        ) VALUES (
          ${sqlQuote(brainId)}, ${sequence}, ${sqlQuote(write.revisionEvent.id)},
          'folder_object_revision', ${sqlQuote(page.folderId)}, ${sqlQuote(page.objectId)},
          1, ${sqlQuote(adminNpub)}, ${sqlQuote(acceptedAt)}, ${sqlQuote(payloadJson)},
          ${sqlQuote(acceptedAt)}, ${write.revisionEvent.kind}
        );`
      );
      statements.push(
        `INSERT INTO current_encrypted_brain_objects (
          brain_id, folder_id, object_id, payload_json, revision, updated_at, deleted
        ) VALUES (
          ${sqlQuote(brainId)}, ${sqlQuote(page.folderId)}, ${sqlQuote(page.objectId)},
          ${sqlQuote(payloadJson)}, 1, ${sqlQuote(acceptedAt)}, 0
        );`
      );
    }

    statements.push("COMMIT;");
    sqliteExec(statements.join("\n"));
  } catch (error) {
    throw error;
  }

  const existingPages = Array.isArray(manifest.pages) ? manifest.pages : [];
  const seededIds = new Set(objectIds);
  manifest.pages = [
    ...existingPages.filter((page) => !seededIds.has(page.objectId)),
    ...pages.map(({ folderId, objectId, path, title }) => ({ folderId, objectId, title, path })),
  ].sort((left, right) =>
    `${left.folderId}/${left.objectId}`.localeCompare(`${right.folderId}/${right.objectId}`)
  );
  manifest.seededAt = createdAtIso;
  fs.writeFileSync(keyManifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

  console.log(
    `Seeded ${pages.length} FiniteBrain smoke doc Pages into ${dbPath} for brain ${brainId}.`
  );
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
