const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

function element() {
  const attributes = new Map();
  return {
    className: "",
    disabled: false,
    hidden: false,
    open: false,
    checked: false,
    dataset: {},
    style: {},
    textContent: "",
    value: "",
    children: [],
    classList: {
      add() {},
      remove() {},
      toggle() {},
    },
    appendChild(child) {
      this.children.push(child);
    },
    append(...children) {
      this.children.push(...children);
    },
    addEventListener() {},
    contains() {
      return false;
    },
    focus() {},
    getAttribute(name) {
      return attributes.get(name) ?? null;
    },
    querySelector() {
      return null;
    },
    querySelectorAll() {
      return [];
    },
    removeAttribute(name) {
      attributes.delete(name);
    },
    replaceChildren(...children) {
      this.children = children;
    },
    setAttribute(name, value) {
      attributes.set(name, String(value));
    },
    setSelectionRange() {},
  };
}

const elements = new Map();
const context = {
  TextDecoder,
  TextEncoder,
  Uint8Array,
  URLSearchParams,
  atob: (value) => Buffer.from(value, "base64").toString("binary"),
  btoa: (value) => Buffer.from(value, "binary").toString("base64"),
  console,
  crypto: crypto.webcrypto,
  document: {
    addEventListener() {},
    createElement: element,
    createTextNode(value) {
      return textNode(value);
    },
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, element());
      return elements.get(id);
    },
    querySelector() {
      return element();
    },
    querySelectorAll() {
      return [];
    },
    title: "FiniteBrain",
  },
  window: {
    __FINITE_BRAIN_DISABLE_AUTOSTART__: true,
  },
};
context.globalThis = context;

const source = fs.readFileSync(path.join(__dirname, "product-client.js"), "utf8");
const htmlSource = fs.readFileSync(path.join(__dirname, "product-client.html"), "utf8");
const cssSource = fs.readFileSync(path.join(__dirname, "product-client.css"), "utf8");
vm.runInNewContext(source, context, { filename: "product-client.js" });

const client = context.window.FiniteBrainProductClient;

function accessFailureTestSeams() {
  const testElements = new Map();
  const testContext = {
    ...context,
    document: {
      ...context.document,
      getElementById(id) {
        if (!testElements.has(id)) testElements.set(id, element());
        return testElements.get(id);
      },
    },
    window: {
      ...context.window,
      __FINITE_BRAIN_DISABLE_AUTOSTART__: true,
    },
  };
  testContext.globalThis = testContext;
  let seams = null;
  testContext.window.__FINITE_BRAIN_CAPTURE_TEST_SEAMS__ = (value) => {
    seams = value;
  };
  const seamSource = source.replace(
    "  return {\n    accessActionRoute,",
    "  window.__FINITE_BRAIN_CAPTURE_TEST_SEAMS__?.({ state, failAccessOperation, lockSession, lockSessionForVaultAccessChange, protectedRequest, reportClientActionFailure });\n\n  return {\n    accessActionRoute,"
  );
  assert.notEqual(seamSource, source, "The access-failure test must capture the Product Client's real closure seams");
  vm.runInNewContext(seamSource, testContext, { filename: "product-client-access-failure.test.js" });
  assert.ok(seams, "The Product Client must expose the captured access-failure seams to this deterministic test");
  return { context: testContext, elements: testElements, seams };
}

const prepareDraftWriteSource = source.slice(
  source.indexOf("async function prepareDraftWrite(options = {})"),
  source.indexOf("async function savePreparedPage()")
);
assert.match(
  prepareDraftWriteSource,
  /signEvent:\s*requireNip07SignEvent\(\),/,
  "Save must sign its Page revision through the session-aware NIP-07 adapter"
);

const deletePageFromContextTargetSource = source.slice(
  source.indexOf("async function deletePageFromContextTarget(target)"),
  source.indexOf("function selectReaderFolder(folderId, options = {})")
);
assert.match(
  deletePageFromContextTargetSource,
  /signEvent:\s*requireNip07SignEvent\(\),/,
  "Delete Page must sign its tombstone through the session-aware NIP-07 adapter"
);

const reportClientActionFailureSource = source.slice(
  source.indexOf("function reportClientActionFailure(error)"),
  source.indexOf("function markAccessFailureHandled(error)")
);
const failAccessOperationSource = source.slice(
  source.indexOf("function failAccessOperation(sessionEpoch, title, error, detail = (value) => value.message)"),
  source.indexOf("function finishAccessOperation(sessionEpoch)")
);
const createVaultInvitationFromPanelSource = source.slice(
  source.indexOf("async function createVaultInvitationFromPanel()"),
  source.indexOf("async function inspectVaultInvitationFromPanel()")
);
const protectedRequestSource = source.slice(
  source.indexOf("async function protectedRequest(path, options = {})"),
  source.indexOf("async function loadVisibleVaults()")
);
assert.match(
  reportClientActionFailureSource,
  /handledAccessFailures\.has\(error\)\) return;/,
  "A failure already shown by the access panel must not also show global feedback"
);
assert.match(
  failAccessOperationSource,
  /markAccessFailureHandled\(error\);\s*if \(!sessionOperationIsCurrent\(state\.sessionEpoch, sessionEpoch, state\.sessionStatus\)\) return;\s*setAccessResult\("error", title, detail\(error\)\);/s,
  "Access failures must stay in the existing access result and be suppressed before stale requests return"
);
assert.match(
  createVaultInvitationFromPanelSource,
  /catch \(error\) \{\s*markAccessFailureHandled\(error\);\s*if \(state\.sessionEpoch === sessionEpoch\)/s,
  "Invitation failures must be suppressed before a post-lock rethrow reaches global feedback"
);
assert.match(
  protectedRequestSource,
  /const error = protectedRequestError\(path, response\.status, body\);\s*lockSessionForVaultAccessChange\(error, sessionEpoch\);\s*throw error;/s,
  "Confirmed active-Vault authorization loss must lock before protected work can continue"
);

function objectIdCandidateBaseForTest(value) {
  return `obj_${String(value || "page")
    .trim()
    .toLowerCase()
    .replace(/\.md$/i, "")
    .replace(/[^a-z0-9_-]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 88) || "page"}`.padEnd(16, "0").slice(0, 112);
}

function textNode(value) {
  return { nodeType: 3, nodeValue: value };
}

function nodeTextContent(node) {
  if (node.nodeType === 3) return node.nodeValue || "";
  return (node.childNodes || []).map(nodeTextContent).join("");
}

function elementNode(tagName, children = [], attributes = {}) {
  return {
    nodeType: 1,
    tagName: tagName.toUpperCase(),
    childNodes: children,
    children: children.filter((child) => child.nodeType === 1),
    checked: Boolean(attributes.checked),
    className: attributes.className || "",
    dataset: attributes.dataset || {},
    style: attributes.style || {},
    textContent: attributes.textContent ?? children.map(nodeTextContent).join(""),
    type: attributes.type || "",
    getAttribute(name) {
      return attributes[name] || null;
    },
  };
}

assert.equal(client.deriveSignerState(null).status, "unavailable");
assert.equal(client.deriveSignerState({ getPublicKey() {} }).status, "unsupported");
assert.equal(
  client.deriveSignerState({
    getPublicKey() {},
    signEvent() {},
  }).status,
  "ready"
);

const folderRows = client.metadataFolderRows({
  folders: [
    {
      id: "general",
      path: "General",
      access: "all_members",
      accessUserIds: [],
      currentKeyVersion: 1,
      setupIncomplete: false,
      sharedFolderSource: false,
    },
    {
      id: "restricted",
      path: "Restricted",
      access: "restricted",
      accessUserIds: [],
      currentKeyVersion: 3,
      setupIncomplete: false,
      sharedFolderSource: true,
    },
  ],
});
assert.equal(folderRows[0].status, "ready");
assert.equal(folderRows[1].status, "locked");
assert.match(folderRows[1].detail, /source/);
assert.match(folderRows[1].detail, /locked/);
const badgeLabels = (badges) => Array.from(badges, (badge) => badge.label);
assert.deepEqual(
  badgeLabels(client.accessBadgesForFolder(folderRows[1], new Set(["restricted@3"]))),
  ["restricted", "shared", "locked", "key open", "v3"]
);
assert.deepEqual(
  badgeLabels(client.accessBadgesForFolder(folderRows[1], new Set(["restricted@2"]))),
  ["restricted", "shared", "locked", "v3"]
);
assert.deepEqual(
  badgeLabels(client.sidebarAccessBadgesForFolder(folderRows[0])),
  []
);
assert.deepEqual(
  badgeLabels(client.sidebarAccessBadgesForFolder(folderRows[1])),
  []
);
assert.equal(
  JSON.stringify(client.accessActionRoute("share-folder", { folderId: "restricted" })),
  JSON.stringify({ folderId: "restricted", intent: "links", sidebarMode: "access" })
);
assert.equal(
  JSON.stringify(client.accessActionRoute("manage-access", { folderId: "restricted" })),
  JSON.stringify({ folderId: "restricted", intent: "people", sidebarMode: "access" })
);
assert.equal(client.accessActionRoute("delete-folder", { folderId: "restricted" }), null);
assert.equal(client.accessIntentValue("share"), "links");
assert.equal(client.accessIntentValue("manage"), "people");
assert.equal(client.folderAllowsDirectGrant(folderRows[0]), true);
assert.equal(client.folderAllowsDirectGrant(folderRows[1]), true);
assert.equal(client.folderAllowsDirectGrant({ access: "admin_only" }), false);
assert.equal(client.folderAllowsDirectGrant({ access: "owner" }), false);
assert.equal(client.accessPanelState("links", folderRows[1]).status, "restricted");
assert.equal(client.accessPanelState("links", folderRows[1]).mode, "links");
assert.equal(client.accessPanelState("people", folderRows[1]).title, "Restricted");
assert.equal(client.accessPanelState("manage", folderRows[0]).title, "General");
assert.equal(client.accessPanelState("share", folderRows[0]).status, "all members");
assert.equal(
  client.accessPeopleSummary(folderRows[0], {
    admins: ["npub-admin"],
    members: ["npub-admin", "npub-member"],
  }),
  "2 members"
);
assert.match(htmlSource, /id="accessFolderButton"/);
assert.equal((htmlSource.match(/id="accessFolderButton"/g) || []).length, 1);
assert.equal((htmlSource.match(/id="accessFolderDropdown"/g) || []).length, 1);
assert.equal((htmlSource.match(/id="accessFolderList"/g) || []).length, 1);
assert.match(htmlSource, /id="accessAddPersonPanel"[\s\S]*class="access-folder-selector"[\s\S]*id="accessAddPersonForm"/);
assert.doesNotMatch(htmlSource, /class="access-folder-selector"[\s\S]*id="accessInspector"/);
assert.match(htmlSource, /id="accessWhoHasList"/);
assert.match(htmlSource, /class="access-action-stack"/);
assert.match(htmlSource, /class="access-state-stack"/);
assert.match(htmlSource, /id="accessAddPersonPanel"/);
assert.match(htmlSource, />\s*Grant folder access\s*</);
assert.match(htmlSource, />\s*Choose folder and Member Identity\s*</);
assert.match(htmlSource, /id="accessAdvancedSection"/);
assert.match(htmlSource, />\s*Restricted folder link\s*</);
assert.match(htmlSource, /id="accessSidebarCount"/);
assert.match(htmlSource, /id="accessShareHint"/);
assert.match(htmlSource, /id="accessShareMountHint"/);
assert.match(htmlSource, />\s*Show in recipient's Vault\s*</);
assert.doesNotMatch(htmlSource, />\s*Create personal mount\s*</);
assert.doesNotMatch(htmlSource, />\s*Folder \+ person\s*</);
assert.doesNotMatch(htmlSource, />\s*Single-use Folder access\s*</);
assert.doesNotMatch(htmlSource, />\s*Share with link\s*</);
assert.match(htmlSource, /placeholder="name@example\.com"/);
assert.doesNotMatch(htmlSource, /placeholder="[^"]*(npub|hex|NIP-05)/);
assert.match(htmlSource, /role="tablist"/);
assert.match(htmlSource, /id="accessFolderViewButton"/);
assert.match(htmlSource, /id="accessVaultViewButton"/);
assert.match(htmlSource, />\s*Vaults\s*</);
assert.match(htmlSource, />\s*Access\s*</);
assert.match(htmlSource, /id="accessFolderPanel"/);
assert.match(htmlSource, /id="accessVaultPanel"/);
assert.match(htmlSource, /id="vaultSwitchList"/);
assert.match(htmlSource, /id="vaultPeopleList"/);
assert.match(htmlSource, /id="vaultPeopleSection"/);
assert.match(htmlSource, /id="vaultPeopleActionPanel"/);
assert.match(htmlSource, /class="vault-access-action-grid"/);
assert.match(htmlSource, />\s*Give Vault access\s*</);
assert.match(htmlSource, />\s*Invite, add, promote\s*</);
assert.match(htmlSource, />\s*Vault Member Identities\s*</);
assert.match(htmlSource, />\s*Invite by email\s*</);
assert.match(htmlSource, />\s*Add member now\s*</);
assert.match(htmlSource, />\s*Make admin\s*</);
assert.match(htmlSource, />\s*Join with invite\s*</);
assert.match(htmlSource, />\s*Accept invite code\s*</);
assert.doesNotMatch(htmlSource, />\s*Invite, add, or promote\s*</);
assert.doesNotMatch(htmlSource, />\s*Accept received invite\s*</);
assert.doesNotMatch(htmlSource, />\s*Choose folder and person\s*</);
assert.doesNotMatch(htmlSource, />\s*Vault people\s*</);
assert.doesNotMatch(source, /Invite, add, or promote/);
assert.doesNotMatch(source, /Accept received invite/);
assert.doesNotMatch(source, /Admin-only controls/);
assert.match(source, /Member Identities or Links/);
assert.match(htmlSource, /id="folderShareLinkListSection"/);
assert.match(htmlSource, /id="vaultInvitationListSection"/);
assert.match(htmlSource, /id="sharedFolderSection"/);
assert.match(htmlSource, /id="accessCreateOrganizationPanel"/);
assert.match(htmlSource, /id="manageVaultCreateDetails"/);
assert.doesNotMatch(htmlSource, /id="vaultControlDetails"/);
assert.doesNotMatch(htmlSource, /id="vaultSelect"/);
assert.doesNotMatch(htmlSource, /id="connectSignerButton"/);
assert.match(htmlSource, /id="accessShareTargetInput"/);
assert.match(htmlSource, /id="addVaultMemberButton"/);
assert.match(htmlSource, /id="addVaultAdminButton"/);
assert.match(htmlSource, /id="vaultInvitationPanel" class="access-vault-admin"/);
assert.match(htmlSource, /id="vaultInvitationActionSection" class="access-admin-section vault-access-option primary"/);
assert.doesNotMatch(htmlSource, /id="vaultInvitationPanel"[^>]*open/);
assert.doesNotMatch(htmlSource, /id="accessChangeMode"/);
assert.doesNotMatch(htmlSource, /id="accessManageToggle"/);
assert.doesNotMatch(htmlSource, /id="accessManageSection"/);
assert.match(cssSource, /\[hidden\]\s*\{[^}]*display: none !important;/s);
assert.match(cssSource, /\.access-view-switch/);
assert.match(cssSource, /\.access-action-stack\s*\{[^}]*gap:\s*8px;/s);
assert.match(cssSource, /\.access-state-stack\s*\{[^}]*gap:\s*12px;/s);
assert.match(cssSource, /\.access-advanced-summary,\s*\.access-admin-summary\s*\{[^}]*grid-template-areas:/s);
assert.match(cssSource, /\.access-advanced-summary \.icon,\s*\.access-admin-summary \.icon\s*\{/s);
assert.match(cssSource, /#accessAddPersonPanel\s+\.access-folder-selector\s*\{[^}]*margin:\s*0;/s);
assert.match(cssSource, /#accessAddPersonPanel\s+\.access-advanced-content\s*\{[^}]*display:\s*grid;[^}]*gap:\s*12px;/s);
assert.match(cssSource, /\.access-checkbox-hint\s*\{[^}]*margin:\s*-6px 0 0 23px;/s);
assert.match(cssSource, /\.vault-management-section/);
assert.match(cssSource, /#accessSidebarPanel\s*\{[^}]*overflow-x:\s*hidden;/s);
assert.match(cssSource, /#accessSidebarPanel\s*\{[^}]*--access-panel-inset:\s*10px;/s);
assert.match(cssSource, /\.access-mode-panel\s*\{[^}]*overflow-x:\s*hidden;/s);
assert.match(cssSource, /\.access-who-has-list\s+li\s*\{[^}]*flex-wrap:\s*wrap;/s);
assert.match(cssSource, /\.access-button-row\s*\{[^}]*display:\s*grid;/s);
assert.doesNotMatch(cssSource, /\.vault-person-action\s*\{[^}]*min-width:\s*max-content/s);
assert.match(cssSource, /\.vault-management-section\s+\.access-who-has-list\s+li\s*\{[^}]*background:\s*transparent;[^}]*box-shadow:\s*none;/s);
assert.match(cssSource, /\.vault-access-action-grid\s*\{[^}]*gap:\s*10px;/s);
assert.match(cssSource, /#vaultPeopleActionPanel\s+\.access-inline-field\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\);/s);
assert.match(cssSource, /#vaultPeopleSection\s+\.access-person-name\s*\{[^}]*white-space:\s*normal;/s);
assert.match(cssSource, /#vaultInvitationPanel\s+\.access-button-row\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\);/s);
assert.match(cssSource, /\.vault-switch-list/);
assert.match(cssSource, /\.vault-switch-button/);
assert.match(cssSource, /\.access-vault-create/);
assert.match(cssSource, /\.vault-picker\s+\.vault-load-button/);
assert.doesNotMatch(cssSource, /\.file-sidebar:has\(> \.vault-control-strip/);
assert.match(cssSource, /\.file-sidebar\s*>\s*#accessSidebarPanel\s*\{[^}]*display:\s*none;/s);
assert.doesNotMatch(cssSource, /inset\s+2px\s+0/);
assert.doesNotMatch(cssSource, /\.ribbon-button\.active::before/);
assert.doesNotMatch(cssSource, /\.folder-dropdown\s*\{[^}]*position:\s*absolute/s);
assert.match(cssSource, /\.folder-option-button/);
assert.doesNotMatch(cssSource, /\.folder-dropdown-list\s+\.obsidian-folder-button/);
assert.equal(client.normalizeAccessView("vault"), "vault");
assert.equal(client.normalizeAccessView("folder"), "folder");
assert.equal(client.normalizeAccessView("other"), "folder");
assert.equal(client.hasOrganizationVaultControls({ kind: "personal" }), false);
assert.equal(client.hasOrganizationVaultControls({ kind: "organization" }), true);
assert.equal(client.showsCreateOrganizationControl({ kind: "personal" }), true);
assert.equal(client.showsCreateOrganizationControl({ kind: "organization" }), false);
const unresolvedOrgPeopleRows = client.vaultPeopleRows({
  kind: "organization",
  members: ["npub-admin", "npub-member"],
  admins: ["npub-admin"],
});
assert.equal(
  JSON.stringify(unresolvedOrgPeopleRows.map(({ id, name, role, status, type, removable }) => ({
    id,
    name,
    role,
    status,
    type,
    removable,
  }))),
  JSON.stringify([
    {
      id: "npub-admin",
      name: "npub-admin",
      role: "admin",
      status: "No email or NIP-05 metadata loaded",
      type: "admin",
      removable: true,
    },
    {
      id: "npub-member",
      name: "npub-member",
      role: "member",
      status: "No email or NIP-05 metadata loaded",
      type: "member",
      removable: true,
    },
  ])
);
assert.equal(
  JSON.stringify(unresolvedOrgPeopleRows[0].details.slice(0, 2)),
  JSON.stringify([
    { label: "Email / NIP-05", value: "Not resolved in this client" },
    { label: "Public key", value: "npub-admin" },
  ])
);
const unresolvedOwnerRows = client.vaultPeopleRows({
  kind: "personal",
  ownerUserId: "npub-owner",
});
assert.equal(
  JSON.stringify(unresolvedOwnerRows.map(({ id, name, role, status, type, removable }) => ({
    id,
    name,
    role,
    status,
    type,
    removable,
  }))),
  JSON.stringify([
    {
      id: "npub-owner",
      name: "npub-owner",
      role: "owner",
      status: "No email or NIP-05 metadata loaded",
      type: "owner",
      removable: false,
    },
  ])
);
assert.equal(
  JSON.stringify(client.vaultHealthBadges(
    {
      kind: "organization",
      folders: [{ id: "getting-started" }],
      grantCount: 2,
      mountedFolders: [{ id: "mount-1" }],
    },
    "connected"
  ).map((badge) => badge.label)),
  JSON.stringify(["signer connected", "organization", "1 folders", "2 grants", "1 mounts"])
);
assert.equal(client.personalVaultIdForPubkey("ab".repeat(32)), "personal-abababababababab");
assert.equal(
  client.normalizeVisibleVault({
    vaultId: "acme",
    kind: "organization",
    name: "Acme",
    role: "invited",
    inviteCode: "invite-acme",
  }).inviteCode,
  "invite-acme"
);
assert.equal(
  JSON.stringify(client.visibleVaultOptions([
    { vaultId: "acme", kind: "organization", name: "Acme", role: "admin" },
    { vaultId: "personal-ab", kind: "personal", name: "Personal vault", role: "owner" },
  ]).map((vault) => [vault.vaultId, vault.kind, vault.role])),
  JSON.stringify([
    ["personal-ab", "personal", "owner"],
    ["acme", "organization", "admin"],
  ])
);

const projection = client.createClientProjection();
projection.localDrafts.set("general/obj_draft", {
  baseRevision: 0,
  path: "obj_draft.md",
  text: "# Draft Page\n\nUnsaved but visible.",
});
const draftPages = client.projectionPagesFromProjection(projection);
assert.equal(draftPages.length, 1);
assert.equal(draftPages[0].folderId, "general");
assert.equal(draftPages[0].localDraft, true);
assert.equal(draftPages[0].status, "ready");
assert.equal(client.readerPageRows("general", draftPages)[0].label, "Draft Page");

const sessionKeyring = client.createSessionKeyring();
sessionKeyring.keys.set("vault:general:1", { rawKey: "raw-folder-key-sentinel" });
sessionKeyring.openedGrants.push({
  folderId: "general",
  keyVersion: 1,
  vaultId: "vault",
});
const sessionProjection = client.createClientProjection();
sessionProjection.pages.set("general/page", {
  folderId: "general",
  objectId: "page",
  status: "ready",
  text: "decrypted-page-sentinel",
});
sessionProjection.localDrafts.set("general/draft", {
  baseRevision: 1,
  path: "draft.md",
  text: "plaintext-draft-sentinel",
});
sessionProjection.conflicts.push({ plaintext: "conflict-plaintext-sentinel" });
const sessionState = {
  accessResult: { detail: "member-access-sentinel" },
  folderShareLinks: [{ id: "share-link-sentinel" }],
  identityByNpub: new Map([["npub-member", { display: "member@example.com" }]]),
  keyring: sessionKeyring,
  lastEmailInvitePostProof: { invitedEmail: "invitee@example.com" },
  lastEmailInviteSecret: "invite-secret-sentinel",
  lastEmailInviteUrl: "https://finite.test/#inviteSecret=invite-secret-sentinel",
  metadata: { name: "private-vault-sentinel" },
  okfPlan: { entries: [{ markdown: "okf-plaintext-sentinel" }] },
  preparedWrite: { envelopeJson: "encrypted-write", plaintext: "prepared-plaintext-sentinel" },
  preparedWriteTarget: { folderId: "general", objectId: "draft" },
  projection: sessionProjection,
  sessionStatus: "unlocked",
  signerStatus: "connected",
  vaultInvitations: [{ invitedEmail: "invitee@example.com" }],
};
client.clearSessionSecretsAndPlaintext(sessionState);
assert.equal(sessionState.sessionStatus, "locked");
assert.equal(sessionState.signerStatus, "connected");
assert.equal(sessionState.keyring, null);
assert.equal(sessionKeyring.keys.size, 0);
assert.equal(sessionKeyring.openedGrants.length, 0);
assert.equal(sessionState.metadata, null);
assert.equal(sessionState.projection.pages.size, 0);
assert.equal(sessionState.projection.localDrafts.size, 0);
assert.equal(sessionState.projection.conflicts.length, 0);
assert.equal(sessionState.preparedWrite, null);
assert.equal(sessionState.preparedWriteTarget, null);
assert.equal(sessionState.okfPlan, null);
assert.equal(sessionState.identityByNpub.size, 0);
assert.equal(sessionState.accessResult, null);
assert.equal(sessionState.vaultInvitations, null);
assert.equal(sessionState.folderShareLinks, null);
assert.equal(sessionState.lastEmailInviteSecret, null);
assert.equal(sessionState.lastEmailInviteUrl, null);
assert.equal(sessionState.lastEmailInvitePostProof, null);
assert.equal(
  JSON.stringify(client.sessionStatusView("locked")),
  JSON.stringify({
    action: "Unlock session",
    detail: "Folder Keys and temporary plaintext are cleared. Unlock to reopen encrypted grants.",
    locked: true,
    title: "Session locked",
  })
);
assert.equal(
  JSON.stringify(client.sessionStatusView("unlocked")),
  JSON.stringify({
    action: "Lock session",
    detail: "Readable content and Session Folder Keys are held in memory for this session.",
    locked: false,
    title: "Session unlocked",
  })
);
assert.equal(
  JSON.stringify(client.sessionStatusView("resuming")),
  JSON.stringify({
    action: "Lock session",
    detail: "Opening encrypted Folder Key Grants and rebuilding the temporary client view.",
    locked: false,
    title: "Unlocking session",
  })
);
const activeVaultAccessLoss = client.protectedRequestError(
  "/_admin/vaults/acme/metadata",
  403,
  { error: "vault access required" }
);
assert.equal(activeVaultAccessLoss.status, 403);
assert.equal(activeVaultAccessLoss.reason, "vault access required");
assert.equal(activeVaultAccessLoss.path, "/_admin/vaults/acme/metadata");
for (const path of [
  "/_admin/vaults/acme/metadata",
  "/_admin/vaults/acme/export",
  "/_admin/vaults/acme/sync/bootstrap",
]) {
  assert.equal(
    client.isActiveVaultAuthorizationLoss(
      client.protectedRequestError(path, 403, { error: "vault access required" }),
      "acme"
    ),
    true,
    `A confirmed membership loss must lock for the active Vault state read ${path}`
  );
}
for (const [status, reason, path] of [
  [401, "vault access required", "/_admin/vaults/acme/metadata"],
  [403, "replayed Nostr authorization event", "/_admin/vaults/acme/metadata"],
  [403, "stale Nostr event timestamp", "/_admin/vaults/acme/metadata"],
  [403, "vault admin access required", "/_admin/vaults/acme/invitations"],
  [403, "folder access required", "/_admin/vaults/acme/folders/restricted/objects/page"],
  [403, "vault access required", "/_admin/vaults/other/metadata"],
]) {
  assert.equal(
    client.isActiveVaultAuthorizationLoss(client.protectedRequestError(path, status, { error: reason }), "acme"),
    false,
    `Only a confirmed active-Vault membership loss may lock the session (${status} ${reason} ${path})`
  );
}
assert.match(htmlSource, /id="sessionSecurityStatus"[^>]*aria-live="polite"/);
assert.match(htmlSource, /id="sessionSecurityTitle"[^>]*>Session locked</);
assert.match(htmlSource, /id="resumeSessionButton"[^>]*>Unlock session</);
assert.match(htmlSource, /id="lockSessionButton"[^>]*>Lock session</);
assert.match(
  htmlSource,
  /id="clientActionFeedback"[^>]*role="status"[^>]*aria-live="polite"[^>]*aria-atomic="true"/
);
assert.match(cssSource, /\.client-action-feedback\[hidden\]\s*\{\s*display:\s*none;/);
assert.match(
  cssSource,
  /@media \(max-width: 1180px\) \{[\s\S]*?\.obsidian-shell\s*\{[\s\S]*?grid-template-rows:\s*minmax\(0, 1fr\) auto;/,
  "The compact shell must retain the status-feedback row"
);
assert.doesNotMatch(source, /window\.alert/);
assert.match(htmlSource, /id="sessionAccountVaultButton"[^>]*aria-haspopup="menu"/);
assert.match(htmlSource, /id="sessionAccountVaultButton"[^>]*aria-controls="vaultSwitcherMenu"/);
assert.match(htmlSource, /id="vaultSwitcherMenu"[^>]*role="menu"/);
assert.match(htmlSource, /id="vaultSwitcherList"/);
assert.match(htmlSource, /id="manageVaultsButton"/);
assert.match(source, /sessionAccountVaultButton[\s\S]{0,120}openVaultSwitcher\(\)/);
assert.doesNotMatch(source, /sessionAccountVaultButton[\s\S]{0,180}openSettingsModal\("vault"\)/);
assert.match(htmlSource, /id="manageVaultsModal"[^>]*role="dialog"[^>]*aria-modal="true"/s);
assert.match(htmlSource, /id="manageVaultsList"/);
assert.match(htmlSource, /id="manageVaultsLoadButton"/);
assert.match(htmlSource, /id="manageVaultsLoadButton"[^>]*>Unlock Vault</);
assert.match(htmlSource, /id="accessLoadVaultButton"[^>]*>Unlock Vault</);
assert.match(htmlSource, /id="manageVaultsConnectSignerButton"/);
assert.match(htmlSource, /id="manageCreateOrganizationVaultButton"/);
assert.match(source, /manageVaultsButton[\s\S]{0,120}openManageVaultsModal\(\)/);
assert.match(source, /manageVaultsLoadButton[\s\S]{0,120}manageVaultsLoadAction\(\)/);
assert.match(
  source,
  /onOptionalClick\("accessLoadVaultButton",[\s\S]{0,180}state\.sessionStatus === SESSION_STATUS\.LOCKED \? resumeSession\(\) : loadVaultReader\(\)/
);
assert.match(htmlSource, /id="sessionSettingsButton"[^>]*aria-haspopup="dialog"/);
assert.match(
  htmlSource,
  /id="sessionSettingsButton"[\s\S]{0,900}<circle cx="12" cy="12" r="3"\s*\/>[\s\S]{0,900}M19\.4 15a1\.65 1\.65/,
);
assert.match(htmlSource, /id="settingsModal"[^>]*role="dialog"[^>]*aria-modal="true"/s);
assert.match(htmlSource, /id="settingsNavSession"[^>]*role="tab"/);
assert.match(htmlSource, /id="settingsNavVault"[^>]*role="tab"/);
assert.match(htmlSource, /id="settingsNavAccess"[^>]*role="tab"[^>]*aria-controls="settingsAccessPanel"/);
assert.match(htmlSource, /id="settingsNavInvitations"[^>]*role="tab"[^>]*aria-controls="settingsInvitationsPanel"/);
assert.match(htmlSource, /id="settingsSessionPanel"[^>]*role="tabpanel"/);
assert.match(htmlSource, /id="settingsVaultPanel"[^>]*role="tabpanel"/);
assert.match(htmlSource, /id="settingsAccessPanel"[^>]*role="tabpanel"/);
assert.match(htmlSource, /id="settingsAccessPanelMount"/);
assert.match(htmlSource, /id="settingsInvitationsPanel"[^>]*role="tabpanel"/);
assert.match(htmlSource, /id="settingsInvitationsPanelMount"/);
assert.match(htmlSource, /id="settingsConnectSignerButton"/);
assert.match(source, /openSettingsModal\("session"\)/);
assert.match(source, /settingsNavAccess[\s\S]{0,120}setSettingsSection\("access"\)/);
assert.match(source, /settingsNavInvitations[\s\S]{0,120}setSettingsSection\("invitations"\)/);
assert.match(source, /mountAccessPanelInSettings\(\)/);
assert.match(source, /mountInvitationPanelInSettings\(\)/);
assert.match(source, /mount\.appendChild\(panel\)/);
assert.match(source, /for \(const node of invitationNodes\) \{[\s\S]{0,160}mount\.appendChild\(node\)/);
assert.match(source, /start\(\) \{[\s\S]{0,180}mountAccessPanelInSettings\(\);[\s\S]{0,120}mountInvitationPanelInSettings\(\);/);
assert.match(source, /state\.settingsSection = "invitations"/);
assert.match(source, /nextMode === "access"[\s\S]{0,100}openSettingsModal\("access"\)/);
assert.doesNotMatch(source, /\$\("accessSidebarPanel"\)\.hidden = mode !== "access"/);
assert.match(source, /state\.settingsModalOpen && state\.settingsSection === "access"[\s\S]{0,100}refreshAccessManagementListsInBackground\(\)/);
assert.match(source, /closeSettingsModal\(\)/);
assert.match(cssSource, /\.settings-modal-backdrop\s*\{/);
assert.match(
  cssSource,
  /\.settings-modal-panel\s*\{[^}]*border:\s*1px solid var\(--line-strong\);[^}]*border-radius:\s*var\(--radius-popover\);[^}]*background:\s*var\(--surface-raised\);[^}]*box-shadow:\s*var\(--shadow-obsi-popover\);/s,
);
assert.match(
  cssSource,
  /\.command-palette-backdrop\s*\{[^}]*display:\s*grid;[^}]*align-items:\s*start;[^}]*justify-items:\s*center;[^}]*padding:\s*max\(24px, calc\(\(100vh - 480px\) \/ 2\)\) 24px 24px;/s,
);
assert.match(
  cssSource,
  /\.graph-topbar #graphStats\s*\{[^}]*font-variant-numeric:\s*tabular-nums;[^}]*padding:\s*2px 8px;/s,
);
assert.match(htmlSource, /id="zoomInGraphButton"[^>]*title="Zoom in"/);
assert.match(htmlSource, /id="zoomOutGraphButton"[^>]*title="Zoom out"/);
assert.doesNotMatch(htmlSource, /id="zoomInGraphButton"[\s\S]{0,180}<circle/);
assert.doesNotMatch(htmlSource, /id="zoomOutGraphButton"[\s\S]{0,180}<circle/);
assert.match(htmlSource, /id="fitGraphButton"[^>]*title="Reset zoom"/);
assert.match(htmlSource, /id="fullscreenGraphButton"[^>]*title="Enter full screen"/);
assert.doesNotMatch(htmlSource, /id="resetGraphButton"/);
assert.doesNotMatch(htmlSource, /id="renderGraphButton"/);
assert.doesNotMatch(htmlSource, /id="replayGraphButton"/);
assert.doesNotMatch(htmlSource, /id="toggleGraphHistoryButton"/);
assert.doesNotMatch(htmlSource, /id="replayList"/);
assert.match(source, /requestFullscreen\(\)/);
assert.match(source, /document\.addEventListener\("fullscreenchange", updateGraphFullscreenControl\)/);
assert.match(source, /zoomGraphView\(1\)/);
assert.match(source, /zoomGraphView\(-1\)/);
assert.match(cssSource, /\.graph-icon-button,[\s\S]*?width:\s*40px;[\s\S]*?min-height:\s*40px;/);
assert.match(cssSource, /\.graph-icon-button:active:not\(:disabled\),[\s\S]*?transform:\s*scale\(0\.96\);/);
assert.match(cssSource, /\.settings-modal-layout\s*\{[^}]*grid-template-columns:/s);
assert.match(cssSource, /\.settings-invitations-section\s*\{/);
assert.match(cssSource, /#settingsInvitationsPanelMount\s*\{/);
assert.match(cssSource, /@media \(max-width: 640px\)/);
assert.match(cssSource, /\.settings-modal-layout\s*\{[^}]*display:\s*flex;/s);
assert.match(htmlSource, /<span class="pill ready">email invite<\/span>/);
assert.doesNotMatch(htmlSource, /<span class="pill ready">new Member Identity<\/span>/);
assert.match(source, /clearSessionSecretsAndPlaintext\(state\)/);
assert.equal(client.sessionGrantOpeningAllowed("locked"), false);
assert.equal(client.sessionGrantOpeningAllowed("resuming"), true);
assert.equal(client.sessionGrantOpeningAllowed("unlocked"), true);
assert.equal(
  JSON.stringify(client.lockedVaultSelection("locked", "org-acme", [])),
  JSON.stringify({ label: "Selected Vault (locked)", value: "org-acme" })
);
assert.equal(client.lockedVaultSelection("unlocked", "org-acme", []), null);
assert.equal(client.lockedVaultSelection("locked", "org-acme", [{ vaultId: "org-acme" }]), null);
assert.equal(
  client.missingVisibleVaultFallback(
    "unlocked",
    "org-acme",
    [{ vaultId: "personal-a", kind: "personal" }],
    "aa".repeat(32),
    "personal"
  ),
  "personal-a"
);
assert.equal(
  client.missingVisibleVaultFallback(
    "locked",
    "org-acme",
    [{ vaultId: "personal-a", kind: "personal" }],
    "aa".repeat(32),
    "personal"
  ),
  null
);
assert.equal(
  client.missingVisibleVaultFallback(
    "unlocked",
    "personal-aaaaaaaaaaaaaaaa",
    [],
    "aa".repeat(32),
    "personal"
  ),
  null
);
assert.equal(
  client.missingVisibleVaultFallback(
    "resuming",
    "personal-aaaaaaaaaaaaaaaa",
    [{ vaultId: "org-testr-mr9bmjs", kind: "organization" }],
    "aa".repeat(32),
    "personal"
  ),
  "org-testr-mr9bmjs"
);
assert.deepEqual(
  Array.from(
    client.withActiveVaultOption(
      [{ vaultId: "personal-a", kind: "personal", name: "Personal vault", role: "owner" }],
      "org-acme",
      { vaultId: "org-acme", kind: "organization", name: "Acme", role: "admin" }
    ),
    (vault) => vault.vaultId
  ),
  ["personal-a", "org-acme"]
);
assert.equal(client.signerIdentityChanged(null, "aa".repeat(32)), false);
assert.equal(client.signerIdentityChanged("aa".repeat(32), "aa".repeat(32)), false);
assert.equal(client.signerIdentityChanged("aa".repeat(32), "bb".repeat(32)), true);
assert.equal(
  client.signedEventMatchesPinnedIdentity("aa".repeat(32), { pubkey: "aa".repeat(32) }),
  true
);
assert.equal(
  client.signedEventMatchesPinnedIdentity("aa".repeat(32), { pubkey: "bb".repeat(32) }),
  false
);
assert.equal(client.signedEventMatchesPinnedIdentity(null, { pubkey: "aa".repeat(32) }), false);
assert.equal(client.sessionOperationIsCurrent(4, 4, "unlocked"), true);
assert.equal(client.sessionOperationIsCurrent(5, 4, "unlocked"), false);
assert.equal(client.sessionOperationIsCurrent(4, 4, "locked"), false);
const originalKeyring = client.createSessionKeyring();
originalKeyring.keys.set("vault/folder@1", "key-sentinel");
originalKeyring.openedGrants.push({ id: "grant-sentinel" });
const clonedKeyring = client.cloneSessionKeyring(originalKeyring);
originalKeyring.keys.clear();
originalKeyring.openedGrants.length = 0;
assert.equal(clonedKeyring.keys.get("vault/folder@1"), "key-sentinel");
assert.equal(clonedKeyring.openedGrants[0].id, "grant-sentinel");
assert.deepEqual(
  JSON.parse(JSON.stringify(client.inviteNavigationFromHash("#invite&code=invite-1&email=MEMBER%40Example.com&inviteSecret=secret-1"))),
  {
    inviteCode: "invite-1",
    inviteEmail: "member@example.com",
    inviteSecret: "secret-1",
  }
);
assert.equal(client.inviteNavigationFromHash("#section=invite-free"), null);
assert.equal(client.inviteNavigationFromHash("#code=unrelated"), null);
const originalWindowLocation = context.window.location;
const originalWindowHistory = context.window.history;
let inviteFallbackUrl = null;
context.window.location = {
  hash: "#invite&inviteCode=invite-missing-history&inviteSecret=secret-must-not-import",
  href: "https://finite.test/client#invite&inviteCode=invite-missing-history&inviteSecret=secret-must-not-import",
  pathname: "/client",
  search: "",
  replace(url) {
    inviteFallbackUrl = url;
    this.hash = "";
    this.href = `https://finite.test${url}`;
  },
};
context.window.history = {};
assert.equal(client.populateInviteFromHash(), false);
assert.equal(inviteFallbackUrl, "/client");
assert.equal(client.applyPendingInviteNavigation(), false);
assert.equal(context.document.getElementById("vaultInviteSecretInput").value, "");
inviteFallbackUrl = null;
context.window.location.hash =
  "#invite&inviteCode=invite-rejected-history&inviteSecret=second-secret-must-not-import";
context.window.location.href =
  "https://finite.test/client#invite&inviteCode=invite-rejected-history&inviteSecret=second-secret-must-not-import";
context.window.history = {
  replaceState() {
    throw new Error("history replacement denied");
  },
};
assert.equal(client.populateInviteFromHash(), false);
assert.equal(inviteFallbackUrl, "/client");
assert.equal(client.applyPendingInviteNavigation(), false);
assert.equal(context.document.getElementById("vaultInviteSecretInput").value, "");
context.window.location = originalWindowLocation;
context.window.history = originalWindowHistory;
assert.match(source, /return loadVaultReader\(\{ allowResume: true \}\);/);
assert.match(source, /window\.addEventListener\?\.\("pagehide", handlePageHide\)/);
assert.match(source, /window\.addEventListener\?\.\("pageshow", handlePageShow\)/);
assert.match(source, /openFolderKeyGrants\(keyring, exported, expectedRecipient, \{[\s\S]{0,120}assertCurrent/);
assert.match(source, /state\.sessionStatus = SESSION_STATUS\.UNLOCKED;[\s\S]{0,160}applyPendingInviteNavigation\(\)/);
assert.match(
  source,
  /setOptionalDisabled\([\s\S]{0,120}loadVaultButton[\s\S]{0,180}state\.sessionStatus === SESSION_STATUS\.RESUMING/
);
for (const [surface, pattern] of [
  ["localStorage", /\blocalStorage\b/],
  ["sessionStorage", /\bsessionStorage\b/],
  ["IndexedDB", /\bindexedDB\b/],
  ["Cache Storage", /\b(?:caches|CacheStorage)\b/],
  ["cookies", /\bdocument\.cookie\b/],
  ["window.name", /\bwindow\.name\b/],
  ["storage manager", /\bnavigator\.storage\b/],
  ["history push", /\bhistory\??\.pushState\b/],
]) {
  assert.doesNotMatch(source, pattern, `Product Client must not write session plaintext through ${surface}`);
}
const historyReplacements = source.match(/window\.history\.replaceState\([^\n]+/g) || [];
assert.equal(historyReplacements.length, 1);
assert.match(historyReplacements[0], /replaceState\(null, "", fallbackUrl\)/);
assert.equal((source.match(/console\.(?:debug|info|log|warn|error)\(/g) || []).length, 1);
assert.match(source, /console\.debug\(`\[FiniteBrain\] \$\{message\}`\);/);
assert.match(source, /SESSION_PLAINTEXT_INPUT_IDS/);
assert.match(source, /"folderKeyInput"/);
assert.match(source, /"okfBundleInput"/);
assert.match(source, /"pageDraftInput"/);
assert.match(source, /"vaultInviteSecretInput"/);

(async () => {
  const event = await client.buildAuthEventTemplate(
    "post",
    "http://finite.test/_admin/vaults/smoke/metadata",
    "{\"name\":\"Smoke\"}"
  );
  assert.equal(event.kind, 27235);
  assert.deepEqual(Array.from(event.tags[0]), [
    "u",
    "http://finite.test/_admin/vaults/smoke/metadata",
  ]);
  assert.deepEqual(Array.from(event.tags[1]), ["method", "POST"]);
  assert.equal(event.tags[2][0], "payload");
  assert.equal(event.tags[2][1].length, 64);

  const keyring = client.createSessionKeyring();
  const folderKey = Buffer.alloc(32, 7).toString("base64");
  await client.openFolderKeyGrantPlaintext(keyring, {
    version: "finite-folder-key-grant-v1",
    vaultId: "smoke",
    folderId: "general",
    keyVersion: 1,
    issuerNpub: "npub-issuer",
    recipientNpub: "npub-recipient",
    folderKey,
    issuedAt: "2026-06-24T00:00:00.000Z",
  });
  assert.equal(keyring.openedGrants.length, 1);
  await client.openFolderKeyGrantPlaintext(keyring, {
    version: "finite-folder-key-grant-v1",
    vaultId: "smoke",
    folderId: "general",
    keyVersion: 1,
    issuerNpub: "npub-issuer",
    recipientNpub: "npub-recipient",
    folderKey,
    issuedAt: "2026-06-24T00:00:00.000Z",
  });
  assert.equal(keyring.openedGrants.length, 1);

  const authorNpub = client.npubFromHex("00".repeat(32));
  const otherNpub = client.npubFromHex("11".repeat(32));
  assert.match(authorNpub, /^npub1/);
  assert.equal(client.npubToHex(authorNpub), "00".repeat(32));
  assert.equal(client.publicKeyIdentityFromInput("11".repeat(32)).npub, otherNpub);
  assert.equal(client.publicKeyIdentityFromInput(otherNpub).hex, "11".repeat(32));
  client.rememberIdentity({
    npub: authorNpub,
    hex: "00".repeat(32),
    display: "alice@example.com",
    nip05: "alice@example.com",
    relays: ["wss://relay.example.com"],
    verifiedAt: "2026-07-06T00:00:00Z",
  });
  assert.equal(client.identityDisplay(authorNpub), "alice@example.com");
  assert.equal(client.identityDisplay(otherNpub), client.shortKey(otherNpub));
  assert.equal(
    client.vaultPeopleRows({
      kind: "organization",
      members: [authorNpub, otherNpub],
      admins: [authorNpub],
    })[0].name,
    "alice@example.com"
  );
  assert.equal(
    client.vaultPeopleRows({
      kind: "organization",
      members: [authorNpub, otherNpub],
      admins: [authorNpub],
    })[1].name,
    client.shortKey(otherNpub)
  );

  const devGrant = {
    id: "dev-grant",
    folderId: "general",
    keyVersion: 1,
    recipientNpub: authorNpub,
    wrappedEventJson: JSON.stringify({
      kind: 1059,
      content: JSON.stringify({
        version: "finite-folder-key-grant-v1",
        vaultId: "smoke",
        folderId: "general",
        keyVersion: 1,
        issuerNpub: "npub-issuer",
        recipientNpub: authorNpub,
        folderKey,
        issuedAt: "2026-06-24T00:00:00.000Z",
      }),
    }),
  };
  assert.equal(
    client.plaintextDevelopmentGrantFromExportGrant(devGrant, authorNpub).folderId,
    "general"
  );
  assert.equal(client.plaintextDevelopmentGrantFromExportGrant(devGrant, otherNpub), null);
  const hardenedDevOpen = await client.openFolderKeyGrants(
    client.createSessionKeyring(),
    { keyGrants: [devGrant] },
    authorNpub,
    { decrypt: async () => "{}" }
  );
  assert.equal(hardenedDevOpen.opened.length, 0);
  assert.equal(hardenedDevOpen.skipped.length, 1);
  const devKeyring = client.createSessionKeyring();
  const devOpen = await client.openDevelopmentFolderKeyGrants(
    devKeyring,
    { keyGrants: [devGrant, { id: "opaque", wrappedEventJson: "{\"kind\":1059}" }] },
    authorNpub
  );
  assert.equal(devOpen.opened.length, 1);
  assert.equal(devOpen.skipped.length, 1);
  assert.equal(devKeyring.openedGrants.length, 1);

  const accessPayload = {
    vaultId: "smoke",
    changeId: "access-change-test",
    action: "grant-folder-access",
    adminNpub: authorNpub,
    folderId: "restricted",
    targetNpub: authorNpub,
    keyVersion: 2,
    createdAt: "2026-06-23T00:02:00Z",
  };
  assert.equal(
    client.canonicalAdminAccessChangePayload(accessPayload),
    `{"version":"finite-vault-admin-access-change-v1","vaultId":"smoke","changeId":"access-change-test","action":"grant-folder-access","adminNpub":"${authorNpub}","folderId":"restricted","targetNpub":"${authorNpub}","keyVersion":2,"createdAt":"2026-06-23T00:02:00Z"}`
  );
  assert.equal(
    JSON.stringify(client.adminAccessChangeTags(accessPayload)),
    JSON.stringify([
      ["d", "finite-vault-admin-access-change:smoke:access-change-test"],
      ["vault", "smoke"],
      ["action", "grant-folder-access"],
      ["folder", "restricted"],
      ["p", "00".repeat(32)],
      ["keyVersion", "2"],
    ])
  );

  const fakeEncrypt = async (_pubkey, plaintext) =>
    `nip44:${Buffer.from(plaintext, "utf8").toString("base64url")}`;
  const fakeDecrypt = async (_pubkey, ciphertext) => {
    if (!String(ciphertext).startsWith("nip44:")) throw new Error("bad fake ciphertext");
    return Buffer.from(String(ciphertext).slice("nip44:".length), "base64url").toString("utf8");
  };
  const localSignerSecret = "1".padStart(64, "0");
  const peerSignerSecret = "2".padStart(64, "0");
  const localSigner = client.createLocalNip07ProviderFromSecret(localSignerSecret);
  const peerSigner = client.createLocalNip07ProviderFromSecret(peerSignerSecret);
  const localPublicKey = await localSigner.getPublicKey();
  const peerPublicKey = await peerSigner.getPublicKey();
  assert.equal(localPublicKey, client.inviteUnwrapKeypairFromSecret(localSignerSecret).publicKeyHex);
  const localSigned = await localSigner.signEvent({
    kind: 27235,
    created_at: 1780000000,
    tags: [["u", "http://finite.test/_admin/vaults"]],
    content: "",
  });
  assert.equal(localSigned.pubkey, localPublicKey);
  assert.match(localSigned.id, /^[0-9a-f]{64}$/);
  assert.match(localSigned.sig, /^[0-9a-f]{128}$/);
  const localToPeer = await localSigner.nip44.encrypt(peerPublicKey, "hello peer");
  assert.equal(await peerSigner.nip44.decrypt(localPublicKey, localToPeer), "hello peer");
  const peerToLocal = await peerSigner.nip44.encrypt(localPublicKey, "hello local");
  assert.equal(await localSigner.nip44.decrypt(peerPublicKey, peerToLocal), "hello local");
  assert.equal(
    await client.nip44DecryptWithSecret(peerSignerSecret, localPublicKey, localToPeer),
    "hello peer"
  );
  let grantSignedIndex = 0;
  context.window.nostr = {
    signEvent: async (template) => ({
      ...template,
      id: `signed-event-${++grantSignedIndex}`,
      pubkey: "00".repeat(32),
      sig: "signed-event-signature",
    }),
    nip44: {
      decrypt: fakeDecrypt,
      encrypt: fakeEncrypt,
    },
  };
  const accessEvent = await client.buildAdminAccessChangeEvent({
    ...accessPayload,
    createdAtUnix: Date.parse(accessPayload.createdAt) / 1000,
  });
  assert.equal(accessEvent.kind, 30078);
  assert.equal(JSON.stringify(accessEvent.tags), JSON.stringify(client.adminAccessChangeTags(accessPayload)));
  assert.equal(accessEvent.content, client.canonicalAdminAccessChangePayload(accessPayload));
  let providerBoundSignCalls = 0;
  const providerBoundSigner = {
    signEvent(template) {
      if (this !== providerBoundSigner) {
        throw new TypeError("Cannot read properties of undefined (reading 'enable')");
      }
      providerBoundSignCalls += 1;
      return {
        ...template,
        id: "provider-bound-access-change",
        pubkey: "00".repeat(32),
        sig: "provider-bound-signature",
      };
    },
  };
  const providerSignedAccessEvent = await client.buildAdminAccessChangeEvent({
    ...accessPayload,
    changeId: "access-change-provider-bound",
    provider: providerBoundSigner,
    createdAtUnix: Date.parse(accessPayload.createdAt) / 1000,
  });
  assert.equal(providerBoundSignCalls, 1);
  assert.equal(providerSignedAccessEvent.id, "provider-bound-access-change");

  assert.equal(
    JSON.stringify(client.initialVaultInvitationFolders("getting-started restricted getting-started")),
    JSON.stringify(["getting-started", "restricted"])
  );
  assert.equal(
    JSON.stringify(
      client.buildVaultInvitationRequest({
      targetNpub: otherNpub,
      initialFolderAccess: "getting-started,restricted getting-started",
      expiresAt: "2026-07-04T00:00:00.000Z",
      })
    ),
    JSON.stringify({
      targetNpub: otherNpub,
      initialFolderAccess: ["getting-started", "restricted"],
      expiresAt: "2026-07-04T00:00:00.000Z",
    })
  );
  assert.equal(client.vaultInvitationCreatePath("smoke org"), "/_admin/vaults/smoke%20org/invitations");
  assert.equal(client.vaultInvitationLinkPath("invite/code"), "/_admin/vault-invitation-links/invite%2Fcode");
  assert.equal(client.vaultInvitationAcceptPath("invite/code"), "/_admin/vault-invitation-links/invite%2Fcode/accept");
  assert.equal(client.emailInviteBootstrapPath("invite/code"), "/_admin/vault-invitation-links/invite%2Fcode/bootstrap");
  assert.equal(client.emailInviteInstructionsPath("invite/code"), "/_admin/vault-invitation-links/invite%2Fcode/instructions");
  assert.equal(client.emailInviteClaimPath("invite/code"), "/_admin/vault-invitation-links/invite%2Fcode/claim");
  assert.equal(
    client.emailInviteClientUrl({
      publicBaseUrl: "https://finite.test/app/",
      inviteCode: "invite/code",
      invitedEmail: "Friend@Example.com",
      inviteSecret: "secret-value",
    }),
    "https://finite.test/app/client#inviteCode=invite%2Fcode&inviteEmail=friend%40example.com&inviteSecret=secret-value"
  );
  assert.equal(client.vaultInvitationIdentifierHint("invite-0fe6eda60e1bf6e662acb8e2b5c425d9"), null);
  assert.match(
    client.vaultInvitationIdentifierHint("invitation-4f82a37c1b82bcdd54973c466cdde914"),
    /invitation id/
  );
  assert.match(client.vaultInvitationIdentifierHint("4f82a37c1b82bcdd54973c466cdde914"), /start with invite-/);
  assert.match(
    client.vaultInvitationUnavailableDetail(new Error("vault invitation unavailable")),
    /Check the Invite Code, active signer/
  );
  assert.equal(
    client.vaultInvitationRevokePath("smoke org", "invitation/one"),
    "/_admin/vaults/smoke%20org/invitations/invitation%2Fone"
  );
  assert.match(htmlSource, /id="vaultInviteUrlInput"/);
  assert.match(htmlSource, /id="vaultInviteEmailInput"/);
  assert.match(htmlSource, /id="vaultInviteEmailProofCreatedAtInput"/);
  assert.match(htmlSource, /id="vaultInviteSecretInput"/);
  assert.match(htmlSource, /id="vaultInviteConnectSignerButton"/);
  assert.match(htmlSource, /id="getEmailInviteInstructionsButton"/);

  const nip44VectorSender = client.inviteUnwrapKeypairFromSecret("2".padStart(64, "0"));
  assert.equal(
    await client.nip44DecryptWithSecret(
      "1".padStart(64, "0"),
      nip44VectorSender.publicKeyHex,
      "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABee0G5VSK0/9YypIObAtDKfYEAjD35uVkHyB0F4DwrcNaCXlCWZKaArsGrY6M9wnuTMxWfp1RTN9Xga8no+kF5Vsb"
    ),
    "a"
  );

  const emailMetadata = {
    folders: [
      {
        id: "getting-started",
        path: "Getting Started",
        access: "all_members",
        currentKeyVersion: 1,
      },
      {
        id: "restricted",
        path: "Restricted",
        access: "restricted",
        currentKeyVersion: 3,
      },
      {
        id: "vault-ops",
        path: "Vault Ops",
        access: "admin_only",
        currentKeyVersion: 1,
      },
    ],
  };
  assert.equal(client.canonicalInviteEmail(" Friend@Example.COM "), "friend@example.com");
  assert.equal(
    JSON.stringify(
      client.emailInviteScope(emailMetadata, "restricted").map((folder) => [
        folder.folderId,
        folder.access,
        folder.keyVersion,
      ])
    ),
    JSON.stringify([
      ["getting-started", "all_members", 1],
      ["restricted", "restricted", 3],
    ])
  );
  assert.throws(() => client.emailInviteScope(emailMetadata, "vault-ops"), /all-members and selected restricted/);

  const emailKeyring = client.createSessionKeyring();
  const restrictedEmailFolderKey = Buffer.alloc(32, 8).toString("base64");
  await client.openFolderKeyGrantPlaintext(emailKeyring, {
    version: "finite-folder-key-grant-v1",
    vaultId: "smoke",
    folderId: "getting-started",
    keyVersion: 1,
    issuerNpub: authorNpub,
    recipientNpub: authorNpub,
    folderKey,
    issuedAt: "2026-06-24T00:00:00.000Z",
  });
  await client.openFolderKeyGrantPlaintext(emailKeyring, {
    version: "finite-folder-key-grant-v1",
    vaultId: "smoke",
    folderId: "restricted",
    keyVersion: 3,
    issuerNpub: authorNpub,
    recipientNpub: authorNpub,
    folderKey: restrictedEmailFolderKey,
    issuedAt: "2026-06-24T00:00:00.000Z",
  });
  const inviteKeypair = client.inviteUnwrapKeypairFromSecret("3".padStart(64, "0"));
  let emailInviteSignedIndex = 0;
  const emailInviteSigner = async (template) => ({
    ...template,
    id: `email-invite-signed-${++emailInviteSignedIndex}`,
    pubkey: "00".repeat(32),
    sig: "email-invite-signature",
  });
  const emailInviteRequest = await client.buildEmailVaultInvitationRequest(emailKeyring, {
    createdAtUnix: 1780000400,
    expiresAt: "2026-07-04T00:00:00.000Z",
    grantIdFactory: (item) => `bootstrap-${item.folderId}`,
    initialFolderAccess: "restricted",
    inviteKeypair,
    issuerNpub: authorNpub,
    metadata: emailMetadata,
    provider: { signEvent: emailInviteSigner, nip44: { encrypt: fakeEncrypt, decrypt: fakeDecrypt } },
    signEvent: emailInviteSigner,
    target: "FRIEND@EXAMPLE.COM",
    vaultId: "smoke",
  });
  assert.equal(emailInviteRequest.body.target, "friend@example.com");
  assert.equal(emailInviteRequest.body.inviteUnwrapNpub, inviteKeypair.npub);
  assert.match(emailInviteRequest.body.bootstrapPayloadHash, /^sha256:[0-9a-f]{64}$/);
  assert.equal(JSON.stringify(emailInviteRequest.body.initialFolderAccess), JSON.stringify(["restricted"]));
  assert.equal(
    JSON.stringify(emailInviteRequest.scope.map((folder) => [folder.folderId, folder.access, folder.keyVersion])),
    JSON.stringify([
      ["getting-started", "all_members", 1],
      ["restricted", "restricted", 3],
    ])
  );
  assert.doesNotMatch(JSON.stringify(emailInviteRequest.body), /inviteSecret/i);
  const scopedEmailInviteRequest = await client.buildEmailVaultInvitationRequest(emailKeyring, {
    createdAtUnix: 1780000401,
    expiresAt: "2026-07-04T00:00:00.000Z",
    grantIdFactory: (item) => `scoped-bootstrap-${item.folderId}`,
    inviteKeypair,
    issuerNpub: authorNpub,
    provider: { signEvent: emailInviteSigner, nip44: { encrypt: fakeEncrypt, decrypt: fakeDecrypt } },
    scope: emailInviteRequest.scope,
    signEvent: emailInviteSigner,
    target: "friend@example.com",
    vaultId: "smoke",
  });
  assert.equal(JSON.stringify(scopedEmailInviteRequest.body.initialFolderAccess), JSON.stringify(["restricted"]));

  const emailInvitation = {
    vaultId: "smoke",
    inviteCode: "invite-email",
    inviteUnwrapNpub: inviteKeypair.npub,
    bootstrapPayloadHash: emailInviteRequest.body.bootstrapPayloadHash,
    bootstrapWrappedEventJson: emailInviteRequest.body.bootstrapWrappedEventJson,
    bootstrapScope: emailInviteRequest.scope,
  };
  const openedEmailBootstrap = await client.openEmailInviteBootstrap(emailInvitation, {
    email: "friend@example.com",
    inviteSecret: emailInviteRequest.inviteSecret,
    inviteDecrypt: fakeDecrypt,
  });
  assert.equal(
    JSON.stringify(openedEmailBootstrap.payload.grants.map((entry) => entry.folderId)),
    JSON.stringify(["getting-started", "restricted"])
  );
  await assert.rejects(
    () =>
      client.openEmailInviteBootstrap(emailInvitation, {
        email: "friend@example.com",
        inviteSecret: "4".padStart(64, "0"),
        inviteDecrypt: fakeDecrypt,
      }),
    /Invite Secret does not match/
  );
  await assert.rejects(
    () =>
      client.openEmailInviteBootstrap(emailInvitation, {
        email: "other@example.com",
        inviteSecret: emailInviteRequest.inviteSecret,
        inviteDecrypt: fakeDecrypt,
      }),
    /email mismatch/
  );
  await assert.rejects(
    () =>
      client.openEmailInviteBootstrap(
        {
          ...emailInvitation,
          bootstrapPayloadHash: "sha256:" + "0".repeat(64),
        },
        {
          email: "friend@example.com",
          inviteSecret: emailInviteRequest.inviteSecret,
          inviteDecrypt: fakeDecrypt,
        }
      ),
    /payload hash mismatch/
  );
  await assert.rejects(
    () =>
      client.openEmailInviteBootstrap(
        {
          ...emailInvitation,
          bootstrapScope: emailInvitation.bootstrapScope.slice(0, 1),
        },
        {
          email: "friend@example.com",
          inviteSecret: emailInviteRequest.inviteSecret,
          inviteDecrypt: fakeDecrypt,
        }
      ),
    /scope mismatch/
  );
  await assert.rejects(
    () =>
      client.openEmailInviteBootstrap(
        {
          ...emailInvitation,
          inviteUnwrapNpub: client.inviteUnwrapKeypairFromSecret("5".padStart(64, "0")).npub,
        },
        {
          email: "friend@example.com",
          inviteSecret: emailInviteRequest.inviteSecret,
          inviteDecrypt: fakeDecrypt,
        }
      ),
    /Invite Secret does not match/
  );
  let emailClaimSignedIndex = 0;
  const emailClaimSigner = async (template) => ({
    ...template,
    id: `email-claim-signed-${++emailClaimSignedIndex}`,
    pubkey: "11".repeat(32),
    sig: "email-claim-signature",
  });
  const emailClaimRequest = await client.buildEmailInviteClaimRequest({
    claimantNpub: otherNpub,
    claimGrantIdFactory: (entry) => `claim-${entry.folderId}`,
    createdAtUnix: 1780000500,
    email: "friend@example.com",
    emailProofCreatedAt: "2026-06-23T00:00:00.000Z",
    invitation: emailInvitation,
    inviteDecrypt: fakeDecrypt,
    inviteSecret: emailInviteRequest.inviteSecret,
    provider: { signEvent: emailClaimSigner, nip44: { encrypt: fakeEncrypt, decrypt: fakeDecrypt } },
    signEvent: emailClaimSigner,
  });
  assert.equal(emailClaimRequest.openedGrantCount, 2);
  assert.equal(emailClaimRequest.body.email, "friend@example.com");
  assert.equal(
    JSON.stringify(emailClaimRequest.body.grants.map((entry) => [entry.folderId, entry.grant.id, entry.grant.recipientNpub])),
    JSON.stringify([
      ["getting-started", "claim-getting-started", otherNpub],
      ["restricted", "claim-restricted", otherNpub],
    ])
  );
  const inviteProof = JSON.parse(emailClaimRequest.body.inviteUnwrapProofEventJson);
  assert.equal(inviteProof.pubkey, inviteKeypair.publicKeyHex);
  assert.match(inviteProof.content, /finite-email-invite-bootstrap-claim-proof-v1/);
  assert.doesNotMatch(JSON.stringify(emailClaimRequest.body), new RegExp(emailInviteRequest.inviteSecret, "i"));
  assert.doesNotMatch(JSON.stringify(emailClaimRequest.body), /folderKey/);

  const claimedKeyring = client.createSessionKeyring();
  const claimedOpen = await client.openFolderKeyGrants(
    claimedKeyring,
    {
      keyGrants: emailClaimRequest.body.grants.map((entry) => ({
        ...entry.grant,
        folderId: entry.folderId,
      })),
    },
    otherNpub,
    { decrypt: fakeDecrypt }
  );
  assert.equal(claimedOpen.opened.length, 2);
  assert.equal(claimedOpen.skipped.length, 0);
  assert.equal(claimedKeyring.keys.has("smoke:getting-started:1"), true);
  assert.equal(claimedKeyring.keys.has("smoke:restricted:3"), true);
  assert.equal(claimedKeyring.keys.has("smoke:vault-ops:1"), false);

  const claimedSharedWrite = await client.buildPageWriteRequest(claimedKeyring, {
    authorNpub: otherNpub,
    baseRevision: null,
    createdAtUnix: 1780000600,
    folderId: "getting-started",
    keyVersion: 1,
    nonceBytes: new Uint8Array(12).fill(4),
    objectId: "obj_email_shared01",
    plaintext: "# Shared Email Invite Page\n\nOpened through the claimed all-members grant.",
    signEvent: emailClaimSigner,
    vaultId: "smoke",
  });
  const openedClaimedShared = await client.openFolderObject(claimedKeyring, {
    vaultId: "smoke",
    folderId: "getting-started",
    objectId: "obj_email_shared01",
    revision: 1,
    ciphertext: claimedSharedWrite.ciphertext,
  });
  assert.equal(openedClaimedShared.status, "ready");
  assert.match(openedClaimedShared.text, /claimed all-members grant/);

  const claimedRestrictedWrite = await client.buildPageWriteRequest(claimedKeyring, {
    authorNpub: otherNpub,
    baseRevision: null,
    createdAtUnix: 1780000601,
    folderId: "restricted",
    keyVersion: 3,
    nonceBytes: new Uint8Array(12).fill(5),
    objectId: "obj_email_restricted01",
    plaintext: "# Restricted Email Invite Page\n\nOpened through the selected restricted grant.",
    signEvent: emailClaimSigner,
    vaultId: "smoke",
  });
  const openedClaimedRestricted = await client.openFolderObject(claimedKeyring, {
    vaultId: "smoke",
    folderId: "restricted",
    objectId: "obj_email_restricted01",
    revision: 1,
    ciphertext: claimedRestrictedWrite.ciphertext,
  });
  assert.equal(openedClaimedRestricted.status, "ready");
  assert.match(openedClaimedRestricted.text, /selected restricted grant/);
  await assert.rejects(
    () =>
      client.buildPageWriteRequest(claimedKeyring, {
        authorNpub: otherNpub,
        baseRevision: null,
        createdAtUnix: 1780000602,
        folderId: "vault-ops",
        keyVersion: 1,
        nonceBytes: new Uint8Array(12).fill(6),
        objectId: "obj_email_locked01",
        plaintext: "# Locked\n\nThis should not encrypt.",
        signEvent: emailClaimSigner,
        vaultId: "smoke",
      }),
    /No Folder Key opened/
  );

  const accessGrant = await client.buildFolderKeyGrantRequest({
    id: "grant-test",
    vaultId: "smoke",
    folderId: "restricted",
    keyVersion: 2,
    folderKey,
    issuerNpub: authorNpub,
    recipientNpub: authorNpub,
    createdAtUnix: 1780000000,
  });
  assert.equal(accessGrant.id, "grant-test");
  assert.equal(accessGrant.recipientNpub, authorNpub);
  const wrappedGrant = JSON.parse(accessGrant.wrappedEventJson);
  assert.equal(wrappedGrant.kind, 1059);
  assert.deepEqual(wrappedGrant.tags, [["p", "00".repeat(32)]]);
  assert.notEqual(wrappedGrant.content[0], "{");
  const sealEvent = JSON.parse(await fakeDecrypt(wrappedGrant.pubkey, wrappedGrant.content));
  assert.equal(sealEvent.kind, 13);
  const rumorEvent = JSON.parse(await fakeDecrypt(sealEvent.pubkey, sealEvent.content));
  assert.equal(rumorEvent.kind, 30078);
  assert.match(rumorEvent.id, /^[0-9a-f]{64}$/);
  const grantPlaintext = JSON.parse(rumorEvent.content);
  assert.equal(grantPlaintext.folderId, "restricted");
  assert.equal(grantPlaintext.folderKey, folderKey);
  const hardenedKeyring = client.createSessionKeyring();
  const hardenedOpen = await client.openFolderKeyGrants(
    hardenedKeyring,
    {
      keyGrants: [
        {
          id: "grant-test",
          folderId: "restricted",
          keyVersion: 2,
          recipientNpub: authorNpub,
          wrappedEventJson: accessGrant.wrappedEventJson,
        },
      ],
    },
    authorNpub,
    { decrypt: fakeDecrypt }
  );
  assert.equal(hardenedOpen.opened.length, 1);
  assert.equal(hardenedOpen.skipped.length, 0);
  assert.equal(hardenedKeyring.openedGrants[0].folderId, "restricted");
  let providerEncryptCalls = 0;
  let providerDecryptCalls = 0;
  const providerBackedNostr = {
    signEvent: context.window.nostr.signEvent,
    nip44: {
      encrypt(pubkey, plaintext) {
        if (!this.provider) throw new TypeError("Cannot read properties of undefined (reading 'enable')");
        providerEncryptCalls += 1;
        return fakeEncrypt(pubkey, plaintext);
      },
      decrypt(pubkey, ciphertext) {
        if (!this.provider) throw new TypeError("Cannot read properties of undefined (reading 'enable')");
        providerDecryptCalls += 1;
        return fakeDecrypt(pubkey, ciphertext);
      },
    },
  };
  const providerBoundGrant = await client.buildFolderKeyGrantRequest({
    id: "grant-provider-backed",
    vaultId: "smoke",
    folderId: "restricted",
    keyVersion: 2,
    folderKey,
    issuerNpub: authorNpub,
    provider: providerBackedNostr,
    recipientNpub: authorNpub,
    signEvent: providerBackedNostr.signEvent,
    createdAtUnix: 1780000001,
  });
  assert.equal(providerEncryptCalls, 2);
  const providerBoundOpen = await client.openFolderKeyGrants(
    client.createSessionKeyring(),
    {
      keyGrants: [
        {
          id: "grant-provider-backed",
          folderId: "restricted",
          keyVersion: 2,
          recipientNpub: authorNpub,
          wrappedEventJson: providerBoundGrant.wrappedEventJson,
        },
      ],
    },
    authorNpub,
    { provider: providerBackedNostr }
  );
  assert.equal(providerBoundOpen.opened.length, 1);
  assert.equal(providerBoundOpen.skipped.length, 0);
  assert.equal(providerDecryptCalls, 2);
  let boundProviderEncryptCalls = 0;
  const boundNip44Prototype = {
    encrypt(pubkey, plaintext) {
      if (!this.provider) throw new TypeError("Cannot read properties of undefined (reading 'enable')");
      boundProviderEncryptCalls += 1;
      return fakeEncrypt(pubkey, plaintext);
    },
  };
  const boundProviderNip44 = Object.create(boundNip44Prototype);
  boundProviderNip44.encrypt = boundNip44Prototype.encrypt.bind(boundProviderNip44);
  const boundWrapperNostr = {
    signEvent: context.window.nostr.signEvent,
    nip44: boundProviderNip44,
  };
  const boundWrapperGrant = await client.buildFolderKeyGrantRequest({
    id: "grant-bound-wrapper",
    vaultId: "smoke",
    folderId: "restricted",
    keyVersion: 2,
    folderKey,
    issuerNpub: authorNpub,
    provider: boundWrapperNostr,
    recipientNpub: authorNpub,
    signEvent: boundWrapperNostr.signEvent,
    createdAtUnix: 1780000002,
  });
  assert.equal(boundWrapperGrant.id, "grant-bound-wrapper");
  assert.equal(boundProviderEncryptCalls, 2);

  assert.equal(
    JSON.stringify(client.defaultVaultBootstrapFolderIds("personal")),
    JSON.stringify(["getting-started", "restricted"])
  );
  assert.equal(
    JSON.stringify(client.defaultVaultBootstrapFolderIds("organization")),
    JSON.stringify(["getting-started", "restricted"])
  );
  assert.equal(client.defaultVaultPagesFolderId("personal"), "getting-started");
  assert.equal(client.defaultVaultPagesFolderId("organization"), "getting-started");
  assert.match(htmlSource, /id="vaultInviteFoldersInput"\s+value="getting-started"/);
  assert.match(htmlSource, />Invite code<\/span>/);
  assert.doesNotMatch(htmlSource, /Invite code or id/);
  assert.match(htmlSource, /id="pageFolderIdInput" value="getting-started"/);
  assert.match(htmlSource, /id="okfDestinationFolderInput" value="getting-started"/);
  const defaultPages = client.defaultVaultPages("organization");
  assert.equal(
    JSON.stringify(defaultPages.slice(0, 5).map((page) => [page.folderId, page.objectId, page.path])),
    JSON.stringify([
      ["getting-started", "obj_default_agents", "AGENTS.md"],
      ["getting-started", "obj_default_humans", "HUMANS.md"],
      ["getting-started", "obj_default_getting-started_scope_config", "config.md"],
      ["getting-started", "obj_default_getting-started_scope_index", "_index.md"],
      ["getting-started", "obj_default_getting-started_scope_log", "log.md"],
    ])
  );
  assert.equal(defaultPages.length, 12);
  assert.equal(new Set(defaultPages.map((page) => page.objectId)).size, defaultPages.length);
  assert.equal(defaultPages.some((page) => page.folderId === "vault-ops"), false);
  assert.equal(defaultPages.some((page) => page.folderId === "product"), false);
  const gettingStartedReadme = defaultPages.find((page) => page.path === "README.md");
  assert.match(gettingStartedReadme?.markdown || "", /Default Folders/);
  assert.match(gettingStartedReadme?.markdown || "", /encrypted Assets under/);
  assert.match(gettingStartedReadme?.markdown || "", /Source Note/);
  const restrictedExamplePage = defaultPages.find((page) => page.path === "wiki/restricted-folder-example.md");
  assert.equal(restrictedExamplePage?.folderId, "restricted");
  assert.match(defaultPages[0].markdown, /Use `fbrain`/);
  assert.match(defaultPages[0].markdown, /LLM Wiki Rules/);
  assert.match(defaultPages[0].markdown, /raw\/assets\//);
  assert.match(defaultPages[0].markdown, /Source Note/);
  assert.match(defaultPages[1].markdown, /private, encrypted knowledge workspace/);
  assert.match(defaultPages[1].markdown, /Source Notes/);
  assert.match(defaultPages[2].markdown, /raw\/assets\//);
  assert.match(defaultPages[2].markdown, /Source Note/);
  const seedGraphPages = defaultPages.map((page) => ({
    ...page,
    key: `${page.folderId}/${page.objectId}`,
    status: "ready",
    text: page.markdown,
  }));
  const missingSeedLinks = seedGraphPages.flatMap((page) =>
    client
      .pageLinkContext(page, seedGraphPages)
      .outgoing.filter((row) => row.status !== "resolved")
      .map((row) => `${page.folderId}/${page.path}->${row.label}`)
  );
  assert.equal(JSON.stringify(missingSeedLinks), JSON.stringify([]));
  assert.equal(
    seedGraphPages.filter((page) => client.extractPageLinks(page.markdown).length > 0).length,
    defaultPages.length
  );
  assert.equal(
    seedGraphPages.filter((page) => client.pageLinkContext(page, seedGraphPages).backlinks.length > 0).length,
    defaultPages.length
  );
  assert.match(defaultPages[3].markdown, /# Getting Started Index/);
  assert.match(defaultPages[9].markdown, /# Restricted Index/);

  let bootstrapSignedIndex = 0;
  const bootstrapSigner = async (template) => ({
    ...template,
    id: `bootstrap-signed-${++bootstrapSignedIndex}`,
    pubkey: "00".repeat(32),
    sig: "bootstrap-signature",
  });
  const orgBootstrapPlan = await client.buildVaultBootstrapPlan({
    actorNpub: authorNpub,
    createdAtUnix: 1780000200,
    kind: "organization",
    provider: { signEvent: bootstrapSigner, nip44: { encrypt: fakeEncrypt, decrypt: fakeDecrypt } },
    rawKeysByFolderId: {
      "getting-started": new Uint8Array(32).fill(12),
      restricted: new Uint8Array(32).fill(13),
    },
    signEvent: bootstrapSigner,
    vaultId: "org-smoke",
  });
  assert.equal(
    JSON.stringify(orgBootstrapPlan.bootstrapGrants.map((entry) => entry.folderId)),
    JSON.stringify(["getting-started", "restricted"])
  );
  assert.equal(orgBootstrapPlan.defaultFolderId, "getting-started");
  assert.equal(orgBootstrapPlan.keyring.keys.has("org-smoke:getting-started:1"), true);
  assert.equal(orgBootstrapPlan.keyring.keys.has("org-smoke:restricted:1"), true);
  const starterWrites = await client.buildDefaultVaultPageWrites({
    actorNpub: authorNpub,
    createdAtUnix: 1780000300,
    kind: "organization",
    keyring: orgBootstrapPlan.keyring,
    nonceFactory: (index) => new Uint8Array(12).fill(index + 1),
    signEvent: bootstrapSigner,
    vaultId: "org-smoke",
  });
  assert.equal(
    JSON.stringify(
      starterWrites.slice(0, 8).map((write) => [write.folderId, write.objectId, write.targetPath])
    ),
    JSON.stringify([
      ["getting-started", "obj_default_agents", "AGENTS.md"],
      ["getting-started", "obj_default_humans", "HUMANS.md"],
      ["getting-started", "obj_default_getting-started_scope_config", "config.md"],
      ["getting-started", "obj_default_getting-started_scope_index", "_index.md"],
      ["getting-started", "obj_default_getting-started_scope_log", "log.md"],
      ["getting-started", "obj_default_getting-started_readme", "README.md"],
      ["getting-started", "obj_default_getting-started_how_finitebrain_works", "wiki/how-finitebrain-works.md"],
      ["getting-started", "obj_default_getting-started_access_and_folders", "wiki/access-and-folders.md"],
    ])
  );
  assert.equal(starterWrites.length, 12);
  assert.equal(new Set(starterWrites.map((write) => write.objectId)).size, starterWrites.length);
  assert.equal(starterWrites.some((write) => write.folderId === "vault-ops"), false);
  const openedAgentsDefault = await client.openFolderObject(orgBootstrapPlan.keyring, {
    vaultId: "org-smoke",
    folderId: "getting-started",
    objectId: "obj_default_agents",
    revision: 1,
    ciphertext: starterWrites[0].body.ciphertext,
  });
  assert.equal(openedAgentsDefault.status, "ready");
  assert.equal(openedAgentsDefault.path, "AGENTS.md");
  assert.match(openedAgentsDefault.text, /FiniteBrain vault/);
  assert.match(openedAgentsDefault.text, /raw\/assets\//);
  assert.match(openedAgentsDefault.text, /Source Note/);
  const openedHumansDefault = await client.openFolderObject(orgBootstrapPlan.keyring, {
    vaultId: "org-smoke",
    folderId: "getting-started",
    objectId: "obj_default_humans",
    revision: 1,
    ciphertext: starterWrites[1].body.ciphertext,
  });
  assert.equal(openedHumansDefault.status, "ready");
  assert.equal(openedHumansDefault.path, "HUMANS.md");
  assert.match(openedHumansDefault.text, /private, encrypted knowledge workspace/);
  assert.match(openedHumansDefault.text, /Source Notes/);

  const wrongRecipientOpen = await client.openFolderKeyGrants(
    client.createSessionKeyring(),
    {
      keyGrants: [
        {
          id: "grant-test",
          folderId: "restricted",
          keyVersion: 2,
          recipientNpub: authorNpub,
          wrappedEventJson: accessGrant.wrappedEventJson,
        },
      ],
    },
    otherNpub,
    { decrypt: fakeDecrypt }
  );
  assert.equal(wrongRecipientOpen.opened.length, 0);
  assert.match(wrongRecipientOpen.skipped[0].error, /not addressed/);
  const malformedShellOpen = await client.openFolderKeyGrants(
    client.createSessionKeyring(),
    {
      keyGrants: [
        {
          id: "malformed-shell",
          folderId: "restricted",
          keyVersion: 2,
          recipientNpub: authorNpub,
          wrappedEventJson: JSON.stringify({
            kind: 1059,
            pubkey: "00".repeat(32),
            tags: [["p", "00".repeat(32)]],
            content: "",
          }),
        },
      ],
    },
    authorNpub,
    { decrypt: fakeDecrypt }
  );
  assert.equal(malformedShellOpen.opened.length, 0);
  assert.match(malformedShellOpen.skipped[0].error, /wrapper content is missing/);
  const malformedSealOpen = await client.openFolderKeyGrants(
    client.createSessionKeyring(),
    {
      keyGrants: [
        {
          id: "malformed-seal",
          folderId: "restricted",
          keyVersion: 2,
          recipientNpub: authorNpub,
          wrappedEventJson: JSON.stringify({
            kind: 1059,
            pubkey: "00".repeat(32),
            tags: [["p", "00".repeat(32)]],
            content: await fakeEncrypt(
              "00".repeat(32),
              JSON.stringify({ kind: 14, pubkey: "00".repeat(32), content: "sealed" })
            ),
          }),
        },
      ],
    },
    authorNpub,
    { decrypt: fakeDecrypt }
  );
  assert.equal(malformedSealOpen.opened.length, 0);
  assert.match(malformedSealOpen.skipped[0].error, /seal must be kind 13/);
  const malformedRumorOpen = await client.openFolderKeyGrants(
    client.createSessionKeyring(),
    {
      keyGrants: [
        {
          id: "malformed-rumor",
          folderId: "restricted",
          keyVersion: 2,
          recipientNpub: authorNpub,
          wrappedEventJson: JSON.stringify({
            kind: 1059,
            pubkey: "00".repeat(32),
            tags: [["p", "00".repeat(32)]],
            content: await fakeEncrypt(
              "00".repeat(32),
              JSON.stringify({
                kind: 13,
                pubkey: "00".repeat(32),
                content: await fakeEncrypt(
                  "00".repeat(32),
                  JSON.stringify({ kind: 1, pubkey: "00".repeat(32), content: "{}" })
                ),
              })
            ),
          }),
        },
      ],
    },
    authorNpub,
    { decrypt: fakeDecrypt }
  );
  assert.equal(malformedRumorOpen.opened.length, 0);
  assert.match(malformedRumorOpen.skipped[0].error, /rumor must be kind 30078/);

  const write = await client.buildPageWriteRequest(keyring, {
    authorNpub,
    baseRevision: null,
    createdAtUnix: 1780000000,
    folderId: "general",
    keyVersion: 1,
    nonceBytes: new Uint8Array(12),
    objectId: "obj_000000000001",
    plaintext: "# Hello\n\nEncrypted locally.",
    signEvent: async (template) => ({
      ...template,
      id: "revision-event-id",
      pubkey: "00".repeat(32),
      sig: "revision-signature",
    }),
    vaultId: "smoke",
  });
  assert.equal(write.baseRevision, null);
  assert.equal(write.keyVersion, 1);
  assert.equal(write.cipher, "AES-256-GCM");
  assert.equal(write.revisionEvent.kind, 30078);
  assert.equal(
    JSON.stringify(write.revisionEvent.tags),
    JSON.stringify([
      ["d", "finite-folder-object-revision:smoke:general:obj_000000000001:1"],
      ["vault", "smoke"],
      ["folder", "general"],
      ["object", "obj_000000000001"],
      ["operation", "create"],
      ["keyVersion", "1"],
    ])
  );
  assert.match(write.revisionEvent.content, /finite-folder-object-revision-v1/);
  assert.match(write.revisionEvent.content, /ciphertextHash/);

  const deleteRequest = await client.buildPageDeleteRequest({
    authorNpub,
    baseRevision: 3,
    createdAtUnix: 1780000002,
    folderId: "general",
    objectId: "obj_000000000001",
    signEvent: async (template) => ({
      ...template,
      id: "tombstone-event-id",
      pubkey: "00".repeat(32),
      sig: "tombstone-signature",
    }),
    vaultId: "smoke",
  });
  assert.equal(deleteRequest.baseRevision, 3);
  assert.equal(deleteRequest.tombstoneEvent.kind, 30078);
  assert.equal(
    JSON.stringify(deleteRequest.tombstoneEvent.tags),
    JSON.stringify([
      ["d", "finite-folder-object-tombstone:smoke:general:obj_000000000001:4"],
      ["vault", "smoke"],
      ["folder", "general"],
      ["object", "obj_000000000001"],
      ["operation", "delete"],
    ])
  );
  assert.match(deleteRequest.tombstoneEvent.content, /finite-folder-object-tombstone-v1/);
  assert.match(deleteRequest.tombstoneEvent.content, /"baseRevision":3/);

  const openedPage = await client.openFolderObject(keyring, {
    vaultId: "smoke",
    folderId: "general",
    objectId: "obj_000000000001",
    revision: 1,
    ciphertext: write.ciphertext,
  });
  assert.equal(openedPage.status, "ready");
  assert.equal(openedPage.text, "# Hello\n\nEncrypted locally.");

  const openedSync = await client.openSyncObjects(keyring, {
    objects: [
      {
        vaultId: "smoke",
        folderId: "general",
        objectId: "obj_000000000001",
        revision: 1,
        ciphertext: write.ciphertext,
      },
    ],
  });
  assert.equal(openedSync.objects[0].status, "ready");
  assert.equal(openedSync.objects[0].title, "Hello");

  const cliPageWrite = await client.buildPageWriteRequest(keyring, {
    authorNpub,
    baseRevision: null,
    createdAtUnix: 1780000000,
    folderId: "general",
    keyVersion: 1,
    nonceBytes: new Uint8Array(12).fill(2),
    objectId: "obj_cli_page0001",
    plaintext: JSON.stringify({
      version: "finite-folder-object-page-v1",
      path: "compiled/hermes-agent-overview.md",
      markdown: "# Hermes Agent Overview\n\nAgent-authored docs.",
    }),
    signEvent: async (template) => ({
      ...template,
      id: "cli-revision-event-id",
      pubkey: "00".repeat(32),
      sig: "revision-signature",
    }),
    vaultId: "smoke",
  });
  const openedCliPage = await client.openFolderObject(keyring, {
    vaultId: "smoke",
    folderId: "general",
    objectId: "obj_cli_page0001",
    revision: 1,
    ciphertext: cliPageWrite.ciphertext,
  });
  assert.equal(openedCliPage.status, "ready");
  assert.equal(openedCliPage.path, "compiled/hermes-agent-overview.md");
  assert.equal(openedCliPage.text, "# Hermes Agent Overview\n\nAgent-authored docs.");

  const assetPlaintext = await client.encodeFolderObjectAssetPlaintext(
    "raw/assets/source.pdf",
    new TextEncoder().encode("%PDF asset\n"),
    "application/pdf"
  );
  const assetWrite = await client.buildPageWriteRequest(keyring, {
    authorNpub,
    baseRevision: null,
    createdAtUnix: 1780000000,
    folderId: "general",
    keyVersion: 1,
    nonceBytes: new Uint8Array(12).fill(3),
    objectId: "obj_cli_asset001",
    plaintext: assetPlaintext,
    signEvent: async (template) => ({
      ...template,
      id: "asset-revision-event-id",
      pubkey: "00".repeat(32),
      sig: "revision-signature",
    }),
    vaultId: "smoke",
  });
  const openedAsset = await client.openFolderObject(keyring, {
    vaultId: "smoke",
    folderId: "general",
    objectId: "obj_cli_asset001",
    revision: 1,
    ciphertext: assetWrite.ciphertext,
  });
  assert.equal(openedAsset.status, "ready");
  assert.equal(openedAsset.type, "asset");
  assert.equal(openedAsset.path, "raw/assets/source.pdf");
  assert.equal(openedAsset.contentType, "application/pdf");
  assert.equal(new TextDecoder().decode(openedAsset.bytes), "%PDF asset\n");
  assert.equal(openedAsset.text, undefined);
  assert.equal(client.buildGraphProjection([openedAsset]).nodes.length, 0);
  assert.equal(client.searchPageRows("source", [openedAsset]).length, 0);
  assert.equal(client.readerPageRows("general", [openedAsset]).length, 0);
  await assert.rejects(
    () =>
      client.encodeFolderObjectAssetPlaintext(
        "attachments/source.pdf",
        new TextEncoder().encode("%PDF asset\n"),
        "application/pdf"
      ),
    /raw\/assets/
  );

  const openedCliSync = await client.openSyncObjects(keyring, {
    objects: [
      {
        vaultId: "smoke",
        folderId: "general",
        objectId: "obj_cli_page0001",
        revision: 1,
        ciphertext: cliPageWrite.ciphertext,
      },
    ],
  });
  const openedCliRow = client.readerPageRows("general", openedCliSync.objects)[0];
  assert.equal(openedCliRow.label, "Hermes Agent Overview");
  assert.equal(openedCliRow.detail, "compiled/hermes-agent-overview.md");
  assert.equal(client.pagePathLabel(openedCliRow), "general/compiled/hermes-agent-overview.md");

  const restrictedOldKey = Buffer.alloc(32, 8).toString("base64");
  await client.openFolderKeyGrantPlaintext(keyring, {
    version: "finite-folder-key-grant-v1",
    vaultId: "smoke",
    folderId: "restricted",
    keyVersion: 1,
    issuerNpub: authorNpub,
    recipientNpub: authorNpub,
    folderKey: restrictedOldKey,
    issuedAt: "2026-06-24T00:00:00.000Z",
  });
  let signedIndex = 0;
  const signDeterministically = async (template) => ({
    ...template,
    id: `signed-${++signedIndex}`,
    pubkey: "00".repeat(32),
    sig: "revision-signature",
  });
  const restrictedWrite = await client.buildPageWriteRequest(keyring, {
    authorNpub,
    baseRevision: null,
    createdAtUnix: 1780000001,
    folderId: "restricted",
    keyVersion: 1,
    nonceBytes: new Uint8Array(12).fill(1),
    objectId: "obj_restricted0001",
    plaintext: "# Restricted\n\nRotate this page.",
    signEvent: signDeterministically,
    vaultId: "smoke",
  });
  const targetNpub = client.npubFromHex("11".repeat(32));
  const remainingNpub = client.npubFromHex("22".repeat(32));
  const removal = await client.buildFolderAccessRemovalRequest(keyring, {
    vaultId: "smoke",
    metadata: { admins: [authorNpub] },
    row: {
      id: "restricted",
      path: "Restricted",
      access: "restricted",
      accessUserIds: [targetNpub, remainingNpub],
      currentKeyVersion: 1,
    },
    targetNpub,
    objects: [
      {
        vaultId: "smoke",
        folderId: "restricted",
        objectId: "obj_restricted0001",
        revision: 1,
        status: "ready",
        text: "# Restricted\n\nRotate this page.",
        ciphertext: restrictedWrite.ciphertext,
      },
    ],
    newRawKey: new Uint8Array(32).fill(9),
    createdAtUnix: 1780000100,
    actorNpub: authorNpub,
    signEvent: signDeterministically,
  });
  assert.equal(removal.newKeyVersion, 2);
  assert.equal(
    JSON.stringify(removal.grants.map((grant) => grant.recipientNpub).sort()),
    JSON.stringify([authorNpub, remainingNpub].sort())
  );
  assert.equal(removal.grants.some((grant) => grant.recipientNpub === targetNpub), false);
  assert.equal(removal.reencryptedRecords.length, 1);
  assert.equal(removal.reencryptedRecords[0].objectId, "obj_restricted0001");
  assert.equal(removal.reencryptedRecords[0].baseRevision, 1);
  assert.equal(removal.reencryptedRecords[0].keyVersion, 2);
  assert.equal(
    JSON.stringify(removal.reencryptedRecords[0].revisionEvent.tags),
    JSON.stringify([
      ["d", "finite-folder-object-revision:smoke:restricted:obj_restricted0001:2"],
      ["vault", "smoke"],
      ["folder", "restricted"],
      ["object", "obj_restricted0001"],
      ["operation", "update"],
      ["keyVersion", "2"],
    ])
  );
  assert.match(removal.accessChangeEvent.content, /remove-folder-access/);
  const rotatedPage = await client.openFolderObject(keyring, {
    vaultId: "smoke",
    folderId: "restricted",
    objectId: "obj_restricted0001",
    revision: 2,
    ciphertext: removal.reencryptedRecords[0].ciphertext,
  });
  assert.equal(rotatedPage.status, "ready");
  assert.equal(rotatedPage.text, "# Restricted\n\nRotate this page.");

  const readerFolders = client.readerFolderRows(
    {
      folders: [
        {
          id: "general",
          path: "General",
          access: "all_members",
          accessUserIds: [],
          currentKeyVersion: 1,
          setupIncomplete: false,
          sharedFolderSource: false,
        },
        {
          id: "restricted",
          path: "Restricted",
          access: "restricted",
          accessUserIds: [],
          currentKeyVersion: 1,
          setupIncomplete: false,
          sharedFolderSource: false,
        },
      ],
    },
    openedSync.objects
  );
  assert.equal(readerFolders[0].readableCount, 1);
  assert.equal(readerFolders[0].pageCount, 1);
  assert.equal(readerFolders[0].access, "all_members");
  assert.equal(readerFolders[0].accessLabel, "all members");
  assert.equal(readerFolders[1].status, "locked");
  assert.equal(readerFolders[1].accessLabel, "restricted");
  const compatibilityRows = client.metadataFolderRows({
    folders: [
      {
        id: "architecture",
        path: "Architecture",
        access_mode: "all_members",
        access_user_ids: [],
        current_key_version: 2,
        setup_incomplete: false,
        shared_folder_source: false,
      },
      {
        id: "vault-ops",
        path: "vault-ops",
        accessMode: "AdminOnly",
        accessUserIds: [],
        currentKeyVersion: 1,
        setupIncomplete: false,
        sharedFolderSource: false,
      },
    ],
  });
  assert.equal(compatibilityRows[0].access, "all_members");
  assert.equal(compatibilityRows[0].accessLabel, "all members");
  assert.equal(compatibilityRows[0].currentKeyVersion, 2);
  assert.equal(compatibilityRows[1].access, "admin_only");
  assert.equal(compatibilityRows[1].accessLabel, "admin only");
  assert.equal(
    client.readerFolderDetail(readerFolders[0]),
    "1 page"
  );
  assert.equal(
    client.readerFolderDetail({
      accessLabel: "all members",
      pageCount: 0,
      readableCount: 0,
    }),
    "Empty"
  );
  assert.equal(
    client.readerFolderDetail({
      accessLabel: "restricted",
      pageCount: 2,
      readableCount: 0,
    }),
    "Locked"
  );
  assert.equal(client.workspaceTabTitle(null, null), "Open a Vault");
  assert.equal(client.workspaceTabTitle({ name: "Smoke" }, null), "Smoke");
  assert.equal(
    client.workspaceTabTitle({ name: "Smoke" }, { title: "Folder Object Crypto" }),
    "Folder Object Crypto"
  );
  assert.equal(client.workspaceChromeState("page").shellView, "page");
  assert.equal(client.workspaceChromeState("page").pageHidden, false);
  assert.equal(client.workspaceChromeState("page").graphHidden, true);
  assert.equal(client.workspaceChromeState("graph").shellView, "graph");
  assert.equal(client.workspaceChromeState("graph").pageHidden, true);
  assert.equal(client.workspaceChromeState("graph").graphHidden, false);
  assert.match(client.workspaceChromeState("graph").ribbonGraphClass, /active/);
  assert.equal(client.graphEmptyStateCopy().title, "No graph yet");
  assert.equal(
    client.graphEmptyStateCopy({ readablePageCount: 3 }).copy,
    "Readable pages are open, but none link to another page yet."
  );
  assert.equal(
    client.graphEmptyStateCopy({ filterText: "folder key", readablePageCount: 3 }).title,
    "No matching Pages"
  );
  assert.equal(
    client.graphEmptyStateCopy({ filterText: "folder key", readablePageCount: 0 }).title,
    "No graph yet"
  );
  assert.equal(client.normalizeSidebarMode("search"), "search");
  assert.equal(client.normalizeSidebarMode("access"), "access");
  assert.equal(client.normalizeSidebarMode("bogus"), "files");
  assert.equal(client.sidebarModeLabel("search"), "Search");
  assert.equal(client.sidebarModeLabel("bogus"), "Files");
  assert.equal(client.globalVaultControlState("files").hidden, true);
  assert.equal(client.globalVaultControlState("search").hidden, true);
  assert.equal(client.globalVaultControlState("access").hidden, true);
  assert.equal(client.globalVaultControlState("bogus").hidden, true);
  assert.equal(
    JSON.stringify(client.commandPaletteCommands().map((row) => row.id)),
    JSON.stringify(["files", "search", "access", "graph", "new-page", "refresh"])
  );
  const searchRows = client.searchPageRows("folder key", [
    {
      folderId: "crypto",
      objectId: "page-a",
      path: "folder-keys.md",
      status: "ready",
      text: "# Folder Keys\n\nReadable key material stays client-side.",
      title: "Folder Keys",
    },
    {
      folderId: "sync",
      objectId: "page-b",
      path: "sync.md",
      status: "ready",
      text: "# Sync\n\nCursor notes.",
      title: "Sync",
    },
  ]);
  assert.equal(searchRows.length, 1);
  assert.equal(searchRows[0].detail, "crypto/folder-keys.md");
  assert.equal(searchRows[0].matchSnippet, "# Folder Keys Readable key material stays client-side.");
  assert.equal(
    JSON.stringify(client.searchHighlightSegments("Testing test TEST", "test")),
    JSON.stringify([
      { match: true, text: "Test" },
      { match: false, text: "ing " },
      { match: true, text: "test" },
      { match: false, text: " " },
      { match: true, text: "TEST" },
    ])
  );
  assert.equal(
    JSON.stringify(client.searchHighlightSegments("a+b and A+B", "a+b")),
    JSON.stringify([
      { match: true, text: "a+b" },
      { match: false, text: " and " },
      { match: true, text: "A+B" },
    ])
  );
  assert.equal(
    client.searchResultSnippet(
      { path: "notes.md", text: "A focused keyword appears in this sentence.", title: "Notes" },
      "keyword"
    ),
    "A focused keyword appears in this sentence."
  );
  assert.equal(
    client.readerSearchHighlightForPage("crypto/page-a", {
      pageKey: "crypto/page-a",
      query: " folder key ",
    }),
    "folder key"
  );
  assert.equal(
    client.readerSearchHighlightForPage("crypto/page-b", {
      pageKey: "crypto/page-a",
      query: "folder key",
    }),
    ""
  );
  assert.match(source, /selectReaderPage\(row\.key, \{ searchQuery: query \}\)/);
  assert.match(source, /highlightReaderSearchMatches\(content, searchQuery\)/);
  assert.match(source, /scrollIntoView\?\.\(\{ behavior, block: "center", inline: "nearest" \}\)/);
  assert.match(cssSource, /\.reader-search-match\s*\{[\s\S]*?scroll-margin-block: 28px;/);
  const paletteRows = client.commandPaletteRows("folder", [
    {
      folderId: "crypto",
      key: "crypto/page-a",
      objectId: "page-a",
      path: "folder-keys.md",
      status: "ready",
      text: "# Folder Keys\n\nReadable key material stays client-side.",
      title: "Folder Keys",
    },
  ]);
  assert.equal(paletteRows.some((row) => row.id === "new-page"), true);
  assert.equal(paletteRows.some((row) => row.kind === "page" && row.label === "Folder Keys"), true);
  assert.equal(client.commandPaletteRows("", []).length, 6);
  assert.equal(
    JSON.stringify(client.editorSlashCommandRows("").slice(0, 4).map((row) => row.id)),
    JSON.stringify(["paragraph", "heading1", "heading2", "bullet"])
  );
  assert.equal(
    JSON.stringify(client.editorSlashCommandRows("h1").map((row) => row.id)),
    JSON.stringify(["heading1"])
  );
  assert.equal(
    JSON.stringify(client.editorSlashCommandRows("code").map((row) => row.id)),
    JSON.stringify(["codeblock", "code"])
  );
  const folderMenu = client.contextMenuItemsForTarget({ type: "folder", folderId: "crypto" });
  assert.equal(folderMenu.some((item) => item.action === "new-page"), true);
  assert.equal(folderMenu.some((item) => item.action === "share-folder"), true);
  assert.equal(folderMenu.find((item) => item.action === "delete-folder").disabled, true);
  const pageMenu = client.contextMenuItemsForTarget({
    type: "page",
    folderId: "crypto",
    objectId: "page-a",
  });
  assert.equal(pageMenu.some((item) => item.action === "open-graph"), true);
  assert.equal(pageMenu.some((item) => item.action === "edit-page"), false);
  assert.equal(pageMenu.find((item) => item.action === "delete-page").disabled, false);
  const readerPages = client.readerPageRows("general", openedSync.objects);
  assert.equal(readerPages[0].label, "Hello");
  assert.equal(readerPages[0].detail, "obj_000000000001.md");
  assert.equal(client.pagePathLabel(readerPages[0]), "general/obj_000000000001.md");
  assert.equal(client.readerPageDetail(readerPages[0]), "obj_000000000001.md");
  const emptyReadablePage = {
    folderId: "general",
    objectId: "obj_empty_page01",
    revision: 1,
    status: "ready",
    text: "",
  };
  const readerFoldersWithEmptyPage = client.readerFolderRows(
    {
      folders: [
        {
          id: "general",
          path: "General",
          access: "all_members",
          accessUserIds: [],
          currentKeyVersion: 1,
          setupIncomplete: false,
          sharedFolderSource: false,
        },
      ],
    },
    [...openedSync.objects, emptyReadablePage]
  );
  assert.equal(readerFoldersWithEmptyPage[0].pageCount, 2);
  assert.equal(readerFoldersWithEmptyPage[0].readableCount, 2);
  const emptyReaderPage = client.readerPageRows("general", [emptyReadablePage])[0];
  assert.equal(emptyReaderPage.label, "obj_empty_page01");
  assert.match(client.nextDraftObjectId(), /^obj_[A-Za-z0-9_-]{12,124}$/);
  assert.ok(client.nextDraftObjectId().length >= 16);

  const lockedPage = await client.openFolderObject(client.createSessionKeyring(), {
    vaultId: "smoke",
    folderId: "general",
    objectId: "obj_000000000001",
    revision: 1,
    ciphertext: write.ciphertext,
  });
  assert.equal(lockedPage.status, "locked");

  const lockedSync = await client.openSyncObjects(client.createSessionKeyring(), {
    objects: [
      {
        vaultId: "smoke",
        folderId: "general",
        objectId: "obj_000000000001",
        revision: 1,
        ciphertext: write.ciphertext,
      },
    ],
  });
  assert.equal(lockedSync.objects[0].status, "locked");

  const projection = client.createClientProjection();
  projection.localDrafts.set("general/obj_000000000001", {
    baseRevision: 1,
    text: "Unresolved local edit",
  });
  const merged = client.mergeSyncProjection(projection, {
    records: [{ recordEventId: "event-a" }, { recordEventId: "event-a" }],
    objects: [
      {
        folderId: "general",
        objectId: "obj_000000000001",
        revision: 2,
        ciphertext: write.ciphertext,
      },
    ],
  });
  assert.equal(merged.seenEventIds.size, 1);
  assert.equal(merged.conflicts.length, 1);
  assert.equal(merged.conflicts[0].status, "conflict");
  assert.equal(merged.localDrafts.has("general/obj_000000000001"), true);
  assert.equal(merged.pages.has("general/obj_000000000001"), false);

  assert.deepEqual(
    Array.from(client.extractPageLinks("[[Roadmap]] [Spec](Specs/OKF.md) [Web](https://example.com)")),
    ["roadmap", "specs/okf"]
  );
  assert.equal(
    JSON.stringify(client.inlineLinkSegments("Read [[Roadmap]] and [Spec](Specs/OKF.md).")),
    JSON.stringify([
      { kind: "text", text: "Read " },
      { kind: "internal", target: "roadmap", text: "Roadmap" },
      { kind: "text", text: " and " },
      { kind: "internal", target: "specs/okf", text: "Spec" },
      { kind: "text", text: "." },
    ])
  );
  assert.equal(
    JSON.stringify(client.inlineLinkSegments("Read [[Roadmap#Now|Q3 roadmap]].")),
    JSON.stringify([
      { kind: "text", text: "Read " },
      { kind: "internal", target: "roadmap", text: "Q3 roadmap" },
      { kind: "text", text: "." },
    ])
  );
  assert.equal(
    JSON.stringify(client.markdownPreviewBlocks(
      [
        "# Title",
        "",
        "- One",
        "- [x] Done",
        "",
        "1. First",
        "2. Second",
        "",
        "> Note",
        "",
        "| Name | Status |",
        "| --- | :---: |",
        "| Brain | **ready** |",
        "",
        "```js",
        "const ok = true;",
        "```",
      ].join("\n")
    )),
    JSON.stringify([
      { level: 1, text: "Title", type: "heading" },
      {
        items: [
          { checked: null, text: "One" },
          { checked: true, text: "Done" },
        ],
        ordered: false,
        start: null,
        type: "list",
      },
      {
        items: [
          { checked: null, text: "First" },
          { checked: null, text: "Second" },
        ],
        ordered: true,
        start: 1,
        type: "list",
      },
      { text: "Note", type: "quote" },
      {
        alignments: ["", "center"],
        headers: ["Name", "Status"],
        rows: [["Brain", "**ready**"]],
        type: "table",
      },
      { language: "js", text: "const ok = true;", type: "code" },
    ])
  );
  assert.equal(
    JSON.stringify(client.markdownPreviewBlocks(
      [
        "```bash",
        "",
        "  hermes doctor",
        "",
        "",
        "```",
      ].join("\n")
    )),
    JSON.stringify([{ language: "bash", text: "hermes doctor", type: "code" }])
  );
  assert.equal(
    JSON.stringify(client.markdownPreviewBlocks(
      [
        "```python",
        "    if ready:",
        "        ship()",
        "```",
      ].join("\n")
    )),
    JSON.stringify([{ language: "python", text: "if ready:\n    ship()", type: "code" }])
  );
  assert.equal(
    client.markdownFromEditorElement(
      elementNode("div", [
        elementNode("h1", [textNode("Draft Title")]),
        elementNode("p", [
          textNode("Write "),
          elementNode("strong", [textNode("bold")]),
          textNode(", "),
          elementNode("em", [textNode("soft")]),
          textNode(", "),
          elementNode("code", [textNode("local")]),
          textNode(", and "),
          elementNode("span", [textNode("Roadmap")], {
            className: "internal-link",
            dataset: { target: "roadmap" },
          }),
          textNode(", not "),
          elementNode("del", [textNode("stale")]),
          textNode("."),
        ]),
        elementNode("ul", [
          elementNode("li", [textNode("First")]),
          elementNode("li", [textNode("Second")]),
        ]),
        elementNode("ol", [
          elementNode("li", [textNode("Plan")]),
          elementNode("li", [textNode("Ship")]),
        ]),
        elementNode("ul", [
          elementNode("li", [
            elementNode("input", [], { checked: true, type: "checkbox" }),
            textNode("Verified"),
          ]),
        ]),
        elementNode("blockquote", [textNode("Keep it simple")]),
        elementNode("pre", [], {
          dataset: { language: "js" },
          textContent: "\n  const ok = true;\n\n",
        }),
        elementNode("table", [
          elementNode("thead", [
            elementNode("tr", [
              elementNode("th", [textNode("Name")]),
              elementNode("th", [textNode("Status")]),
            ]),
          ]),
          elementNode("tbody", [
            elementNode("tr", [
              elementNode("td", [textNode("Brain")]),
              elementNode("td", [elementNode("strong", [textNode("ready")])]),
            ]),
          ]),
        ]),
        elementNode("hr"),
      ])
    ),
    [
      "# Draft Title",
      "Write **bold**, *soft*, `local`, and [[roadmap|Roadmap]], not ~~stale~~.",
      "- First\n- Second",
      "1. Plan\n2. Ship",
      "- [x] Verified",
      "> Keep it simple",
      "```js\nconst ok = true;\n```",
      "| Name | Status |\n| --- | --- |\n| Brain | **ready** |",
      "---",
    ].join("\n\n")
  );
  assert.equal(JSON.stringify(client.pageStatsForText("# Title\n\nSee [[Roadmap]] and words.")), JSON.stringify({
    links: 1,
    words: 6,
  }));
  const linkContext = client.pageLinkContext(
    {
      folderId: "general",
      key: "general/alpha",
      objectId: "alpha",
      status: "ready",
      text: "# Alpha\n\nSee [[Beta]] and [[Missing]].",
      title: "Alpha",
    },
    [
      {
        folderId: "general",
        key: "general/alpha",
        objectId: "alpha",
        status: "ready",
        text: "# Alpha\n\nSee [[Beta]] and [[Missing]].",
        title: "Alpha",
      },
      {
        folderId: "general",
        key: "general/beta",
        objectId: "beta",
        status: "ready",
        text: "# Beta\n\nBack to [[Alpha]].",
        title: "Beta",
      },
      {
        folderId: "restricted",
        key: "restricted/locked",
        objectId: "locked",
        status: "locked",
        text: "# Locked\n\n[[Alpha]]",
        title: "Locked",
      },
    ]
  );
  assert.equal(
    JSON.stringify(linkContext.outgoing.map((row) => [row.label, row.status])),
    JSON.stringify([
      ["Beta", "resolved"],
      ["missing", "missing"],
    ])
  );
  assert.equal(
    JSON.stringify(linkContext.backlinks.map((row) => [row.label, row.key])),
    JSON.stringify([["Beta", "general/beta"]])
  );
  assert.equal(client.pageKeyForReference("Beta", [
    {
      folderId: "general",
      key: "general/beta",
      objectId: "beta",
      path: "wiki/beta.md",
      status: "ready",
      text: "# Beta\n\nBack to [[Alpha]].",
      title: "Beta",
    },
  ]), "general/beta");
  assert.equal(client.pageKeyForReference("wiki/beta.md", [
    {
      folderId: "general",
      key: "general/beta",
      objectId: "beta",
      path: "wiki/beta.md",
      status: "ready",
      text: "# Beta\n\nBack to [[Alpha]].",
      title: "Beta",
    },
  ]), "general/beta");
  assert.equal(client.pageKeyForReference("Locked", [
    {
      folderId: "restricted",
      key: "restricted/locked",
      objectId: "locked",
      path: "locked.md",
      status: "locked",
      text: "# Locked",
      title: "Locked",
    },
  ]), null);
  const pathLinkContext = client.pageLinkContext(
    {
      folderId: "docs",
      key: "docs/intro",
      objectId: "intro",
      path: "docs/intro.md",
      status: "ready",
      text: "# Intro\n\nSee [Deep Dive](deep-dive.md).",
      title: "Intro",
    },
    [
      {
        folderId: "docs",
        key: "docs/intro",
        objectId: "intro",
        path: "docs/intro.md",
        status: "ready",
        text: "# Intro\n\nSee [Deep Dive](deep-dive.md).",
        title: "Intro",
      },
      {
        folderId: "docs",
        key: "docs/deep-dive",
        objectId: "deep-dive",
        path: "docs/deep-dive.md",
        status: "ready",
        text: "# Deep Dive\n\nBack to [Intro](intro.md).",
        title: "Deep Dive",
      },
    ]
  );
  assert.equal(
    JSON.stringify(pathLinkContext.outgoing.map((row) => [row.label, row.status])),
    JSON.stringify([["Deep Dive", "resolved"]])
  );
  assert.equal(
    JSON.stringify(pathLinkContext.backlinks.map((row) => [row.label, row.key])),
    JSON.stringify([["Deep Dive", "docs/deep-dive"]])
  );

  const okfInput = {
    manifest: {
      version: "finite-okf-vault-export-v1",
      objects: [
        {
          folderId: "source-concepts",
          objectId: "obj_source_alpha1",
          path: "content/Concepts/alpha.md",
          contentType: "text/markdown",
          contentHash: "hash-alpha",
        },
        {
          folderId: "source-concepts",
          objectId: "obj_source_beta01",
          path: "content/Concepts/beta.md",
          contentType: "text/markdown",
          contentHash: "hash-beta",
        },
        {
          folderId: "source-concepts",
          objectId: "obj_source_asset1",
          path: "content/Concepts/raw/assets/source.pdf",
          contentType: "application/pdf",
          contentHash: "hash-pdf",
        },
      ],
      omissions: [{ folderId: "secret", displayPath: "Secret", reason: "inaccessible" }],
    },
    files: {
      "content/Concepts/alpha.md": "# Alpha\n\nSee [Beta](beta.md), [[Loose Wiki]], and raw/assets/source.pdf.",
      "content/Concepts/beta.md": "# Beta\n\nImported target.",
      "content/Concepts/raw/assets/source.pdf": {
        bytesBase64: Buffer.from("%PDF okf\n").toString("base64"),
      },
    },
  };
  const parsedOkf = client.parseOkfBundle(JSON.stringify(okfInput), {
    destinationFolderId: "general",
  });
  assert.equal(parsedOkf.pages.length, 2);
  assert.equal(parsedOkf.assets.length, 1);
  assert.equal(parsedOkf.pages[0].folderId, "general");
  assert.equal(parsedOkf.pages[0].targetPath, "alpha.md");
  assert.deepEqual(Array.from(parsedOkf.pages[0].links), ["loose wiki", "beta"]);
  assert.equal(parsedOkf.assets[0].targetPath, "raw/assets/source.pdf");
  assert.equal(parsedOkf.assets[0].contentType, "application/pdf");
  assert.equal(parsedOkf.omissions[0].reason, "inaccessible");
  const explicitAssetOkf = client.parseOkfBundle(
    {
      assets: [
        {
          path: "attachments/photo.png",
          bytesBase64: Buffer.from("png").toString("base64"),
          contentType: "image/png",
        },
      ],
    },
    { destinationFolderId: "general" }
  );
  assert.equal(explicitAssetOkf.assets[0].targetPath, "raw/assets/photo.png");
  const assetLinkOkf = client.parseOkfBundle(
    {
      pages: [
        {
          path: "content/Notes/source.md",
          content: "# Source\n\n[Photo](../attachments/photo.png)",
        },
      ],
      assets: [
        {
          path: "content/attachments/photo.png",
          bytesBase64: Buffer.from("one").toString("base64"),
          contentType: "image/png",
        },
        {
          path: "content/more/photo.png",
          bytesBase64: Buffer.from("two").toString("base64"),
          contentType: "image/png",
        },
      ],
    },
    { destinationFolderId: "general" }
  );
  const assetLinkPlan = client.planOkfImport(assetLinkOkf, [], { conflictMode: "skip" });
  const assetLinkPage = assetLinkPlan.entries.find((entry) => entry.kind === "page");
  const assetLinkTargets = assetLinkPlan.entries
    .filter((entry) => entry.kind === "asset")
    .map((entry) => entry.targetPath)
    .sort();
  assert.match(assetLinkPage.markdown, /\[Photo\]\(raw\/assets\/photo\.png\)/);
  assert.equal(
    JSON.stringify(assetLinkTargets),
    JSON.stringify(["raw/assets/photo imported.png", "raw/assets/photo.png"])
  );

  const skipPlan = client.planOkfImport(
    parsedOkf,
    [
      {
        folderId: "general",
        objectId: "obj_existing_alpha_01",
        path: "alpha.md",
        revision: 3,
      },
      {
        folderId: "general",
        objectId: "obj_existing_beta_01",
        path: "beta.md",
        revision: 7,
      },
    ],
    { conflictMode: "skip" }
  );
  assert.equal(skipPlan.summary.skip, 2);
  assert.equal(skipPlan.summary.create, 1);
  assert.equal(skipPlan.entries.filter((entry) => entry.kind === "page").every((entry) => entry.action === "skip"), true);

  const copyPlan = client.planOkfImport(
    parsedOkf,
    [
      {
        folderId: "general",
        objectId: "obj_existing_beta_01",
        path: "beta.md",
        revision: 7,
      },
    ],
    { conflictMode: "copy" }
  );
  const copyAlpha = copyPlan.entries.find((entry) => entry.targetPath === "alpha.md");
  const copyBeta = copyPlan.entries.find((entry) => entry.action === "copy");
  const copyAsset = copyPlan.entries.find((entry) => entry.kind === "asset");
  assert.equal(copyPlan.summary.create, 2);
  assert.equal(copyPlan.summary.copy, 1);
  assert.equal(copyBeta.targetPath, "beta imported.md");
  assert.equal(copyAsset.targetPath, "raw/assets/source.pdf");
  assert.match(copyAlpha.markdown, /\[Beta\]\(beta imported\.md\)/);

  const saturatedObjectIdBase = objectIdCandidateBaseForTest("beta imported.md");
  const saturatedObjectPages = Array.from({ length: 1000 }, (_, index) => ({
    folderId: "general",
    objectId: index === 0 ? saturatedObjectIdBase : `${saturatedObjectIdBase}_${index + 1}`,
    path: `collision-${index}.md`,
    revision: 1,
  }));
  assert.throws(
    () =>
      client.planOkfImport(
        parsedOkf,
        [
          {
            folderId: "general",
            objectId: "obj_existing_beta_01",
            path: "beta.md",
            revision: 7,
          },
          ...saturatedObjectPages,
        ],
        { conflictMode: "copy" }
      ),
    /could not allocate import object id for beta imported\.md/
  );

  const overwritePlan = client.planOkfImport(
    parsedOkf,
    [
      {
        folderId: "general",
        objectId: "obj_existing_alpha_01",
        path: "alpha.md",
        revision: 3,
      },
    ],
    { conflictMode: "overwrite" }
  );
  assert.equal(overwritePlan.entries[0].action, "overwrite");
  assert.equal(overwritePlan.entries[0].baseRevision, 3);
  assert.equal(overwritePlan.entries[0].objectId, "obj_existing_alpha_01");

  await assert.rejects(
    () =>
      client.prepareOkfImportWrites(client.createSessionKeyring(), copyPlan, {
        authorNpub,
        signEvent: async (template) => template,
        vaultId: "smoke",
      }),
    /Folder Key is not open for general/
  );

  const preparedImport = await client.prepareOkfImportWrites(keyring, copyPlan, {
    authorNpub,
    createdAtUnix: 1780000001,
    nonceFactory: (index) => new Uint8Array(12).fill(index + 1),
    signEvent: async (template) => ({
      ...template,
      id: `import-event-${template.created_at}`,
      pubkey: "00".repeat(32),
      sig: "import-signature",
    }),
    vaultId: "smoke",
  });
  assert.equal(preparedImport.writes.length, 3);
  assert.equal(preparedImport.skipped.length, 0);
  assert.match(preparedImport.writes[0].path, /\/_admin\/vaults\/smoke\/folders\/general\/objects\/obj_/);
  assert.equal(preparedImport.writes[0].body.revisionEvent.kind, 30078);

  const implicitFolderOkf = client.parseOkfBundle({
    pages: [{ path: "content/Notes/start.md", content: "# Start\n" }],
  });
  assert.equal(implicitFolderOkf.pages[0].folderId, "getting-started");

  const openedImportedAlpha = await client.openFolderObject(keyring, {
    vaultId: "smoke",
    folderId: preparedImport.writes[0].folderId,
    objectId: preparedImport.writes[0].objectId,
    revision: 1,
    ciphertext: preparedImport.writes[0].body.ciphertext,
  });
  assert.equal(openedImportedAlpha.status, "ready");
  assert.equal(openedImportedAlpha.path, preparedImport.writes[0].targetPath);
  assert.match(openedImportedAlpha.text, /\[Beta\]\(beta imported\.md\)/);
  const importedAssetWrite = preparedImport.writes.find((write) => write.targetPath === "raw/assets/source.pdf");
  const openedImportedAsset = await client.openFolderObject(keyring, {
    vaultId: "smoke",
    folderId: importedAssetWrite.folderId,
    objectId: importedAssetWrite.objectId,
    revision: 1,
    ciphertext: importedAssetWrite.body.ciphertext,
  });
  assert.equal(openedImportedAsset.type, "asset");
  assert.equal(openedImportedAsset.contentType, "application/pdf");
  assert.equal(openedImportedAsset.contentHash, crypto.createHash("sha256").update("%PDF okf\n").digest("hex"));
  assert.equal(new TextDecoder().decode(openedImportedAsset.bytes), "%PDF okf\n");

  const graph = client.buildGraphProjection([
    {
      folderId: "general",
      objectId: "page-a",
      status: "ready",
      text: "# Alpha\n\nLinks to [[Beta]] and [[Hidden]].",
    },
    {
      folderId: "general",
      objectId: "page-b",
      status: "ready",
      text: "# Beta\n\nBack to [Alpha](Alpha.md).",
    },
    {
      folderId: "restricted",
      objectId: "page-hidden",
      status: "locked",
      text: "# Hidden\n\nThis must not appear.",
    },
  ]);
  assert.deepEqual(
    Array.from(graph.nodes.map((node) => node.title).sort()),
    ["Alpha", "Beta"]
  );
  assert.equal(graph.edges.length, 2);
  assert.equal(graph.edges.some((edge) => edge.id.includes("page-hidden")), false);
  const graphMetrics = client.graphStats(graph, 3);
  assert.equal(graphMetrics.edgeCount, 2);
  assert.equal(graphMetrics.filteredOutCount, 1);
  assert.equal(graphMetrics.nodeCount, 2);
  assert.deepEqual(
    Array.from(client.graphNeighborIds(graph, "general/page-a")).sort(),
    ["general/page-a", "general/page-b"]
  );
  assert.deepEqual(Array.from(client.graphNeighborIds(graph, null)), []);

  const filteredGraph = client.buildGraphProjection(
    [
      {
        folderId: "general",
        objectId: "page-a",
        status: "ready",
        text: "# Alpha\n\n[[Beta]]",
      },
      {
        folderId: "general",
        objectId: "page-b",
        status: "ready",
        text: "# Beta",
      },
    ],
    "beta"
  );
  assert.deepEqual(
    Array.from(filteredGraph.nodes.map((node) => node.title).sort()),
    ["Alpha", "Beta"]
  );
  const layout = client.graphLayout(graph, { height: 260, margin: 40, width: 320 });
  assert.equal(layout.size, 2);
  for (const position of layout.values()) {
    assert.equal(position.x >= 40 && position.x <= 280, true);
    assert.equal(position.y >= 40 && position.y <= 220, true);
  }
  assert.equal(
    JSON.stringify(Array.from(client.graphLayout(graph, { height: 260, margin: 40, width: 320 }).entries())),
    JSON.stringify(Array.from(layout.entries()))
  );
  const hubGraph = client.buildGraphProjection([
    {
      folderId: "general",
      objectId: "hub",
      status: "ready",
      text: "# Hub\n\n[[One]] [[Two]] [[Three]] [[Four]]",
    },
    { folderId: "general", objectId: "one", status: "ready", text: "# One" },
    { folderId: "general", objectId: "two", status: "ready", text: "# Two" },
    { folderId: "general", objectId: "three", status: "ready", text: "# Three" },
    { folderId: "general", objectId: "four", status: "ready", text: "# Four" },
  ]);
  const hubLayout = client.graphLayout(hubGraph, { height: 300, margin: 60, width: 400 });
  assert.equal(JSON.stringify(hubLayout.get("general/hub")), JSON.stringify({ x: 200, y: 150 }));
  assert.equal(
    JSON.stringify(client.graphViewBoxForZoom(1)),
    JSON.stringify({ height: 560, width: 900, x: 0, y: 0, zoom: 1 })
  );
  assert.equal(
    JSON.stringify(client.graphViewBoxForZoom(99)),
    JSON.stringify({ height: 224, width: 360, x: 270, y: 168, zoom: 2.5 })
  );
  assert.equal(client.graphViewBoxForZoom(0).zoom, 0.5);

  const invitationRows = client.vaultInvitationRows([
    {
      createdAt: "2026-07-01T00:00:00.000Z",
      expiresAt: "2026-07-30T00:00:00.000Z",
      id: "invitation-old",
      inviteCode: "invite-old",
      status: "accepted",
      userId: "npub1older",
    },
    {
      createdAt: "2026-07-02T00:00:00.000Z",
      expiresAt: "2026-07-30T00:00:00.000Z",
      id: "invitation-new",
      inviteCode: "invite-new",
      status: "pending",
      userId: "npub1newer",
    },
    {
      createdAt: "2026-07-03T00:00:00.000Z",
      expiresAt: "2026-07-30T00:00:00.000Z",
      id: "invitation-revoked",
      inviteCode: "invite-revoked",
      status: "revoked",
      userId: "npub1revoked",
    },
  ]);
  assert.deepEqual(
    Array.from(invitationRows.map((row) => row.id)),
    ["invitation-new", "invitation-old", "invitation-revoked"]
  );
  assert.equal(invitationRows[0].revocable, true);
  assert.equal(invitationRows[1].revocable, false);
  assert.equal(client.vaultInvitationRows(null).length, 0);

  const shareLinkRows = client.folderShareLinkRows([
    {
      createdAt: "2026-07-01T00:00:00.000Z",
      expiresAt: "2026-07-30T00:00:00.000Z",
      id: "share-link-revoked",
      recipientNpub: "npub1gone",
      status: "revoked",
    },
    {
      createdAt: "2026-07-02T00:00:00.000Z",
      expiresAt: "2026-07-30T00:00:00.000Z",
      id: "share-link-pending",
      recipientNpub: "npub1waiting",
      status: "pending",
    },
  ]);
  assert.deepEqual(
    Array.from(shareLinkRows.map((row) => row.id)),
    ["share-link-pending", "share-link-revoked"]
  );
  assert.equal(shareLinkRows[0].revocable, true);

  const relationshipRows = client.sharedFolderRelationshipRows(
    {
      incoming: [
        {
          destinationVaultId: "dest",
          id: "sfi-incoming",
          sourceFolderId: "strategy",
          sourceVaultId: "acme",
          status: "pending",
        },
      ],
      outgoing: [
        {
          destinationVaultId: "partner",
          id: "sfi-outgoing",
          sourceFolderId: "playbooks",
          sourceVaultId: "dest",
          status: "revoked",
        },
      ],
    },
    {
      incoming: [],
      outgoing: [
        {
          destinationVaultId: "partner",
          id: "sfc-outgoing",
          memberNpubs: ["npub1a", "npub1b"],
          sourceFolderId: "playbooks",
          sourceVaultId: "dest",
          status: "active",
        },
      ],
    }
  );
  assert.deepEqual(
    Array.from(relationshipRows.map((row) => row.id)),
    ["sfc-outgoing", "sfi-incoming", "sfi-outgoing"]
  );
  const incomingInvitationRow = relationshipRows.find((row) => row.id === "sfi-incoming");
  assert.equal(incomingInvitationRow.acceptable, true);
  assert.equal(incomingInvitationRow.counterpartVaultId, "acme");
  const outgoingConnectionRow = relationshipRows.find((row) => row.id === "sfc-outgoing");
  assert.equal(outgoingConnectionRow.memberCount, 2);
  assert.equal(outgoingConnectionRow.counterpartVaultId, "partner");
  assert.equal(outgoingConnectionRow.acceptable, false);
  assert.equal(client.sharedFolderRelationshipRows(null, null).length, 0);

  const guideAllTodo = client.vaultGuideStepRows("missing", null, false);
  assert.deepEqual(
    Array.from(guideAllTodo.map((step) => step.done)),
    [false, false, false]
  );
  const guideAdminReady = client.vaultGuideStepRows(
    "connected",
    { kind: "organization" },
    true
  );
  assert.deepEqual(
    Array.from(guideAdminReady.map((step) => step.done)),
    [true, true, true]
  );
  const guidePersonalVault = client.vaultGuideStepRows(
    "connected",
    { kind: "personal" },
    true
  );
  assert.deepEqual(
    Array.from(guidePersonalVault.map((step) => step.done)),
    [true, false, false]
  );

  const accessFailure = accessFailureTestSeams();
  const handledAccessError = new Error("handled-access-error-sentinel");
  accessFailure.seams.state.sessionEpoch = 41;
  accessFailure.seams.state.sessionStatus = "unlocked";
  accessFailure.seams.state.lastError = null;
  const accessFeedback = accessFailure.elements.get("clientActionFeedback");
  accessFailure.seams.failAccessOperation(41, "Add member failed", handledAccessError);
  assert.equal(accessFailure.seams.state.accessResult?.tone, "error");
  assert.equal(accessFailure.seams.state.accessResult?.title, "Add member failed");
  assert.equal(accessFailure.seams.state.accessResult?.detail, "handled-access-error-sentinel");
  accessFailure.seams.reportClientActionFailure(handledAccessError);
  assert.equal(accessFeedback.hidden, true);
  assert.equal(accessFeedback.textContent, "");

  const inFlightSessionEpoch = accessFailure.seams.state.sessionEpoch;
  const staleAccessError = new Error("stale-access-error-sentinel");
  accessFailure.seams.lockSession();
  assert.equal(accessFeedback.hidden, true);
  accessFailure.seams.failAccessOperation(inFlightSessionEpoch, "Add member failed", staleAccessError);
  accessFailure.seams.reportClientActionFailure(staleAccessError);
  assert.equal(accessFeedback.hidden, true);
  assert.equal(accessFeedback.textContent, "");

  const accessLoss = accessFailureTestSeams();
  accessLoss.context.window.nostr = {
    signEvent: async (event) => ({ ...event, id: "auth-event", pubkey: "00".repeat(32), sig: "signature" }),
  };
  accessLoss.context.fetch = async () => ({
    ok: false,
    status: 403,
    text: async () => JSON.stringify({ error: "vault access required" }),
  });
  const accessLossState = accessLoss.seams.state;
  accessLossState.activeVaultId = "acme";
  accessLossState.config = { authScheme: "Nostr", publicBaseUrl: "http://finite.test" };
  accessLossState.keyring = accessLoss.context.window.FiniteBrainProductClient.createSessionKeyring();
  accessLossState.keyring.keys.set("acme/general@1", { rawKey: "folder-key-sentinel" });
  accessLossState.keyring.openedGrants.push({ folderId: "general", keyVersion: 1, vaultId: "acme" });
  accessLossState.metadata = { name: "Acme private metadata" };
  accessLossState.projection.pages.set("general/page", {
    folderId: "general",
    objectId: "page",
    text: "decrypted-page-sentinel",
  });
  accessLossState.readerBusy = true;
  accessLossState.sessionEpoch = 52;
  accessLossState.sessionStatus = "unlocked";
  accessLoss.context.document.getElementById("pageDraftInput").value = "plaintext-draft-sentinel";
  let capturedAccessLoss = null;
  await assert.rejects(
    () => accessLoss.seams.protectedRequest("/_admin/vaults/acme/metadata"),
    (error) => {
      capturedAccessLoss = error;
      assert.equal(error.status, 403);
      assert.equal(error.reason, "vault access required");
      assert.equal(error.path, "/_admin/vaults/acme/metadata");
      return true;
    }
  );
  assert.equal(accessLossState.sessionEpoch, 53);
  assert.equal(accessLossState.sessionStatus, "locked");
  assert.equal(
    accessLossState.sessionNotice,
    "Vault access changed. This session was locked. Select a Vault you can open, then unlock again."
  );
  assert.equal(accessLossState.keyring, null);
  assert.equal(accessLossState.metadata, null);
  assert.equal(accessLossState.projection.pages.size, 0);
  assert.equal(accessLossState.readerBusy, false);
  assert.equal(accessLoss.context.document.getElementById("pageDraftInput").value, "");
  accessLoss.seams.reportClientActionFailure(capturedAccessLoss);
  const accessLossFeedback = accessLoss.elements.get("clientActionFeedback");
  assert.equal(accessLossFeedback.hidden, true);
  assert.equal(accessLossFeedback.textContent, "");

  const staleAccessLoss = accessFailureTestSeams();
  staleAccessLoss.seams.state.activeVaultId = "acme";
  staleAccessLoss.seams.state.sessionEpoch = 81;
  staleAccessLoss.seams.state.sessionStatus = "unlocked";
  assert.equal(
    staleAccessLoss.seams.lockSessionForVaultAccessChange(activeVaultAccessLoss, 80),
    false,
    "A stale request must not lock or overwrite a newer session"
  );
  assert.equal(staleAccessLoss.seams.state.sessionEpoch, 81);
  assert.equal(staleAccessLoss.seams.state.sessionStatus, "unlocked");

  context.document.getElementById("pageDraftInput").value = "runtime-draft-sentinel";
  context.document.getElementById("folderKeyInput").value = "runtime-folder-key-sentinel";
  context.document.getElementById("okfBundleInput").value = "runtime-okf-sentinel";
  context.document.getElementById("vaultInviteSecretInput").value = "runtime-invite-secret-sentinel";
  context.document.getElementById("graphStats").textContent = "12 nodes / 18 links";
  context.document.getElementById("obsidianNewPageButton");
  context.document.getElementById("obsidianNewFolderButton");
  context.window.location = {
    hash: "#inviteSecret=invite-secret-sentinel&inviteEmail=not-an-email",
    href: "http://localhost/client#inviteSecret=invite-secret-sentinel&inviteEmail=not-an-email",
    pathname: "/client",
    search: "",
  };
  context.window.history = { replaceState() {} };
  assert.equal(client.populateInviteFromHash(), false);
  assert.equal(elements.get("clientActionFeedback").hidden, false);
  assert.equal(
    elements.get("clientActionFeedback").textContent,
    "Action could not be completed. Try again. If it continues, check your connection, signer, and unlocked session."
  );
  assert.doesNotMatch(elements.get("clientActionFeedback").textContent, /invite-secret-sentinel/);
  client.lockSession();
  assert.equal(elements.get("pageDraftInput").value, "");
  assert.equal(elements.get("folderKeyInput").value, "");
  assert.equal(elements.get("okfBundleInput").value, "");
  assert.equal(elements.get("vaultInviteSecretInput").value, "");
  assert.equal(elements.get("sessionSecurityTitle").textContent, "Session locked");
  assert.equal(
    elements.get("readerPageContent").textContent,
    "Session locked. Unlock to reopen encrypted Folder Key Grants."
  );
  assert.equal(elements.get("graphCanvas").children.length, 0);
  assert.equal(elements.get("graphStats").textContent, "0 nodes / 0 links");
  assert.equal(elements.get("obsidianNewPageButton").disabled, true);
  assert.equal(elements.get("obsidianNewFolderButton").disabled, true);
  assert.equal(elements.get("clientActionFeedback").hidden, true);
  assert.equal(elements.get("clientActionFeedback").textContent, "");

  console.log("product-client deterministic seams ok");
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
