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
    "sidebar-primary-nav",
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
    "graph-floating-controls",
    "accessFolderButton",
    "accessSidebarCount",
    "accessFolderPanel",
    "accessFolderDropdown",
    "accessFolderList",
    "accessInspector",
    "accessBasicView",
    "accessCurrentFolder",
    "accessSummaryLine",
    "accessWhoHasSection",
    "accessWhoHasList",
    "access-action-stack",
    "access-state-stack",
    "accessAddPersonPanel",
    "accessAddPersonForm",
    "accessAddPersonInput",
    "accessAddPersonButton",
    "accessAdvancedSection",
    "accessShareSection",
    "accessShareForm",
    "accessShareHint",
    "accessShareTargetInput",
    "accessShareExpiresAtInput",
    "accessShareMountInput",
    "createShareLinkButton",
    "accessShareLinkInput",
    "acceptShareLinkButton",
    "revokeShareLinkButton",
    "accessResultPanel",
    "accessBusyStatus",
    "vaultManagementTitle",
    "vaultPeopleList",
    "vaultInvitationList",
    "vaultInvitationCount",
    "sharedFolderList",
    "sharedFolderCount",
    "folderShareLinkListSection",
    "folderShareLinkList",
    "folderShareLinkCount",
    "vaultPeopleActionPanel",
    "vaultPeopleActionHint",
    "addVaultMemberButton",
    "addVaultAdminButton",
    "vaultInvitationPanel",
    "vaultInviteTargetNpubInput",
    "vaultInviteFoldersInput",
    "vaultInviteExpiresAtInput",
    "createVaultInvitationButton",
    "revokeVaultInvitationButton",
    "vaultInviteUrlOutput",
    "vaultInviteUrlInput",
    "copyVaultInviteUrlButton",
    "vaultInviteCodeInput",
    "vaultInviteEmailInput",
    "vaultInviteEmailProofCreatedAtInput",
    "vaultInviteSecretInput",
    "vaultInviteConnectSignerButton",
    "getVaultInvitationButton",
    "getEmailInviteInstructionsButton",
    "acceptVaultInvitationButton",
    "settingsManageVaultsButton",
    "manageVaultsModal",
    "savePageButton",
    "editorSlashMenu",
    "readerPageContent",
    "pageMarkdownEditorLabel",
    "readerPagePath",
    "commandPalette",
    "commandPaletteInput",
    "commandPaletteList",
  ]) {
    assertIncludes(html, marker, "Product Client HTML");
  }
  const primaryNavigationMarkup = html.match(
    /<header class="vault-header">[\s\S]*?<nav class="sidebar-primary-nav" aria-label="Primary navigation">([\s\S]*?)<\/nav>/
  )?.[1];
  assert.ok(primaryNavigationMarkup, "Product Client HTML should keep primary navigation in the File sidebar header");
  for (const buttonId of [
    "ribbonFilesButton",
    "ribbonGraphButton",
    "ribbonSearchButton",
    "ribbonCommandButton",
    "ribbonAccessButton",
  ]) {
    assertIncludes(primaryNavigationMarkup, `id="${buttonId}"`, "Product Client primary navigation");
  }
  assertNotIncludes(html, "app-ribbon", "Product Client HTML");
  assert.ok(
    !/id="vaultInvitationPanel"[^>]*open/.test(html),
    "Product Client HTML should keep the Vault invitation panel closed by default"
  );
  assertNotIncludes(html, "graphFilterInput", "Product Client HTML");
  assertNotIncludes(html, "aria-label=\"Filter graph\"", "Product Client HTML");
  assertNotIncludes(html, "graph-icon-button", "Product Client HTML");
  assertNotIncludes(html, "readerModeButton", "Product Client HTML");
  assertNotIncludes(css, ".graph-controls", "Product Client CSS");
  assertNotIncludes(css, ".graph-icon-button", "Product Client CSS");
  assertNotIncludes(js, "graphFilterInput", "Product Client JS");
  assertNotIncludes(js, "readerMode", "Product Client JS");
  const deleteFolderHandler = js.match(
    /async function deleteFolderFromContextTarget\(target\) \{[\s\S]*?\n  \}\n\n  function /
  )?.[0];
  assert.ok(deleteFolderHandler, "Product Client JS should expose the delete-Folder handler");
  assert.match(
    deleteFolderHandler,
    /if \(\s*!actorHasDestructiveAuthority\(state\.metadata, currentActorNpub\(\)\)\s*\) \{\s*throw new Error\("Your Vault role cannot permanently delete Folders"\);\s*\}/,
    "Product Client delete-Folder handler must exit before deletion when authority is absent"
  );
  assert.ok(
    deleteFolderHandler.indexOf("actorHasDestructiveAuthority(state.metadata, currentActorNpub())") <
      deleteFolderHandler.indexOf("const result = await protectedRequest"),
    "Product Client delete-Folder handler must check authority before its destructive request"
  );
  assertIncludes(deleteFolderHandler, 'action: "delete-folder"', "Product Client delete-Folder handler");
  for (const legacyMarker of [
    "accessFolderViewButton",
    "accessVaultViewButton",
    "accessVaultPanel",
    "accessOverviewPanel",
    "accessFlowPanel",
    "accessTargetNpubInput",
    "grantFolderAccessButton",
    "removeFolderAccessButton",
    "folderKeyInput",
    "okfDestinationFolderInput",
    "okfConflictModeInput",
    "okfBundleInput",
    "encryptDraftButton",
  ]) {
    assertNotIncludes(html, `id="${legacyMarker}"`, "Product Client HTML");
  }

  for (const marker of [
    ".obsidian-shell",
    ".obsidian-shell[data-workspace-view=\"graph\"]",
    "[hidden]",
    "--shadow-access-ring",
    ".sidebar-primary-nav",
    ".obsidian-folder-button",
    ".obsidian-file-title",
    ".context-menu",
    ".command-palette-backdrop",
    ".command-palette-row",
    ".graph-stage",
    ".graph-floating-controls",
    ".graph-empty-state",
    ".graph-canvas.is-hovering",
    ".node.hover-active",
    ".edge.hover-connected",
    ".access-content-panel",
    ".access-folder-selector",
    ".folder-selector-button",
    ".folder-dropdown",
    ".access-inspector-new",
    ".access-basic-view",
    ".access-action-stack",
    ".access-state-stack",
    ".access-who-has-list",
    ".access-inline-form",
    ".access-person-info-button",
    ".access-person-detail-panel",
    ".access-advanced-section",
    ".access-advanced-summary::after",
    ".access-button-row",
    ".vault-access-action-grid",
    ".vault-access-option",
    ".vault-access-option-heading",
    ".vault-management-section",
    ".access-vault-admin",
    ".access-field",
    ".access-checkbox",
    ".access-share-hint",
    ".access-link-status",
    ".access-busy-status",
    ".vault-invite-url-output",
    ".access-content-panel.is-busy",
    ".access-badge",
    ".note-content-empty",
    ".note-markdown",
    ".editor-slash-menu",
    ".editor-slash-row",
    ".inline-page-editor",
    ".note-markdown table",
    ".note-markdown pre[data-language]",
    ".task-list",
    ".page-markdown-editor",
    ".page-save-button",
    ".internal-link",
  ]) {
    assertIncludes(css, marker, "Product Client CSS");
  }
  assertNotIncludes(css, ".app-ribbon", "Product Client CSS");

  for (const marker of [
    "buildGraphProjection",
    "graphLayout",
    "graphStats",
    "buildAdminAccessChangeEvent",
    "buildFolderKeyGrantRequest",
    "canonicalAdminAccessChangePayload",
    "commandPaletteRows",
    "visibleVaultOptions",
    "personalVaultIdForPubkey",
    "openManageVaultsModal",
    "openSettingsModal",
    "workspaceChromeState",
    "graphNeighborIds",
    "accessBadgesForFolder",
    "accessActionRoute",
    "accessIntentValue",
    "accessPanelState",
    "accessPeopleSummary",
    "identityMetadataForNpub",
    "vaultPeopleRows",
    "vaultInvitationRows",
    "folderShareLinkRows",
    "sharedFolderRelationshipRows",
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
    "buildEmailVaultInvitationRequest",
    "copyToClipboard",
    "copyVaultInviteUrl",
    "buildEmailInviteClaimRequest",
    "emailInviteBootstrapPath",
    "emailInviteClientUrl",
    "emailInviteClaimPath",
    "emailInviteScope",
    "openEmailInviteBootstrap",
    "inviteUnwrapKeypairFromSecret",
    "nip44DecryptWithSecret",
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
    "toggleMarkdownTask",
    "pageLinkContext",
    "readerFolderRows",
    "readerPageRows",
    "pagePathLabel",
    "readerPageDetail",
  ]) {
    assertIncludes(js, marker, "Product Client JS");
  }

  assert.match(
    html,
    /id="vaultInviteUrlOutput"[^>]*hidden/,
    "Product Client HTML must keep generated invite URLs hidden before an unlocked session creates one"
  );
  assert.match(
    html,
    /id="vaultInviteUrlInput"[\s\S]{0,180}type="text"[\s\S]{0,180}readonly/,
    "Product Client HTML must expose a generated invite URL as readable local output"
  );
  assert.match(
    html,
    /id="copyVaultInviteUrlButton"[^>]*aria-label="Copy client-only invite link"/,
    "Product Client HTML must name the client-only invite copy action"
  );
  assert.match(
    html,
    /id="vaultInviteSecretInput"[\s\S]{0,180}type="password"/,
    "Product Client HTML must keep manually entered Invite Secrets masked"
  );
  assert.match(
    js,
    /async function copyToClipboard\(text\)/,
    "Product Client JS must route copy actions through one safe helper"
  );
  assert.doesNotMatch(
    js,
    /log\("Copied (?:Page|Folder) ID\./,
    "Product Client JS must not log copied identifiers"
  );

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
