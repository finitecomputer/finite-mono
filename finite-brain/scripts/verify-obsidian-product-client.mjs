#!/usr/bin/env node
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const repoRoot = path.resolve(new URL("..", import.meta.url).pathname);
const dbPath = process.env.FINITE_BRAIN_DB || "/tmp/finite-brain-smoke-test.sqlite3";
const keyManifestPath =
  process.env.FINITE_BRAIN_SMOKE_KEYS || "/tmp/finite-brain-smoke-vault-keys.json";
const vaultId = process.env.FINITE_BRAIN_SMOKE_VAULT || "smoke";

function fail(message) {
  throw new Error(message);
}

function assertIncludes(source, marker, label) {
  assert.ok(source.includes(marker), `${label} is missing ${marker}`);
}

function assertNotIncludes(source, marker, label) {
  assert.ok(!source.includes(marker), `${label} still includes ${marker}`);
}

function element() {
  return {
    children: [],
    className: "",
    dataset: {},
    disabled: false,
    hidden: false,
    style: {},
    textContent: "",
    value: "",
    addEventListener() {},
    appendChild(child) {
      this.children.push(child);
    },
    replaceChildren() {
      this.children = [];
    },
    setAttribute(name, value) {
      this[name] = String(value);
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
      createElementNS: element,
      getElementById(id) {
        if (!elements.has(id)) elements.set(id, element());
        return elements.get(id);
      },
      querySelector() {
        return element();
      },
    },
    window: {
      __FINITE_BRAIN_DISABLE_AUTOSTART__: true,
      innerHeight: 900,
      innerWidth: 1400,
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

function sqlQuote(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

function sqliteRows(sql) {
  const output = execFileSync("sqlite3", [dbPath, sql], {
    encoding: "utf8",
    maxBuffer: 10 * 1024 * 1024,
  }).trim();
  if (!output) return [];
  return output.split("\n");
}

function requireFile(filePath, label) {
  if (!fs.existsSync(filePath)) fail(`${label} not found: ${filePath}`);
  return fs.readFileSync(filePath, "utf8");
}

function readFolders() {
  return sqliteRows(
    `SELECT id || char(9) || path || char(9) || access || char(9) || current_key_version || char(9) || shared_folder_source || char(9) || setup_incomplete
     FROM folders
     WHERE vault_id = ${sqlQuote(vaultId)}
     ORDER BY path;`
  ).map((line) => {
    const [id, folderPath, access, keyVersion, sharedFolderSource, setupIncomplete] =
      line.split("\t");
    return {
      access,
      accessUserIds: [],
      currentKeyVersion: Number(keyVersion),
      id,
      path: folderPath,
      setupIncomplete: setupIncomplete === "1",
      sharedFolderSource: sharedFolderSource === "1",
    };
  });
}

function readCurrentObjects() {
  return sqliteRows(
    `SELECT folder_id || char(9) || object_id || char(9) || revision || char(9) || payload_json
     FROM current_encrypted_vault_objects
     WHERE vault_id = ${sqlQuote(vaultId)} AND deleted = 0
     ORDER BY folder_id, object_id;`
  ).map((line) => {
    const [folderId, objectId, revision, payloadJson] = line.split("\t");
    return {
      folderId,
      objectId,
      revision: Number(revision),
      vaultId,
      ...JSON.parse(payloadJson),
    };
  });
}

async function openFixturePages(client, manifest, objects) {
  const keyring = client.createSessionKeyring();
  const adminNpub = manifest.seededAdminNpub || "npub-smoke-admin";
  for (const [folderId, folderKey] of Object.entries(manifest.folderKeys || {})) {
    await client.openFolderKeyGrantPlaintext(keyring, {
      version: "finite-folder-key-grant-v1",
      folderId,
      folderKey,
      issuerNpub: adminNpub,
      issuedAt: manifest.seededAt || new Date(0).toISOString(),
      keyVersion: 1,
      recipientNpub: adminNpub,
      vaultId,
    });
  }
  return client.openSyncObjects(keyring, { objects, vaultId });
}

function checkStaticShell() {
  const html = requireFile(
    path.join(repoRoot, "crates/finite-brain-server/src/product-client.html"),
    "Product Client HTML"
  );
  const css = requireFile(
    path.join(repoRoot, "crates/finite-brain-server/src/product-client.css"),
    "Product Client CSS"
  );
  const js = requireFile(
    path.join(repoRoot, "crates/finite-brain-server/src/product-client.js"),
    "Product Client JS"
  );

  for (const marker of [
    "obsidian-shell",
    "compact-icon-button",
    "vaultControlDetails",
    "vaultControlSummary",
    "vaultSelect",
    "vault-connect-button",
    "organizationVaultNameInput",
    "createOrganizationVaultButton",
    "app-ribbon",
    "file-sidebar",
    "ribbonFilesButton",
    "ribbonGraphButton",
    "ribbonCommandButton",
    "ribbonAccessButton",
    "sidebarModeTitle",
    "sidebar-icon-button",
    "searchSidebarPanel",
    "readerFolderList",
    "contextMenu",
    "pageWorkspace",
    "graphWorkspace",
    "graphEmptyState",
    "graph-icon-button",
    "accessFolderButton",
    "accessSidebarCount",
    "accessFolderViewButton",
    "accessVaultViewButton",
    "accessFolderPanel",
    "accessVaultPanel",
    "accessFolderDropdown",
    "accessFolderList",
    "accessInspector",
    "accessBasicView",
    "accessCurrentFolder",
    "accessSummaryLine",
    "accessWhoHasSection",
    "accessManageToggle",
    "accessWhoHasList",
    "accessAddPersonForm",
    "accessAddPersonInput",
    "accessAddPersonButton",
    "accessAdvancedSection",
    "accessShareSection",
    "accessShareForm",
    "accessShareHint",
    "accessShareTargetInput",
    "accessOverviewPanel",
    "accessFlowPanel",
    "accessTargetNpubInput",
    "accessShareExpiresAtInput",
    "accessShareMountInput",
    "grantFolderAccessButton",
    "removeFolderAccessButton",
    "createShareLinkButton",
    "accessShareLinkInput",
    "acceptShareLinkButton",
    "revokeShareLinkButton",
    "accessResultPanel",
    "accessBusyStatus",
    "vaultManagementTitle",
    "vaultPeopleList",
    "vaultGuideSteps",
    "vaultInvitationList",
    "vaultInvitationCount",
    "sharedFolderList",
    "sharedFolderCount",
    "folderShareLinkList",
    "folderShareLinkCount",
    "addVaultMemberButton",
    "addVaultAdminButton",
    "vaultInvitationPanel",
    "vaultInviteTargetNpubInput",
    "vaultInviteFoldersInput",
    "vaultInviteExpiresAtInput",
    "createVaultInvitationButton",
    "revokeVaultInvitationButton",
    "vaultInviteCodeInput",
    "getVaultInvitationButton",
    "acceptVaultInvitationButton",
    "readerModeButton",
    "editorSlashMenu",
    "readerPageContent",
    "pageSourceEditorLabel",
    "readerPagePath",
    "commandPalette",
    "commandPaletteInput",
    "commandPaletteList",
  ]) {
    assertIncludes(html, marker, "Product Client HTML");
  }

  for (const marker of [
    ".obsidian-shell",
    ".obsidian-shell[data-workspace-view=\"graph\"]",
    "[hidden]",
    "--shadow-access-ring",
    ".compact-icon-button",
    ".vault-control-body",
    ".vault-picker",
    ".vault-create-row",
    ".vault-connect-button",
    ".app-ribbon",
    ".obsidian-folder-button",
    ".obsidian-file-title",
    ".context-menu",
    ".command-palette-backdrop",
    ".command-palette-row",
    ".graph-stage",
    ".graph-icon-button",
    ".graph-empty-state",
    ".graph-canvas.is-hovering",
    ".node.hover-active",
    ".edge.hover-connected",
    ".graph-replay-overlay",
    ".access-view-switch",
    ".access-mode-panel",
    ".access-folder-selector",
    ".folder-selector-button",
    ".folder-dropdown",
    ".access-inspector-new",
    ".access-basic-view",
    ".access-who-has-list",
    ".access-inline-form",
    ".access-advanced-section",
    ".access-advanced-summary::after",
    ".access-button-row",
    ".vault-management-section",
    ".access-vault-admin",
    ".access-field",
    ".access-checkbox",
    ".access-share-hint",
    ".access-share-list",
    ".access-link-status",
    ".vault-guide-steps",
    ".vault-guide-marker",
    ".access-busy-status",
    ".access-mode-panel.is-busy",
    ".access-badge",
    ".note-content-empty",
    ".note-markdown",
    ".editor-slash-menu",
    ".editor-slash-row",
    ".inline-page-editor",
    ".note-markdown table",
    ".note-markdown pre[data-language]",
    ".task-list",
    ".page-source-editor",
    ".note-source",
    ".internal-link",
  ]) {
    assertIncludes(css, marker, "Product Client CSS");
  }

  for (const marker of [
    "buildGraphProjection",
    "buildReplayFrames",
    "buildAdminAccessChangeEvent",
    "buildFolderKeyGrantRequest",
    "canonicalAdminAccessChangePayload",
    "commandPaletteRows",
    "renderVaultControlChrome",
    "visibleVaultOptions",
    "personalVaultIdForPubkey",
    "vaultControlsCollapsedAfterLoad",
    "workspaceChromeState",
    "graphNeighborIds",
    "accessBadgesForFolder",
    "accessActionRoute",
    "accessIntentValue",
    "accessPanelState",
    "accessPeopleSummary",
    "normalizeAccessView",
    "vaultPeopleRows",
    "vaultInvitationRows",
    "folderShareLinkRows",
    "sharedFolderRelationshipRows",
    "vaultGuideStepRows",
    "refreshVaultAdminLists",
    "refreshFolderShareLinks",
    "revokeVaultInvitationById",
    "revokeShareLinkById",
    "acceptSharedFolderInvitationById",
    "revokeSharedFolderInvitationById",
    "vaultHealthBadges",
    "addVaultMemberFromPanel",
    "addVaultAdminFromPanel",
    "buildFolderAccessRemovalRequest",
    "buildVaultInvitationRequest",
    "vaultInvitationCreatePath",
    "vaultInvitationLinkPath",
    "vaultInvitationAcceptPath",
    "vaultInvitationRevokePath",
    "markdownFromEditorElement",
    "saveActivePage",
    "splitMarkdownTableRow",
    "tableMarkdownFromEditorNode",
    "parseMarkdownListItem",
    "visualEditorElement",
    "markdownPreviewBlocks",
    "pageLinkContext",
    "readerFolderRows",
    "readerPageRows",
    "pagePathLabel",
    "readerPageDetail",
  ]) {
    assertIncludes(js, marker, "Product Client JS");
  }

  for (const marker of [
    "obsidian-titlebar",
    "traffic-lights",
    "titlebarTabLabel",
    "titlebarVaultLabel",
    "pageTabButton",
    "graphTabButton",
    "titlebarNewTabButton",
    "editorToolbar",
    "inline-editor-toolbar",
    "data-editor-command",
    "savePageButton",
    "syncBootstrapButton",
    "workspace-status-cluster",
    "folderCount",
    "folderList",
    "readerPageList",
    "right-sidebar",
    "sidebar-footer",
    "status-bar",
    "outgoingLinkList",
    "backlinkList",
    "pageStatusDetail",
    "vaultStatusDetail",
    "activityLog",
    "Advanced client tools",
    "Smoke UI",
    "header-icon-button",
    "pageVisualEditor",
    "editorModeToggleButton",
    "accessManageSection",
    "accessAcceptSection",
  ]) {
    assertNotIncludes(html, marker, "Product Client HTML");
  }

  for (const marker of [
    ".obsidian-titlebar",
    ".traffic-light",
    ".titlebar-tab",
    ".editor-toolbar",
    ".inline-editor-toolbar",
    ".workspace-tab",
    ".tab-strip",
    ".workspace-status-cluster",
    "#folderCount",
    ".reader-layout",
    ".reader-list-button",
    ".right-sidebar",
    ".sidebar-footer",
    ".status-bar",
    ".right-panel",
    ".link-context-panel",
    ".activity-panel",
    ".dev-console",
    ".header-icon-button",
    ".page-visual-editor",
    ".editor-mode-toggle",
  ]) {
    assertNotIncludes(css, marker, "Product Client CSS");
  }
}

async function main() {
  if (!fs.existsSync(dbPath)) {
    fail(`Smoke SQLite DB not found: ${dbPath}. Run scripts/seed-smoke-doc-pages.mjs first.`);
  }
  if (!fs.existsSync(keyManifestPath)) {
    fail(`Smoke key manifest not found: ${keyManifestPath}. Run the smoke bootstrap first.`);
  }

  checkStaticShell();

  const manifest = JSON.parse(fs.readFileSync(keyManifestPath, "utf8"));
  const client = loadProductClient();
  const folders = readFolders();
  const objects = readCurrentObjects();
  const manifestPages = manifest.pages || [];
  const seededFolderIds = new Set(Object.keys(manifest.folderKeys || {}));
  const seededObjects = objects.filter((object) => seededFolderIds.has(object.folderId));
  assert.ok(folders.length >= 10, `expected at least 10 smoke folders, found ${folders.length}`);
  assert.ok(
    seededObjects.length >= 50,
    `expected at least 50 seeded smoke pages, found ${seededObjects.length}`
  );
  assert.ok(
    seededObjects.length >= manifestPages.length,
    `expected at least ${manifestPages.length} objects from the seed manifest, found ${seededObjects.length}`
  );
  const objectKeys = new Set(seededObjects.map((object) => `${object.folderId}/${object.objectId}`));
  for (const page of manifestPages) {
    assert.ok(
      objectKeys.has(`${page.folderId}/${page.objectId}`),
      `seed manifest page missing from current projection: ${page.folderId}/${page.objectId}`
    );
  }

  const opened = await openFixturePages(client, manifest, seededObjects);
  const readyPages = opened.objects.filter((object) => object.status === "ready");
  assert.equal(readyPages.length, seededObjects.length, "all seeded pages should decrypt");

  const metadata = { folders };
  const folderRows = client.readerFolderRows(metadata, opened.objects);
  const emptySeededFolders = folderRows.filter(
    (folder) => seededFolderIds.has(folder.id) && folder.pageCount === 0
  );
  assert.equal(
    emptySeededFolders.length,
    0,
    `expected no empty seeded folders, got ${emptySeededFolders.map((f) => f.id)}`
  );

  const generalPages = client.readerPageRows("general", opened.objects);
  assert.ok(
    generalPages.some((page) => page.title === "FiniteBrain Smoke Vault"),
    "general folder should contain the smoke vault index page"
  );

  const graph = client.buildGraphProjection(readyPages);
  assert.ok(graph.nodes.length >= 50, `expected graph nodes for every page, got ${graph.nodes.length}`);
  assert.ok(graph.edges.length > 0, "expected graph edges from seeded wiki links");
  assert.equal(client.workspaceChromeState("graph").shellView, "graph");
  assert.equal(client.workspaceChromeState("graph").graphHidden, false);
  const linkSource = readyPages.find((page) => client.extractPageLinks(page.text).length > 0);
  assert.ok(linkSource, "expected at least one seeded Page with local links");
  const linkContext = client.pageLinkContext(linkSource, readyPages);
  assert.ok(
    linkContext.outgoing.length > 0 || linkContext.backlinks.length > 0,
    "expected link context rows for seeded Pages"
  );
  assert.ok(
    client.markdownPreviewBlocks(linkSource.text).some((block) => block.type === "heading"),
    "expected Markdown preview blocks to include headings"
  );

  const restricted = folderRows.find((folder) => folder.id === "restricted-lab");
  assert.ok(restricted, "restricted-lab folder should exist");
  assert.ok(
    client.accessBadgesForFolder(restricted, new Set(["restricted-lab"])).some(
      (badge) => badge.label === "restricted"
    ),
    "restricted-lab should project a restricted badge"
  );
  assert.equal(
    client.accessActionRoute("share-folder", { folderId: "restricted-lab" }).intent,
    "links"
  );
  assert.equal(client.accessPanelState("links", restricted).mode, "links");
  assert.equal(client.accessPanelState("links", restricted).status, "restricted");

  const summary = {
    folders: folders.length,
    graphEdges: graph.edges.length,
    graphNodes: graph.nodes.length,
    pages: seededObjects.length,
    readyPages: readyPages.length,
    vaultId,
  };
  console.log(`obsidian product client smoke ok ${JSON.stringify(summary)}`);
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
