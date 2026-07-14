const FiniteBrainProductClient = (() => {
  const SESSION_STATUS = Object.freeze({
    LOCKED: "locked",
    RESUMING: "resuming",
    UNLOCKED: "unlocked",
  });
  const state = {
    config: null,
    identityProvider: null,
    signerStatus: "checking",
    sessionStatus: SESSION_STATUS.LOCKED,
    sessionEpoch: 0,
    sessionNotice: null,
    pubkeyHex: null,
    activeVaultId: "personal",
    visibleVaults: [],
    metadata: null,
    keyring: null,
    lastError: null,
    clientActionFeedback: null,
    clientActionFeedbackGeneration: 0,
    preparedWrite: null,
    preparedWriteTarget: null,
    pageSaveInFlight: null,
    projection: createClientProjection(),
    readerBusy: false,
    selectedFolderId: null,
    selectedPageKey: null,
    graphZoom: 1,
    searchHighlight: null,
    searchHighlightShouldScroll: false,
    activeWorkspaceView: "page",
    activeSidebarMode: "files",
    settingsModalOpen: false,
    settingsSection: "session",
    settingsModalPreviousFocus: null,
    vaultSwitcherOpen: false,
    vaultSwitcherPreviousFocus: null,
    manageVaultsModalOpen: false,
    manageVaultsModalPreviousFocus: null,
    manageVaultsReturnToSettings: null,
    activeAccessFolderId: null,
    activeAccessIntent: "overview",
    accessBusy: false,
    accessResult: null,
    identityByNpub: new Map(),
    lastShareLinkId: null,
    lastVaultInvitationCode: null,
    lastVaultInvitationId: null,
    lastEmailInviteSecret: null,
    lastEmailInviteUrl: null,
    lastEmailInvitePostProof: null,
    vaultInvitations: null,
    folderShareLinks: null,
    folderShareLinksFolderId: null,
    sharedFolderInvitations: null,
    sharedFolderConnections: null,
    agentWorkspacePairings: null,
    editorMode: "visual",
    expandedFolderIds: new Set(),
    contextMenuTarget: null,
    contextMenuPreviousFocus: null,
    commandPaletteOpen: false,
    commandPaletteSelectedIndex: 0,
    editorSlashOpen: false,
    editorSlashQuery: "",
    editorSlashRange: null,
    editorSlashSelectedIndex: 0,
    accessFolderDropdownOpen: false,
    accessFolderFocusedIndex: 0,
    accessFolderDropdownListenerBound: false,
  };
  const handledAccessFailures = new WeakSet();
  const handledSessionLockFailures = new WeakSet();
  const hostedIdentityProviderStates = new WeakMap();
  let pendingInviteNavigation = null;
  let clientActionFeedbackTimer = null;

  const $ = (id) => document.getElementById(id);
  let lastErrorValue = state.lastError;
  Object.defineProperty(state, "lastError", {
    configurable: true,
    enumerable: true,
    get() {
      return lastErrorValue;
    },
    set(value) {
      const nextError = value || null;
      if (nextError) clearClientActionFeedback({ render: false });
      lastErrorValue = nextError;
      renderClientActionFeedback();
    },
  });
  const setOptionalDisabled = (id, disabled) => {
    const element = $(id);
    if (element) element.disabled = disabled;
  };
  const onOptionalClick = (id, handler) => {
    const element = $(id);
    if (element) element.addEventListener("click", handler);
  };
  const CIPHER = "AES-256-GCM";
  const FOLDER_OBJECT_VERSION = "finite-folder-object-v1";
  const FOLDER_OBJECT_PAGE_VERSION = "finite-folder-object-page-v1";
  const REVISION_VERSION = "finite-folder-object-revision-v1";
  const TOMBSTONE_VERSION = "finite-folder-object-tombstone-v1";
  const APP_EVENT_KIND = 30078;
  const BRAIN_IDENTITY_PROVIDER_VERSION = "finite-brain-identity-provider-v1";
  const BRAIN_SESSION_PROOF_REQUEST = "finite-brain-session-proof-request-v1";
  const BRAIN_SESSION_PROOF_RESPONSE = "finite-brain-session-proof-response-v1";
  const BRAIN_SESSION_ENDED = "finite-brain-session-ended-v1";
  const BRAIN_EVENT_KIND_BY_INTENT = Object.freeze({
    "folder-object-revision": APP_EVENT_KIND,
    "folder-object-tombstone": APP_EVENT_KIND,
    "vault-access-change": APP_EVENT_KIND,
    "vault-invite-authorization": APP_EVENT_KIND,
  });
  const BRAIN_EVENT_D_PREFIX_BY_INTENT = Object.freeze({
    "folder-object-revision": "finite-folder-object-revision:",
    "folder-object-tombstone": "finite-folder-object-tombstone:",
    "vault-access-change": "finite-vault-admin-access-change:",
    "vault-invite-authorization": "finite-email-invite-bootstrap-authorization:",
  });
  const MAX_OBJECT_ID_ATTEMPTS = 1000;
  const MAX_BRAIN_INVITE_BOOTSTRAP_FOLDERS = 100;
  const PERSONAL_VAULT_PLACEHOLDER_ID = "personal";
  const DEFAULT_CLIENT_FOLDER_ID = "getting-started";
  const VAULT_ACCESS_CHANGED_NOTICE =
    "Vault access changed. This session was locked. Select a Vault you can open, then unlock again.";
  const VAULT_ACCESS_REQUIRED_REASON = "vault access required";
  const CLIENT_ACTION_FEEDBACK = Object.freeze({
    inviteLinkCopyFailure: "Could not copy private invite link. Try again.",
    inviteLinkCopySuccess: "Private invite link copied.",
    folderIdCopyFailure: "Could not copy Folder ID. Try again.",
    folderIdCopySuccess: "Folder ID copied.",
    pageIdCopyFailure: "Could not copy Page ID. Try again.",
    pageIdCopySuccess: "Page ID copied.",
    failure:
      "Action could not be completed. Try again. If it continues, check your connection, signer, and unlocked session.",
  });
  const CLIENT_ACTION_FEEDBACK_DURATION_MS = 5000;
  const SESSION_PLAINTEXT_INPUT_IDS = [
    "accessAddPersonInput",
    "accessShareExpiresAtInput",
    "accessShareLinkInput",
    "accessShareMountInput",
    "accessShareTargetInput",
    "agentWorkspaceNpubInput",
    "commandPaletteInput",
    "manageOrganizationVaultNameInput",
    "pageBaseRevisionInput",
    "pageDraftInput",
    "pageFolderIdInput",
    "pageObjectIdInput",
    "sidebarSearchInput",
    "vaultAdminNpubInput",
    "vaultInviteCodeInput",
    "vaultInviteEmailInput",
    "vaultInviteEmailProofCreatedAtInput",
    "vaultInviteExpiresAtInput",
    "vaultInviteFoldersInput",
    "vaultInviteSecretInput",
    "vaultInviteTargetNpubInput",
    "vaultInviteUrlInput",
    "vaultMemberNpubInput",
  ];
  const DEFAULT_AGENTS_MARKDOWN =
    [
      "# AGENTS.md",
      "",
      "This is a FiniteBrain vault. Start with [[Getting Started]], then use",
      "[[How FiniteBrain Works]] and [[Access And Folders]] to understand the",
      "product model. Treat every readable Folder as its own encrypted, syncable",
      "LLM wiki scope.",
      "",
      "## Operating Model",
      "",
      "FiniteBrain stores encrypted Vault state on the server. Trusted clients and",
      "agent runtimes open Folder Key Grants locally, decrypt accessible Pages and",
      "Assets, edit ordinary files, then sync encrypted changes back. See",
      "[[How FiniteBrain Works]] for the technical spine and [[Access And Folders]] for",
      "the privacy boundary.",
      "",
      "Every participating keypair is a Member Identity. FiniteBrain does not classify",
      "that identity as human- or agent-controlled; access and attribution follow the",
      "acting public key.",
      "",
      "A Vault is not one giant wiki with folders. It is a namespace of many",
      "Folder-scoped LLM wikis. Folder access determines which wiki scopes can be read",
      "or written. The local scope contract lives in [[Getting Started Config]],",
      "with navigation in [[Getting Started Index]] and maintenance history in",
      "[[Getting Started Log]].",
      "",
      "## Use `fbrain`",
      "",
      "Use `fbrain` for identity, sync, access, and daemon state.",
      "",
      "Start here:",
      "",
      "```sh",
      "fbrain doctor --server \"$SERVER\"",
      "fbrain auth status --json",
      "fbrain open \"$VAULT\" \"$TREE\" --server \"$SERVER\"",
      "cd \"$TREE\"",
      "fbrain sync now --summary",
      "fbrain conflicts --json",
      "```",
      "",
      "Use an explicit config dir in agent runtimes:",
      "",
      "```sh",
      "fbrain --config-dir \"$HOME/.config/finitebrain\" auth status --json",
      "```",
      "",
      "Never print or expose Nostr secrets, Folder Keys, grant plaintext, auth files, decrypted sync internals, or rotation bodies.",
      "",
      "## Editing Rules",
      "",
      "Before editing:",
      "",
      "1. Sync; the operation reopens available encrypted grants in memory.",
      "2. Read this file.",
      "3. Read [[HUMANS.md]].",
      "4. Read [[Getting Started Index]], [[Getting Started Config]],",
      "   [[Getting Started Log]], `index.md`, or `SCHEMA.md` when present.",
      "5. Search before creating new pages.",
      "",
      "Only edit readable content. Do not edit `.finitebrain/`, encrypted sync evidence, locked metadata-only folders, generated state files, auth files, or key material.",
      "",
      "After editing:",
      "",
      "```sh",
      "fbrain sync now --summary",
      "fbrain conflicts --json",
      "```",
      "",
      "Resolve conflicts before reporting done.",
      "",
      "## LLM Wiki Rules",
      "",
      "Use each readable Folder as a durable LLM wiki scope.",
      "",
      "- The default `getting-started` Folder is the shared orientation scope for users and agents. Its starter map is [[Getting Started]].",
      "- The default `restricted` Folder is the starter tighter-boundary scope for sensitive work. If readable, its starter note is [[Restricted Folder Example]].",
      "- Keep raw sources immutable under that Folder's `raw/`.",
      "- Store non-Markdown source files under that Folder's `raw/assets/`.",
      "- Pair every Asset with a Markdown Source Note that records provenance, content type, hash or extraction status when known.",
      "- Cite Source Notes from synthesized wiki pages; do not make the blob itself the knowledge surface.",
      "- Put synthesized durable knowledge in that Folder's `wiki/`.",
      "- Prefer updating existing pages over creating duplicates.",
      "- Use wikilinks for internal relationships.",
      "- Keep the Folder-local `_index.md` current; this starter scope uses [[Getting Started Index]].",
      "- Append only to the Folder-local `log.md` after meaningful writes in that Folder; this starter scope uses [[Getting Started Log]].",
      "- Use `inventory/` for source candidates, open questions, watch items, and next actions.",
      "- Use `datasets/` for manifests, schemas, samples, and query recipes.",
      "- Use `output/` for reports, plans, summaries, and deliverables.",
      "- Archive superseded material instead of deleting it.",
      "- Answer from curated wiki pages first; say what is missing when evidence is thin.",
      "- Never summarize restricted Folder contents into a less-restricted Folder, index, log, or output.",
      "",
      "## Suggested Layout",
      "",
      "```text",
      "config.md",
      "_index.md",
      "log.md",
      "inbox/",
      "raw/",
      "  assets/",
      "wiki/",
      "inventory/",
      "datasets/",
      "output/",
      "archive/",
      "```",
      "",
      "Local folder instructions may override this layout. Human-facing context is in",
      "[[HUMANS.md]], and the seeded graph hub is [[Getting Started]].",
      "",
      "## Final Report",
      "",
      "When finished, report:",
      "",
      "- working tree path",
      "- acting email, if relevant",
      "- folders readable or locked",
      "- pages or sources created/updated/moved/deleted",
      "- index/log updates",
      "- sync summary",
      "- latest sequence, if available",
      "- whether conflicts are empty",
    ].join("\n") + "\n";
  const DEFAULT_HUMANS_MARKDOWN =
    [
      "# HUMANS.md",
      "",
      "This vault is your private, encrypted knowledge workspace.",
      "",
      "FiniteBrain keeps the server blind to page and asset contents. Your client or agent opens the vault locally, decrypts what you can access, edits ordinary files, then syncs encrypted changes back. [[How FiniteBrain Works]] explains that flow.",
      "",
      "A FiniteBrain vault is a namespace of wiki scopes. Each top-level Folder is its",
      "own LLM wiki with its own `_index.md`, `config.md`, and `log.md`. The",
      "starter orientation scope is mapped in [[Getting Started Index]], configured",
      "by [[Getting Started Config]], and recorded in [[Getting Started Log]].",
      "",
      "Inside a Folder:",
      "",
      "- `raw/` is source material.",
      "- `raw/assets/` is non-Markdown source files such as PDFs, images, audio, video, and datasets.",
      "- Source Notes are Markdown pages that explain those files and make them usable by agents.",
      "- `wiki/` is durable notes and synthesized understanding.",
      "- `inventory/` tracks things to revisit.",
      "- `datasets/` indexes structured references.",
      "- `output/` holds reports, plans, and finished work.",
      "- `log.md` records meaningful changes for that Folder only.",
      "",
      "The default `getting-started` Folder is for orientation and shared operating",
      "rules. The default `restricted` Folder demonstrates a tighter access boundary",
      "for private work.",
      "",
      "Read [[Getting Started]] for the first-page map, [[Access And Folders]] for",
      "sharing rules, and [[AGENTS.md]] for agent operating instructions. Agents",
      "should sync before editing, avoid duplicates, preserve sources, create Source",
      "Notes for assets, and keep the wiki useful for future work.",
    ].join("\n") + "\n";
  const defaultScopeConfigMarkdown = (folderId) => {
    const label = folderId === "restricted" ? "Restricted" : "Getting Started";
    const peerLabel = folderId === "restricted" ? "Getting Started" : "Restricted";
    return (
    [
      `# ${label} Config`,
      "",
      "This Folder is an independent FiniteBrain LLM wiki scope.",
      "",
      `Use [[${label} Index]] as the local navigation hub and append meaningful`,
      `maintenance to [[${label} Log]]. Shared product orientation starts at`,
      "[[Getting Started]], with related model notes in [[How FiniteBrain Works]]",
      "and [[Access And Folders]].",
      "",
      "Use this Folder's `raw/`, `raw/assets/`, `wiki/`, `inventory/`, `datasets/`, and `output/`",
      "directories for knowledge that belongs inside this access boundary. Keep this",
      "Folder's `_index.md` and `log.md` scoped only to pages in this Folder.",
      "",
      "Store non-Markdown source files in `raw/assets/` and pair each one with a",
      "Markdown Source Note in this Folder.",
      "",
      "Do not summarize restricted sibling Folder contents here unless the user",
      "explicitly chooses this Folder as an equal-or-more-restricted destination.",
      "",
      `Related default scope: ${peerLabel === "Restricted" ? "`restricted`" : "`getting-started`"}. Keep cross-Folder synthesis access-safe.`,
    ].join("\n") + "\n"
    );
  };
  const defaultScopeIndexMarkdown = (folderId) => {
    if (folderId === "restricted") {
      return (
        [
          "# Restricted Index",
          "",
          "This index maps the restricted starter wiki scope. It should describe only",
          "content that belongs inside this Folder's access boundary.",
          "",
          "## Local Pages",
          "",
          "- [[Restricted Folder Example]] explains this default tighter-boundary Folder.",
          "- [[Restricted Config]] defines the local wiki conventions.",
          "- [[Restricted Log]] records meaningful writes in this Folder only.",
          "",
          "## Related Orientation",
          "",
          "- [[Getting Started]] is the shared starter map.",
          "- [[How FiniteBrain Works]] explains trusted-client encryption and sync.",
          "- [[Access And Folders]] explains why restricted content must stay inside an equal-or-more-restricted destination.",
          "- [[AGENTS.md]] gives agent operating rules.",
        ].join("\n") + "\n"
      );
    }
    return (
      [
        "# Getting Started Index",
        "",
        "This index maps the shared orientation wiki scope.",
        "",
        "## Local Pages",
        "",
        "- [[Getting Started]] is the first-page map for a new Vault.",
        "- [[How FiniteBrain Works]] explains the trusted-client and encrypted-server model.",
        "- [[Access And Folders]] explains Folder-scoped access boundaries.",
        "- [[AGENTS.md]] gives agent operating rules.",
        "- [[HUMANS.md]] gives human-facing orientation.",
        "- [[Getting Started Config]] defines this scope's wiki conventions.",
        "- [[Getting Started Log]] records meaningful writes in this Folder only.",
        "",
        "## Boundaries",
        "",
        "Do not list private titles, summaries, source hints, assets, or activity from",
        "sibling Folders here. Link out only to product-safe default orientation.",
      ].join("\n") + "\n"
    );
  };
  const defaultScopeLogMarkdown = (folderId) => {
    const label = folderId === "restricted" ? "Restricted" : "Getting Started";
    return (
    [
      `# ${label} Log`,
      "",
      `Append meaningful changes in this Folder only. Keep [[${label} Index]] in`,
      `sync with durable pages and follow [[${label} Config]] for scope rules.`,
      "",
      "Do not record activity from sibling Folders here.",
    ].join("\n") + "\n"
    );
  };
  const DEFAULT_GETTING_STARTED_README_MARKDOWN =
    [
      "# Getting Started",
      "",
      "This Folder explains the default FiniteBrain vault layout.",
      "",
      "For humans, read [[HUMANS.md]]. For agents, read [[AGENTS.md]]. For the local",
      "scope map, use [[Getting Started Index]], [[Getting Started Config]], and",
      "[[Getting Started Log]].",
      "",
      "Default Folders:",
      "",
      "- `getting-started` is the shared orientation scope for users and agents. Keep",
      "  operating rules, onboarding notes, and vault-level guidance here.",
      "- `restricted` is the starter tighter-boundary scope for sensitive work. Do not",
      "  copy restricted titles, summaries, source notes, assets, or logs back here",
      "  unless the intended audience is allowed to read them.",
      "",
      "Core starter pages:",
      "",
      "- [[How FiniteBrain Works]] explains encrypted server state, local Folder Keys, Pages, Assets, and sync.",
      "- [[Access And Folders]] explains why every Folder is its own wiki boundary.",
      "- [[Restricted Folder Example]] is readable only when that Folder's key is open.",
      "",
      "Inside any Folder, keep non-Markdown source files as encrypted Assets under",
      "`raw/assets/`. Pair each Asset with a Markdown Source Note in the same Folder.",
      "Agents and synthesized wiki pages cite the Source Note; the Asset preserves the",
      "original bytes.",
      "",
      "Keep durable knowledge inside Folder-scoped `wiki/` pages, and keep private or",
      "sensitive work inside a Folder with an equal or tighter access boundary.",
      "",
      "Backlinks to keep this starter graph connected: [[HUMANS.md]], [[AGENTS.md]],",
      "[[Getting Started Index]], [[How FiniteBrain Works]], and [[Access And Folders]].",
    ].join("\n") + "\n";
  const DEFAULT_HOW_FINITEBRAIN_WORKS_MARKDOWN =
    [
      "# How FiniteBrain Works",
      "",
      "FiniteBrain stores encrypted Vault data on the server. The client or agent",
      "opens Folder Keys locally, decrypts the Pages and Assets it can access, edits",
      "ordinary files, and syncs encrypted updates back.",
      "",
      "This is the technical companion to [[Getting Started]] and should stay",
      "consistent with [[Access And Folders]], [[Getting Started Config]], and",
      "[[AGENTS.md]].",
      "",
      "Non-Markdown source files are encrypted as Assets and kept under `raw/assets/`.",
      "Agents use Markdown Source Notes to describe those Assets before synthesizing",
      "durable wiki pages from them.",
      "",
      "Each top-level Folder is an LLM wiki scope. A Folder has its own `config.md`,",
      "`_index.md`, and `log.md`, so activity and summaries stay inside the same",
      "access boundary as the content they describe.",
      "",
      "Graph View and backlinks are client-side projections over decrypted Pages. The",
      "server stores encrypted objects and sync records; it does not need plaintext",
      "page titles, links, backlinks, or wiki indexes.",
      "",
      "Related pages: [[Getting Started]], [[Access And Folders]], [[Getting Started Index]], [[HUMANS.md]], and [[AGENTS.md]].",
    ].join("\n") + "\n";
  const DEFAULT_ACCESS_AND_FOLDERS_MARKDOWN =
    [
      "# Access And Folders",
      "",
      "Access is Folder-scoped.",
      "",
      "Read this with [[How FiniteBrain Works]]: Folder Keys are why the wiki graph",
      "is built from readable local Pages instead of server-side plaintext indexing.",
      "",
      "- `getting-started` is the default shared orientation Folder.",
      "- `restricted` is the default example of a tighter access boundary.",
      "- Open Folders are intended for everyone who belongs in that Vault.",
      "- Restricted Folders are for material that should only be visible to approved",
      "  Member Identities.",
      "- Do not copy restricted titles, summaries, Source Notes, Assets, or log entries",
      "  into a less-restricted Folder.",
      "",
      "Use [[Getting Started]] and [[Getting Started Index]] for shared orientation.",
      "Use [[Restricted Folder Example]] only when the restricted Folder is readable.",
      "Agent rules live in [[AGENTS.md]], and human-facing orientation lives in",
      "[[HUMANS.md]].",
    ].join("\n") + "\n";
  const DEFAULT_RESTRICTED_EXAMPLE_MARKDOWN =
    [
      "# Restricted Folder Example",
      "",
      "This Folder demonstrates a tighter access boundary.",
      "",
      "It is the restricted counterpart to [[Getting Started]]. Keep local navigation",
      "in [[Restricted Index]], local rules in [[Restricted Config]], and local history",
      "in [[Restricted Log]].",
      "",
      "In an organization Vault, this Folder starts with access for admins only. Add",
      "specific members later when the work in this Folder should be shared with them.",
      "",
      "Keep this Folder's `_index.md` and `log.md` local to this Folder. Do not",
      "summarize this Folder into `getting-started` unless the user explicitly chooses",
      "that destination and the audience is allowed to see the summary.",
      "",
      "Related shared pages: [[Access And Folders]], [[How FiniteBrain Works]],",
      "[[AGENTS.md]], and [[HUMANS.md]].",
    ].join("\n") + "\n";
  const defaultPage = (folderId, objectId, path, markdown) =>
    Object.freeze({ folderId, objectId, path, markdown });
  const defaultScopePages = (folderId) => [
    defaultPage(
      folderId,
      `obj_default_${folderId}_scope_config`,
      "config.md",
      defaultScopeConfigMarkdown(folderId)
    ),
    defaultPage(folderId, `obj_default_${folderId}_scope_index`, "_index.md", defaultScopeIndexMarkdown(folderId)),
    defaultPage(folderId, `obj_default_${folderId}_scope_log`, "log.md", defaultScopeLogMarkdown(folderId)),
  ];
  const defaultPrimaryScopePages = (folderId) => [
    defaultPage(folderId, "obj_default_agents", "AGENTS.md", DEFAULT_AGENTS_MARKDOWN),
    defaultPage(folderId, "obj_default_humans", "HUMANS.md", DEFAULT_HUMANS_MARKDOWN),
    ...defaultScopePages(folderId),
  ];
  const gettingStartedGuidePages = () => [
    defaultPage(
      "getting-started",
      "obj_default_getting-started_readme",
      "README.md",
      DEFAULT_GETTING_STARTED_README_MARKDOWN
    ),
    defaultPage(
      "getting-started",
      "obj_default_getting-started_how_finitebrain_works",
      "wiki/how-finitebrain-works.md",
      DEFAULT_HOW_FINITEBRAIN_WORKS_MARKDOWN
    ),
    defaultPage(
      "getting-started",
      "obj_default_getting-started_access_and_folders",
      "wiki/access-and-folders.md",
      DEFAULT_ACCESS_AND_FOLDERS_MARKDOWN
    ),
  ];
  const restrictedGuidePage = () =>
    defaultPage(
      "restricted",
      "obj_default_restricted_example",
      "wiki/restricted-folder-example.md",
      DEFAULT_RESTRICTED_EXAMPLE_MARKDOWN
    );
  const starterVaultPages = () => [
    ...defaultPrimaryScopePages("getting-started"),
    ...gettingStartedGuidePages(),
    ...defaultScopePages("restricted"),
    restrictedGuidePage(),
  ];
  const PERSONAL_DEFAULT_VAULT_PAGES = Object.freeze([
    ...starterVaultPages(),
  ]);
  const ORGANIZATION_DEFAULT_VAULT_PAGES = Object.freeze([
    ...starterVaultPages(),
  ]);
  const BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
  const graphViewport = { height: 560, width: 900 };
  const GRAPH_ZOOM_MAX = 2.5;
  const GRAPH_ZOOM_MIN = 0.5;
  const GRAPH_ZOOM_STEP = 1.25;
  const EDITOR_SLASH_COMMANDS = [
    { id: "paragraph", label: "Paragraph", detail: "Normal text", aliases: ["p", "text"] },
    { id: "heading1", label: "Heading 1", detail: "Large section title", aliases: ["h1", "title"] },
    { id: "heading2", label: "Heading 2", detail: "Section heading", aliases: ["h2", "subtitle"] },
    { id: "bullet", label: "Bulleted list", detail: "Start a list", aliases: ["ul", "list"] },
    { id: "quote", label: "Quote", detail: "Callout or excerpt", aliases: ["blockquote", "callout"] },
    { id: "codeblock", label: "Code block", detail: "Fenced code", aliases: ["pre", "code block", "fence"] },
    { id: "code", label: "Inline code", detail: "Code text", aliases: ["backtick", "mono"] },
    { id: "bold", label: "Bold", detail: "Strong emphasis", aliases: ["b", "strong"] },
    { id: "italic", label: "Italic", detail: "Soft emphasis", aliases: ["i", "em"] },
    { id: "link", label: "Link", detail: "Add a URL", aliases: ["url", "href"] },
    { id: "rule", label: "Divider", detail: "Horizontal rule", aliases: ["hr", "line"] },
  ];

  function shortKey(value) {
    if (!value) return "-";
    if (value.length <= 18) return value;
    return `${value.slice(0, 10)}...${value.slice(-8)}`;
  }

  function renderClientActionFeedback() {
    const feedback = $("clientActionFeedback");
    if (!feedback) return;
    const feedbackState = state.clientActionFeedback ||
      (lastErrorValue ? { message: CLIENT_ACTION_FEEDBACK.failure, tone: "error" } : null);
    const visible = Boolean(feedbackState);
    feedback.hidden = !visible;
    feedback.textContent = feedbackState?.message || "";
    if (feedbackState) feedback.dataset.tone = feedbackState.tone;
    else delete feedback.dataset.tone;
  }

  function clearClientActionFeedbackTimer() {
    if (clientActionFeedbackTimer === null) return;
    if (typeof window.clearTimeout === "function") {
      window.clearTimeout(clientActionFeedbackTimer);
    }
    clientActionFeedbackTimer = null;
  }

  function clearClientActionFeedback(options = {}) {
    clearClientActionFeedbackTimer();
    state.clientActionFeedbackGeneration += 1;
    state.clientActionFeedback = null;
    if (options.render !== false) renderClientActionFeedback();
    return state.clientActionFeedbackGeneration;
  }

  function setClientActionFeedback(tone, message, options = {}) {
    const generation = options.generation ?? state.clientActionFeedbackGeneration + 1;
    if (options.generation !== undefined && generation !== state.clientActionFeedbackGeneration) {
      return false;
    }
    // The newest successful client action supersedes an older generic error.
    // Otherwise that error would reappear when this short-lived notice expires.
    if (tone === "success") lastErrorValue = null;
    clearClientActionFeedbackTimer();
    state.clientActionFeedbackGeneration = generation;
    state.clientActionFeedback = { message, tone };
    if (options.expires !== false && typeof window.setTimeout === "function") {
      clientActionFeedbackTimer = window.setTimeout(() => {
        if (state.clientActionFeedbackGeneration !== generation) return;
        state.clientActionFeedback = null;
        clientActionFeedbackTimer = null;
        renderClientActionFeedback();
      }, CLIENT_ACTION_FEEDBACK_DURATION_MS);
    }
    renderClientActionFeedback();
    return true;
  }

  function reportClientActionFailure(error) {
    if (error && typeof error === "object" && handledAccessFailures.has(error)) return;
    if (error && typeof error === "object" && handledSessionLockFailures.has(error)) return;
    state.lastError = error instanceof Error ? error.message : String(error || "Action failed");
  }

  function markAccessFailureHandled(error) {
    if (error && typeof error === "object") handledAccessFailures.add(error);
  }

  function markSessionLockFailureHandled(error) {
    if (error && typeof error === "object") handledSessionLockFailures.add(error);
  }

  function publicKeyIdentityFromInput(input) {
    const value = String(input || "").trim();
    if (!value) return null;
    if (/^[0-9a-fA-F]{64}$/.test(value)) {
      const hex = value.toLowerCase();
      const npub = npubFromHex(hex);
      return { npub, hex, display: shortKey(npub), nip05: null, relays: [], verifiedAt: null };
    }
    try {
      const hex = npubToHex(value);
      const npub = npubFromHex(hex);
      return { npub, hex, display: shortKey(npub), nip05: null, relays: [], verifiedAt: null };
    } catch (_) {
      return null;
    }
  }

  function normalizeIdentityResponse(identity) {
    if (!identity) return null;
    const npub = identity.npub || identity.userId || identity.user_id || "";
    if (!npub) return null;
    return {
      npub,
      hex: identity.hex || identity.publicKeyHex || identity.public_key_hex || null,
      display: identity.display || identity.nip05 || shortKey(npub),
      nip05: identity.nip05 || null,
      relays: identity.relays || [],
      verifiedAt: identity.verifiedAt || identity.verified_at || null,
    };
  }

  function rememberIdentity(identity) {
    const normalized = normalizeIdentityResponse(identity);
    if (!normalized) return null;
    state.identityByNpub.set(normalized.npub, normalized);
    return normalized;
  }

  function rememberIdentitiesFrom(value) {
    if (!value) return;
    if (Array.isArray(value)) {
      value.forEach(rememberIdentitiesFrom);
      return;
    }
    if (Array.isArray(value.identities)) {
      value.identities.forEach(rememberIdentity);
    }
    for (const key of ["invitations", "shareLinks", "outgoing", "incoming"]) {
      if (Array.isArray(value[key])) value[key].forEach(rememberIdentitiesFrom);
    }
  }

  function identityForNpub(npub) {
    const value = String(npub || "");
    if (!value) return null;
    return state.identityByNpub.get(value) || null;
  }

  function looksLikeEmailIdentity(value) {
    return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(String(value || "").trim());
  }

  function identityEmailDisplay(identity) {
    if (!identity) return null;
    if (identity.nip05) return identity.nip05;
    if (looksLikeEmailIdentity(identity.display)) return identity.display;
    if (looksLikeEmailIdentity(identity.name)) return identity.name;
    if (looksLikeEmailIdentity(identity.displayName)) return identity.displayName;
    if (looksLikeEmailIdentity(identity.display_name)) return identity.display_name;
    return null;
  }

  function identityMetadataForNpub(npub) {
    const value = String(npub || "");
    const identity = identityForNpub(value);
    const email = identityEmailDisplay(identity);
    const display = email || shortKey(value);
    const status = email
      ? "Email or NIP-05 metadata resolved"
      : "No email or NIP-05 metadata loaded";
    const details = [
      {
        label: "Email / NIP-05",
        value: email || "Not resolved in this client",
      },
      {
        label: "Public key",
        value: identity?.npub || value || "-",
      },
    ];
    if (identity?.hex) {
      details.push({ label: "Hex key", value: identity.hex });
    }
    if (identity?.relays?.length) {
      details.push({ label: "Relays", value: identity.relays.join(", ") });
    }
    if (identity?.verifiedAt) {
      details.push({ label: "Verified", value: identity.verifiedAt });
    }
    return {
      details,
      display,
      email,
      npub: identity?.npub || value,
      status,
      tooltip: email
        ? `${email}. Public key: ${shortKey(identity?.npub || value)}`
        : `Showing public key fallback. ${status}.`,
    };
  }

  function identityDisplay(npub) {
    return identityMetadataForNpub(npub).display;
  }

  async function resolveIdentityInputValue(input, message) {
    const value = String(input || "").trim();
    if (!value) throw new Error(message);
    const local = publicKeyIdentityFromInput(value);
    if (local) return rememberIdentity(local);
    const resolved = await protectedRequest("/_admin/identities/resolve", {
      method: "POST",
      body: JSON.stringify({ input: value }),
    });
    return rememberIdentity(resolved);
  }

  function personalVaultIdForPubkey(pubkeyHex) {
    return pubkeyHex ? `personal-${pubkeyHex.slice(0, 16)}` : PERSONAL_VAULT_PLACEHOLDER_ID;
  }

  function signerIdentityChanged(previousPubkeyHex, nextPubkeyHex) {
    return Boolean(
      previousPubkeyHex &&
        nextPubkeyHex &&
        previousPubkeyHex !== nextPubkeyHex
    );
  }

  function signedEventMatchesPinnedIdentity(expectedPubkeyHex, signedEvent) {
    return Boolean(
      expectedPubkeyHex &&
        typeof signedEvent?.pubkey === "string" &&
        signedEvent.pubkey.toLowerCase() === expectedPubkeyHex.toLowerCase()
    );
  }

  function normalizeVisibleVault(vault) {
    const vaultId = vault?.vaultId || vault?.vault_id || vault?.id || "";
    if (!vaultId) return null;
    const kind = String(vault.kind || "organization").toLowerCase();
    return {
      vaultId,
      kind: kind === "personal" ? "personal" : "organization",
      name: vault.name || (kind === "personal" ? "Personal vault" : vaultId),
      role: vault.role || (kind === "personal" ? "owner" : "member"),
      inviteCode: vault.inviteCode || vault.invite_code || null,
    };
  }

  function defaultPersonalVault() {
    return {
      vaultId: personalVaultIdForPubkey(state.pubkeyHex),
      kind: "personal",
      name: "Personal vault",
      role: "owner",
      pending: true,
    };
  }

  function visibleVaultOptions(vaults = state.visibleVaults) {
    const normalized = vaults.map(normalizeVisibleVault).filter(Boolean);
    const personal = normalized.find((vault) => vault.kind === "personal") || defaultPersonalVault();
    const organizations = normalized
      .filter((vault) => vault.kind === "organization")
      .sort((left, right) => left.name.localeCompare(right.name) || left.vaultId.localeCompare(right.vaultId));
    return [personal, ...organizations];
  }

  function activeVaultOption() {
    return visibleVaultOptions().find((vault) => vault.vaultId === state.activeVaultId) || defaultPersonalVault();
  }

  function activeVaultLabel() {
    const lockedSelection = lockedVaultSelection(
      state.sessionStatus,
      state.activeVaultId,
      state.visibleVaults
    );
    if (lockedSelection) return lockedSelection.label;
    return state.metadata?.name || activeVaultOption()?.name || state.activeVaultId || "Personal vault";
  }

  function nestedManageVaultsReturnToken() {
    if (!state.manageVaultsModalOpen || !state.manageVaultsReturnToSettings) return null;
    // This only carries a Settings section and a DOM focus target. It is not
    // session content, so a nested Manage Vaults reset can return safely.
    return state.manageVaultsReturnToSettings;
  }

  function resetVaultSessionState(options = {}) {
    const returnToSettings =
      options.preserveManageVaultsReturnToSettings === false ? null : nestedManageVaultsReturnToken();
    state.sessionEpoch += 1;
    pendingInviteNavigation = null;
    clearSessionSecretsAndPlaintext(state);
    if (returnToSettings) state.manageVaultsReturnToSettings = returnToSettings;
    clearSessionOwnedDom();
  }

  function clearSessionOwnedDom() {
    for (const id of SESSION_PLAINTEXT_INPUT_IDS) {
      const input = $(id);
      if (!input) continue;
      if (input.type === "checkbox") input.checked = false;
      else input.value = "";
    }
    for (const id of [
      "accessFolderList",
      "accessResultPanel",
      "accessWhoHasList",
      "commandPaletteList",
      "contextMenu",
      "editorSlashMenu",
      "folderShareLinkList",
      "graphCanvas",
      "readerFolderList",
      "sharedFolderList",
      "sidebarSearchResults",
      "vaultInvitationList",
      "vaultPeopleList",
      "vaultSwitcherList",
      "manageVaultsList",
    ]) {
      $(id)?.replaceChildren?.();
    }
    setText("readerPageTitle", "Session locked");
    setText("readerPagePath", "Unlock the session to reopen encrypted Folder Key Grants");
    const readerContent = $("readerPageContent");
    if (readerContent) {
      readerContent.replaceChildren?.();
      readerContent.textContent = "Session locked. Unlock to reopen encrypted Folder Key Grants.";
    }
    if (typeof document.title === "string") document.title = "FiniteBrain";
    setPill("graphStats", "0 nodes / 0 links", "muted");
    setText("graphEmptyTitle", "No graph yet");
    setText("graphEmptyCopy", "Unlock the session to rebuild the local graph.");
    setText("graphZoomValue", "100%");
    const graphEmptyState = $("graphEmptyState");
    if (graphEmptyState) graphEmptyState.hidden = false;
    const editorDrawer = $("editorDrawer");
    if (editorDrawer) editorDrawer.open = false;
    for (const id of ["commandPalette", "contextMenu", "editorSlashMenu"]) {
      const element = $(id);
      if (element) element.hidden = true;
    }
  }

  function lockSession() {
    resetVaultSessionState({ preserveManageVaultsReturnToSettings: false });
    render();
    log("Locked Product Client session.", { status: state.sessionStatus });
  }

  function lockSessionForVaultAccessChange(error, requestEpoch) {
    if (
      !sessionOperationIsCurrent(state.sessionEpoch, requestEpoch, state.sessionStatus) ||
      !isActiveVaultAuthorizationLoss(error, state.activeVaultId)
    ) {
      return false;
    }
    markSessionLockFailureHandled(error);
    resetVaultSessionState({ preserveManageVaultsReturnToSettings: false });
    state.sessionNotice = VAULT_ACCESS_CHANGED_NOTICE;
    render();
    log("Locked Product Client session after Vault access changed.", {
      status: state.sessionStatus,
    });
    return true;
  }

  function handlePageHide() {
    lockSession();
  }

  function handlePageShow(event) {
    if (event?.persisted) lockSession();
  }

  async function resumeSession() {
    return loadVaultReader({ allowResume: true });
  }

  function sessionContainsSecretsOrPlaintext(target) {
    return Boolean(
      target.keyring ||
        target.metadata ||
        target.preparedWrite ||
        target.preparedWriteTarget ||
        target.lastEmailInviteSecret ||
        target.lastEmailInviteUrl ||
        target.lastEmailInvitePostProof ||
        target.accessResult ||
        target.visibleVaults?.length ||
        target.vaultInvitations?.length ||
        target.folderShareLinks?.length ||
        target.sharedFolderInvitations?.length ||
        target.sharedFolderConnections?.length ||
        target.identityByNpub?.size ||
        target.projection?.pages?.size ||
        target.projection?.localDrafts?.size ||
        target.projection?.conflicts?.length
    );
  }

  function sessionOperationIsCurrent(currentEpoch, operationEpoch, status) {
    return currentEpoch === operationEpoch && status !== SESSION_STATUS.LOCKED;
  }

  function requireCurrentSessionEpoch(epoch) {
    if (sessionOperationIsCurrent(state.sessionEpoch, epoch, state.sessionStatus)) return;
    if (state.sessionEpoch === epoch) {
      clearSessionSecretsAndPlaintext(state);
      clearSessionOwnedDom();
    }
    throw new Error("Session changed while protected client work was in progress; unlock again");
  }

  function setActiveVaultId(vaultId, options = {}) {
    const nextVaultId = vaultId || state.activeVaultId || personalVaultIdForPubkey(state.pubkeyHex);
    const changed = nextVaultId !== state.activeVaultId;
    state.activeVaultId = nextVaultId;
    if (changed && options.reset !== false) resetVaultSessionState();
  }

  function lockedVaultSelection(status, activeVaultId, visibleVaults) {
    if (status === SESSION_STATUS.UNLOCKED || visibleVaults.length) return null;
    return {
      label: "Selected Vault (locked)",
      value: activeVaultId || PERSONAL_VAULT_PLACEHOLDER_ID,
    };
  }

  function missingVisibleVaultFallback(
    status,
    activeVaultId,
    visibleVaults,
    pubkeyHex,
    defaultVaultId
  ) {
    if (
      status === SESSION_STATUS.LOCKED ||
      !activeVaultId ||
      activeVaultId === PERSONAL_VAULT_PLACEHOLDER_ID ||
      activeVaultId === defaultVaultId
    ) {
      return null;
    }
    const normalized = visibleVaults.map(normalizeVisibleVault).filter(Boolean);
    if (normalized.some((vault) => vault.vaultId === activeVaultId)) return null;
    const personal = normalized.find((vault) => vault.kind === "personal");
    const fallbackVaultId = personal?.vaultId || normalized[0]?.vaultId || personalVaultIdForPubkey(pubkeyHex);
    return fallbackVaultId && fallbackVaultId !== activeVaultId ? fallbackVaultId : null;
  }

  function vaultIdFromName(prefix, name) {
    const slug =
      String(name || prefix)
        .trim()
        .toLowerCase()
        .replace(/[^a-z0-9_-]+/g, "-")
        .replace(/^-+|-+$/g, "")
        .slice(0, 48) || prefix;
    return `${prefix}-${slug}-${Date.now().toString(36)}`.slice(0, 128);
  }

  function rememberVisibleVault(metadata) {
    if (!metadata?.vaultId) return;
    const actorNpub = state.pubkeyHex ? npubFromHex(state.pubkeyHex) : null;
    const vault = normalizeVisibleVault({
      vaultId: metadata.vaultId,
      kind: metadata.kind,
      name: metadata.name,
      role: metadataVaultRole(metadata, actorNpub),
    });
    if (!vault) return;
    state.visibleVaults = [
      vault,
      ...state.visibleVaults.filter((candidate) => normalizeVisibleVault(candidate)?.vaultId !== vault.vaultId),
    ];
  }

  function metadataVaultRole(metadata, actorNpub) {
    if (metadata?.kind === "personal") {
      return actorNpub && metadata.ownerUserId === actorNpub ? "owner" : "member";
    }
    return actorNpub && metadata?.admins?.includes(actorNpub) ? "admin" : "member";
  }

  function deriveSignerState(provider) {
    if (!provider) {
      return {
        status: "unavailable",
        label: "missing",
        detail: "No NIP-07 signer was found in this browser.",
        canConnect: false,
      };
    }
    if (typeof provider.getPublicKey !== "function" || typeof provider.signEvent !== "function") {
      return {
        status: "unsupported",
        label: "unsupported",
        detail: "A signer is present, but it does not expose getPublicKey and signEvent.",
        canConnect: false,
      };
    }
    return {
      status: "ready",
      label: "ready",
      detail: "NIP-07 signer detected. Connect to load protected Vault state.",
      canConnect: true,
    };
  }

  function deriveBrainIdentityProviderState(provider) {
    if (!provider) {
      return {
        status: "setup_required",
        label: "setup required",
        detail: "Set up your Finite Chat Hosted Device before opening Brain.",
        canConnect: false,
      };
    }
    const supported =
      provider.version === BRAIN_IDENTITY_PROVIDER_VERSION &&
      typeof provider.identifyMember === "function" &&
      typeof provider.authorizeHttpRequest === "function" &&
      typeof provider.authorizeBrainEvent === "function" &&
      typeof provider.openGrantPayload === "function" &&
      typeof provider.wrapGrantPayload === "function";
    if (!supported) {
      return {
        status: "unsupported",
        label: "unsupported",
        detail: "The available Brain Identity Provider does not support the required contract.",
        canConnect: false,
      };
    }
    const hostedState = hostedIdentityProviderStates.get(provider);
    if (hostedState && hostedState.status !== "ready") {
      return {
        status: hostedState.status,
        label: hostedState.status === "setup_required" ? "setup required" : "checking",
        detail:
          hostedState.detail ||
          (hostedState.status === "setup_required"
            ? "Set up your Finite Chat Hosted Device before opening Brain."
            : "Checking the hosted Brain identity."),
        canConnect: false,
      };
    }
    return {
      status: "ready",
      label: "ready",
      detail: "Brain Identity Provider ready. Connect to load protected Vault state.",
      canConnect: true,
    };
  }

  function configureBrainIdentityProvider(provider) {
    const derived = deriveBrainIdentityProviderState(provider);
    state.identityProvider = provider || null;
    state.signerStatus = derived.status;
    return derived;
  }

  function requireBrainEventAuthorizer(intent, options = {}) {
    if (options.signEvent) return options.signEvent;
    if (typeof options.provider?.signEvent === "function") {
      return (eventTemplate) => options.provider.signEvent.call(options.provider, eventTemplate);
    }
    const provider = options.brainIdentityProvider || state.identityProvider;
    const derived = deriveBrainIdentityProviderState(provider);
    if (!derived.canConnect) throw new Error(derived.detail);
    return async (eventTemplate) => {
      const signed = await provider.authorizeBrainEvent({ intent, eventTemplate });
      if (
        provider === state.identityProvider &&
        state.pubkeyHex &&
        !signedEventMatchesPinnedIdentity(state.pubkeyHex, signed)
      ) {
        expireBrainIdentitySession();
        throw new Error("Brain identity changed while authorizing this action. Reopen Brain from Chat.");
      }
      return signed;
    };
  }

  function normalizeAccessValue(access) {
    const value = String(access || "unknown")
      .trim()
      .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
      .replace(/[-\s]+/g, "_")
      .toLowerCase();
    return value || "unknown";
  }

  function folderAccessValue(folder) {
    return normalizeAccessValue(folder?.access ?? folder?.accessMode ?? folder?.access_mode);
  }

  function folderAccessUsers(folder) {
    return folder?.accessUserIds || folder?.access_user_ids || [];
  }

  function folderStatus(folder) {
    if (folder?.setupIncomplete ?? folder?.setup_incomplete) return "setup";
    if (folderAccessValue(folder) === "restricted" && folderAccessUsers(folder).length === 0) {
      return "locked";
    }
    return "ready";
  }

  function folderAccessLabel(access) {
    const normalized = normalizeAccessValue(access);
    return (
      {
        admin_only: "admin only",
        all_members: "all members",
        owner: "owner",
        restricted: "restricted",
      }[normalized] || normalized.replaceAll("_", " ")
    );
  }

  function metadataFolderRows(metadata) {
    return (metadata?.folders || []).map((folder) => {
      const access = folderAccessValue(folder);
      const status = folderStatus(folder);
      const accessLabel = folderAccessLabel(access);
      const flags = [];
      if (folder.sharedFolderSource ?? folder.shared_folder_source) flags.push("source");
      if (folder.setupIncomplete ?? folder.setup_incomplete) flags.push("setup needed");
      if (status === "locked") flags.push("locked");
      const currentKeyVersion = folder.currentKeyVersion ?? folder.current_key_version ?? 1;
      return {
        access,
        accessLabel,
        accessUserIds: folderAccessUsers(folder),
        currentKeyVersion,
        id: folder.id,
        path: folder.path,
        setupIncomplete: Boolean(folder.setupIncomplete ?? folder.setup_incomplete),
        sharedFolderSource: Boolean(folder.sharedFolderSource ?? folder.shared_folder_source),
        status,
        label: `${folder.path} - ${accessLabel} - key v${currentKeyVersion}`,
        detail: flags.join(", "),
      };
    });
  }

  function folderKeyVersionKey(folderId, keyVersion) {
    return `${folderId}@${keyVersion || 1}`;
  }

  function accessBadgesForFolder(row, openedFolderKeys = new Set()) {
    if (!row) return [];
    const badges = [];
    if (row.access === "admin_only") {
      badges.push({ kind: "access", label: "admin", tone: "warn" });
    } else if (row.access === "restricted") {
      badges.push({ kind: "access", label: "restricted", tone: "warn" });
    } else if (row.access === "all_members") {
      badges.push({ kind: "access", label: "all", tone: "muted" });
    } else {
      badges.push({ kind: "access", label: row.accessLabel || "access", tone: "muted" });
    }
    if (row.sharedFolderSource) badges.push({ kind: "shared", label: "shared", tone: "ready" });
    if (row.setupIncomplete) badges.push({ kind: "setup", label: "setup", tone: "error" });
    if (row.status === "locked" || (row.pageCount > 0 && row.readableCount === 0)) {
      badges.push({ kind: "locked", label: "locked", tone: "warn" });
    }
    if (openedFolderKeys.has(folderKeyVersionKey(row.id, row.currentKeyVersion))) {
      badges.push({ kind: "key", label: "key open", tone: "ready" });
    }
    badges.push({ kind: "version", label: `v${row.currentKeyVersion || 1}`, tone: "muted" });
    return badges;
  }

  function sidebarAccessBadgesForFolder(row, openedFolderKeys = new Set()) {
    return [];
  }

  function accessActionRoute(action, target) {
    if (!target?.folderId) return null;
    if (action === "share-folder") {
      return { folderId: target.folderId, intent: "links", settingsSection: "access" };
    }
    if (action === "manage-access") {
      return { folderId: target.folderId, intent: "people", settingsSection: "access" };
    }
    if (action === "inspect-access") {
      return { folderId: target.folderId, intent: "overview", settingsSection: "access" };
    }
    return null;
  }

  function accessIntentValue(intent) {
    if (intent === "share" || intent === "links") return "links";
    if (intent === "manage" || intent === "people") return "people";
    return "overview";
  }

  // Intent only affects the Settings Access chrome such as expanded share links
  // or the add-person form. Vault selection lives in the footer and Manage Vaults.
  function applyAccessIntentChrome(row) {
    const intent = accessIntentValue(state.activeAccessIntent);
    const advancedSection = $("accessAdvancedSection");
    const addPanel = $("accessAddPersonPanel");
    const addForm = $("accessAddPersonForm");
    if (!row) return;

    if (intent === "links" && advancedSection) {
      advancedSection.open = true;
    }

    const canManage =
      folderAllowsDirectGrant(row) &&
      hasOpenedAccessFolderKey(row) &&
      state.signerStatus === "connected";
    if (intent === "people" && canManage && addPanel && addForm) {
      addPanel.hidden = false;
      addPanel.open = true;
      addForm.hidden = false;
    }
  }

  function accessPanelState(intent, row) {
    const mode = accessIntentValue(intent);
    if (!row) {
      return {
        detail: "Load a Vault and select a Folder to inspect access.",
        mode,
        status: "empty",
        title: "No Folder selected",
        tone: "muted",
      };
    }
    const pageDetail = readerFolderDetail(row);
    return {
      detail: `${pageDetail} in this Folder`,
      mode,
      status: row.accessLabel,
      title: row.path,
      tone: row.status === "ready" ? "ready" : "warn",
    };
  }

  function countLabel(count, singular, plural = `${singular}s`) {
    return `${count} ${count === 1 ? singular : plural}`;
  }

  function accessAudienceSummary(row) {
    if (!row) return "-";
    if (row.access === "owner") return "Owner only";
    if (row.access === "admin_only") return "Admins";
    if (row.access === "all_members") return "All members";
    if (row.access === "restricted") return "Restricted";
    return row.accessLabel || "Unknown";
  }

  function accessPeopleSummary(row, metadata) {
    if (!row) return "-";
    const explicitCount = row.accessUserIds?.length || 0;
    const adminCount = metadata?.admins?.length || 0;
    const memberCount = metadata?.members?.length || 0;
    if (row.access === "owner") return "Owner";
    if (row.access === "admin_only") return countLabel(adminCount, "admin");
    if (row.access === "all_members") return countLabel(memberCount, "member");
    if (row.access === "restricted" && metadata?.kind === "organization") {
      return explicitCount
        ? `${countLabel(adminCount, "admin")} + ${countLabel(explicitCount, "Member Identity", "Member Identities")}`
        : `${countLabel(adminCount, "admin")}`;
    }
    if (row.access === "restricted") {
      return explicitCount ? countLabel(explicitCount, "Member Identity", "Member Identities") : "Owner only";
    }
    return "-";
  }

  function accessKeySummary(row, openedFolderKeys) {
    if (!row) return "-";
    const keyVersion = row.currentKeyVersion || 1;
    const keyOpen = openedFolderKeys.has(folderKeyVersionKey(row.id, keyVersion));
    return `${keyOpen ? "Open" : "Closed"} v${keyVersion}`;
  }

  function accessPagesSummary(row) {
    if (!row) return "-";
    if (!row.pageCount) return "0 pages";
    if (row.readableCount === row.pageCount) return pageCountLabel(row.pageCount);
    if (!row.readableCount) return `${pageCountLabel(row.pageCount)} locked`;
    return `${row.readableCount}/${row.pageCount} readable`;
  }

  function accessOverviewCopy(row, metadata, openedFolderKeys) {
    if (!row) return "Load a Vault to inspect Folder access.";
    const keyOpen = openedFolderKeys.has(folderKeyVersionKey(row.id, row.currentKeyVersion || 1));
    if (row.setupIncomplete) return "This Folder still needs setup before its current key state is reliable.";
    if (row.access === "owner") return "Only the personal Vault owner should be able to open this Folder.";
    if (row.access === "admin_only") return "Vault admins can open this Folder. Ordinary members cannot.";
    if (row.access === "all_members") return "Every member of this Vault can open this Folder after their Folder Key is available.";
    if (row.access === "restricted" && metadata?.kind === "organization") {
      return keyOpen
        ? "Admins and explicitly granted Member Identities can open this restricted Folder."
        : "This restricted Folder needs its Folder Key opened before Member Identities or Links can change it.";
    }
    if (row.access === "restricted") {
      return keyOpen
        ? "This personal restricted Folder is open in this session and stays inside its tighter boundary."
        : "This personal restricted Folder is owner-scoped until you grant or share access.";
    }
    return "Access is Folder-scoped. Keep summaries and logs inside a Folder with the right audience.";
  }

  function accessPeopleHint(row, metadata) {
    if (!row) return "Choose a Folder first.";
    if (row.access === "all_members") {
      return "All Vault members have access; use Add when a late member needs this Folder Key.";
    }
    if (row.access !== "restricted") return "Direct Member Identity grants are only needed for restricted Folders.";
    if (metadata?.kind === "organization") return "Admins can open it; add explicit Member Identities when needed.";
    return "Personal restricted Folders start owner-only; grant one email when sharing is intentional.";
  }

  function folderAllowsDirectGrant(row) {
    return row?.access === "restricted" || row?.access === "all_members";
  }

  function accessFlowHint(row, mode, keyOpen) {
    if (!row) return "Choose a Folder to manage access.";
    if (mode === "people" && !folderAllowsDirectGrant(row)) {
      return "This Folder uses Vault-level access, so there is no direct Member Identity list to edit.";
    }
    if (mode === "links" && row.access !== "restricted") {
      return "Create links from restricted Folders so the link carries a bounded Folder Key Grant.";
    }
    if (!keyOpen) return "Open this Folder key before creating grants or links.";
    if (mode === "people" && row.access === "all_members") {
      return "Grant sends the current Folder Key to an existing Vault member.";
    }
    if (mode === "people") return "Grant adds one email. Remove rotates the Folder Key and re-encrypts readable Pages.";
    if (mode === "links") return "Create a single-use link for a target email, or accept an existing link.";
    return "Choose Member Identities or Links when this Folder needs an access change.";
  }

  function renderAccessSummary(row, metadata, openedFolderKeys) {
    setText("accessAudienceSummary", accessAudienceSummary(row));
    setText("accessKeySummary", accessKeySummary(row, openedFolderKeys));
    setText("accessPeopleSummary", accessPeopleSummary(row, metadata));
    setText("accessPageSummary", accessPagesSummary(row));
  }

  function metadataMountRows(metadata) {
    return (metadata?.mountedFolders || []).map((mount) => ({
      id: mount.mountId,
      label: `${mount.displayName} -> ${mount.sourceVaultId}/${mount.sourceFolderId}`,
      state: mount.state,
    }));
  }

  function bytesToBase64(bytes) {
    let binary = "";
    for (const byte of bytes) binary += String.fromCharCode(byte);
    return btoa(binary);
  }

  function base64ToBytes(value) {
    const binary = atob(value);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return bytes;
  }

  function hexToBytes(value) {
    if (!/^[0-9a-fA-F]+$/.test(value) || value.length % 2 !== 0) {
      throw new Error("hex value is invalid");
    }
    const bytes = new Uint8Array(value.length / 2);
    for (let index = 0; index < bytes.length; index += 1) {
      bytes[index] = Number.parseInt(value.slice(index * 2, index * 2 + 2), 16);
    }
    return bytes;
  }

  function bytesToHex(bytes) {
    return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  }

  function concatBytes(...parts) {
    const length = parts.reduce((sum, part) => sum + part.length, 0);
    const output = new Uint8Array(length);
    let offset = 0;
    for (const part of parts) {
      output.set(part, offset);
      offset += part.length;
    }
    return output;
  }

  function bytesToBigInt(bytes) {
    const hex = bytesToHex(bytes);
    return hex ? BigInt(`0x${hex}`) : 0n;
  }

  function bigIntToBytes(value, length = 32) {
    let hex = value.toString(16);
    if (hex.length > length * 2) throw new Error("integer does not fit target byte length");
    hex = hex.padStart(length * 2, "0");
    return hexToBytes(hex);
  }

  const SECP_P = BigInt("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f");
  const SECP_N = BigInt("0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141");
  const SECP_G = {
    x: BigInt("0x79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"),
    y: BigInt("0x483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8"),
  };

  function mod(value, modulo = SECP_P) {
    const result = value % modulo;
    return result >= 0n ? result : result + modulo;
  }

  function powMod(base, exponent, modulo = SECP_P) {
    let result = 1n;
    let value = mod(base, modulo);
    let power = exponent;
    while (power > 0n) {
      if (power & 1n) result = mod(result * value, modulo);
      value = mod(value * value, modulo);
      power >>= 1n;
    }
    return result;
  }

  function invertMod(value, modulo = SECP_P) {
    let low = mod(value, modulo);
    let high = modulo;
    let lm = 1n;
    let hm = 0n;
    while (low > 1n) {
      const ratio = high / low;
      [lm, hm] = [hm - lm * ratio, lm];
      [low, high] = [high - low * ratio, low];
    }
    return mod(lm, modulo);
  }

  function secpPointAdd(left, right) {
    if (!left) return right;
    if (!right) return left;
    if (left.x === right.x) {
      if (mod(left.y + right.y) === 0n) return null;
      const slope = mod(3n * left.x * left.x * invertMod(2n * left.y));
      const x = mod(slope * slope - 2n * left.x);
      return { x, y: mod(slope * (left.x - x) - left.y) };
    }
    const slope = mod((right.y - left.y) * invertMod(right.x - left.x));
    const x = mod(slope * slope - left.x - right.x);
    return { x, y: mod(slope * (left.x - x) - left.y) };
  }

  function secpPointMultiply(point, scalar) {
    let n = mod(scalar, SECP_N);
    if (!point || n === 0n) return null;
    let result = null;
    let addend = point;
    while (n > 0n) {
      if (n & 1n) result = secpPointAdd(result, addend);
      addend = secpPointAdd(addend, addend);
      n >>= 1n;
    }
    return result;
  }

  function secpLiftX(x) {
    if (x < 0n || x >= SECP_P) throw new Error("secp256k1 x coordinate is out of range");
    const y2 = mod(x * x * x + 7n);
    let y = powMod(y2, (SECP_P + 1n) / 4n);
    if (mod(y * y) !== y2) throw new Error("secp256k1 point is not on curve");
    if (y & 1n) y = SECP_P - y;
    return { x, y };
  }

  function normalizeInviteSecretBytes(secretHex) {
    const secretBytes = hexToBytes(String(secretHex || "").trim());
    if (secretBytes.length !== 32) throw new Error("Invite Secret must be a 32-byte hex key");
    const scalar = bytesToBigInt(secretBytes);
    if (scalar <= 0n || scalar >= SECP_N) throw new Error("Invite Secret is outside secp256k1 range");
    return secretBytes;
  }

  function inviteUnwrapKeypairFromSecret(secretHex) {
    const secretBytes = normalizeInviteSecretBytes(secretHex);
    const scalar = bytesToBigInt(secretBytes);
    const publicPoint = secpPointMultiply(SECP_G, scalar);
    if (!publicPoint) throw new Error("Invite Secret did not produce a public key");
    const publicKeyHex = bytesToHex(bigIntToBytes(publicPoint.x));
    return {
      npub: npubFromHex(publicKeyHex),
      publicKeyHex,
      secretBytes,
      secretHex: bytesToHex(secretBytes),
    };
  }

  function randomInviteSecretBytes() {
    for (let attempt = 0; attempt < 100; attempt += 1) {
      const secretBytes = crypto.getRandomValues(new Uint8Array(32));
      const scalar = bytesToBigInt(secretBytes);
      if (scalar > 0n && scalar < SECP_N) return secretBytes;
    }
    throw new Error("Unable to generate Invite Secret");
  }

  function createInviteUnwrapKeypair() {
    return inviteUnwrapKeypairFromSecret(bytesToHex(randomInviteSecretBytes()));
  }

  async function sha256Bytes(bytes) {
    return new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  }

  async function taggedHash(tag, ...messages) {
    const tagBytes = new TextEncoder().encode(tag);
    const tagHash = await sha256Bytes(tagBytes);
    return sha256Bytes(concatBytes(tagHash, tagHash, ...messages));
  }

  function xorBytes(left, right) {
    if (left.length !== right.length) throw new Error("byte arrays must have equal length");
    const output = new Uint8Array(left.length);
    for (let index = 0; index < left.length; index += 1) output[index] = left[index] ^ right[index];
    return output;
  }

  async function schnorrSign(message32, secretBytes, auxBytes = crypto.getRandomValues(new Uint8Array(32))) {
    if (message32.length !== 32) throw new Error("Schnorr signing requires a 32-byte message hash");
    if (auxBytes.length !== 32) throw new Error("Schnorr aux randomness must be 32 bytes");
    const d0 = bytesToBigInt(secretBytes);
    const point = secpPointMultiply(SECP_G, d0);
    if (!point) throw new Error("Schnorr private key is invalid");
    const d = point.y & 1n ? SECP_N - d0 : d0;
    const publicKey = bigIntToBytes(point.x);
    const t = xorBytes(bigIntToBytes(d), await taggedHash("BIP0340/aux", auxBytes));
    const k0 = bytesToBigInt(await taggedHash("BIP0340/nonce", t, publicKey, message32)) % SECP_N;
    if (k0 === 0n) throw new Error("Schnorr nonce is invalid");
    const noncePoint = secpPointMultiply(SECP_G, k0);
    const k = noncePoint.y & 1n ? SECP_N - k0 : k0;
    const r = bigIntToBytes(noncePoint.x);
    const e = bytesToBigInt(await taggedHash("BIP0340/challenge", r, publicKey, message32)) % SECP_N;
    const s = bigIntToBytes(mod(k + e * d, SECP_N));
    return bytesToHex(concatBytes(r, s));
  }

  async function signEventWithInviteSecret(eventTemplate, inviteSecret, options = {}) {
    const keypair = inviteUnwrapKeypairFromSecret(inviteSecret);
    const event = {
      ...eventTemplate,
      pubkey: keypair.publicKeyHex,
    };
    event.id = await sha256Hex(canonicalNostrEventIdInput(event));
    event.sig = await schnorrSign(
      hexToBytes(event.id),
      keypair.secretBytes,
      options.auxBytes || crypto.getRandomValues(new Uint8Array(32))
    );
    return event;
  }

  async function hmacSha256(keyBytes, messageBytes) {
    const key = await crypto.subtle.importKey(
      "raw",
      keyBytes,
      { name: "HMAC", hash: "SHA-256" },
      false,
      ["sign"]
    );
    return new Uint8Array(await crypto.subtle.sign("HMAC", key, messageBytes));
  }

  async function hkdfExtract(salt, ikm) {
    return hmacSha256(salt, ikm);
  }

  async function hkdfExpand(prk, info, length) {
    const blocks = [];
    let previous = new Uint8Array();
    let outputLength = 0;
    for (let counter = 1; outputLength < length; counter += 1) {
      previous = await hmacSha256(prk, concatBytes(previous, info, Uint8Array.of(counter)));
      blocks.push(previous);
      outputLength += previous.length;
    }
    return concatBytes(...blocks).slice(0, length);
  }

  async function nip44ConversationKey(secretHex, peerHex) {
    const secretBytes = normalizeInviteSecretBytes(secretHex);
    const peerPoint = secpLiftX(bytesToBigInt(hexToBytes(peerHex)));
    const shared = secpPointMultiply(peerPoint, bytesToBigInt(secretBytes));
    if (!shared) throw new Error("NIP-44 shared point is invalid");
    return hkdfExtract(new TextEncoder().encode("nip44-v2"), bigIntToBytes(shared.x));
  }

  async function nip44MessageKeys(conversationKey, nonce) {
    if (conversationKey.length !== 32 || nonce.length !== 32) {
      throw new Error("NIP-44 key derivation requires 32-byte inputs");
    }
    const keys = await hkdfExpand(conversationKey, nonce, 76);
    return {
      chachaKey: keys.slice(0, 32),
      chachaNonce: keys.slice(32, 44),
      hmacKey: keys.slice(44, 76),
    };
  }

  function readU32Le(bytes, offset) {
    return (
      (bytes[offset] |
        (bytes[offset + 1] << 8) |
        (bytes[offset + 2] << 16) |
        (bytes[offset + 3] << 24)) >>>
      0
    );
  }

  function writeU32Le(bytes, offset, value) {
    bytes[offset] = value & 0xff;
    bytes[offset + 1] = (value >>> 8) & 0xff;
    bytes[offset + 2] = (value >>> 16) & 0xff;
    bytes[offset + 3] = (value >>> 24) & 0xff;
  }

  function rotateLeft32(value, bits) {
    return ((value << bits) | (value >>> (32 - bits))) >>> 0;
  }

  function chachaQuarterRound(state, a, b, c, d) {
    state[a] = (state[a] + state[b]) >>> 0;
    state[d] = rotateLeft32(state[d] ^ state[a], 16);
    state[c] = (state[c] + state[d]) >>> 0;
    state[b] = rotateLeft32(state[b] ^ state[c], 12);
    state[a] = (state[a] + state[b]) >>> 0;
    state[d] = rotateLeft32(state[d] ^ state[a], 8);
    state[c] = (state[c] + state[d]) >>> 0;
    state[b] = rotateLeft32(state[b] ^ state[c], 7);
  }

  function chacha20Block(key, nonce, counter) {
    const state = new Uint32Array(16);
    state[0] = 0x61707865;
    state[1] = 0x3320646e;
    state[2] = 0x79622d32;
    state[3] = 0x6b206574;
    for (let index = 0; index < 8; index += 1) state[4 + index] = readU32Le(key, index * 4);
    state[12] = counter >>> 0;
    state[13] = readU32Le(nonce, 0);
    state[14] = readU32Le(nonce, 4);
    state[15] = readU32Le(nonce, 8);
    const working = new Uint32Array(state);
    for (let round = 0; round < 10; round += 1) {
      chachaQuarterRound(working, 0, 4, 8, 12);
      chachaQuarterRound(working, 1, 5, 9, 13);
      chachaQuarterRound(working, 2, 6, 10, 14);
      chachaQuarterRound(working, 3, 7, 11, 15);
      chachaQuarterRound(working, 0, 5, 10, 15);
      chachaQuarterRound(working, 1, 6, 11, 12);
      chachaQuarterRound(working, 2, 7, 8, 13);
      chachaQuarterRound(working, 3, 4, 9, 14);
    }
    const output = new Uint8Array(64);
    for (let index = 0; index < 16; index += 1) {
      writeU32Le(output, index * 4, (working[index] + state[index]) >>> 0);
    }
    return output;
  }

  function chacha20Xor(key, nonce, data) {
    if (key.length !== 32 || nonce.length !== 12) throw new Error("invalid ChaCha20 key or nonce");
    const output = new Uint8Array(data.length);
    let counter = 0;
    for (let offset = 0; offset < data.length; offset += 64) {
      const block = chacha20Block(key, nonce, counter);
      counter += 1;
      for (let index = 0; index < Math.min(64, data.length - offset); index += 1) {
        output[offset + index] = data[offset + index] ^ block[index];
      }
    }
    return output;
  }

  function timingSafeEqual(left, right) {
    if (left.length !== right.length) return false;
    let diff = 0;
    for (let index = 0; index < left.length; index += 1) diff |= left[index] ^ right[index];
    return diff === 0;
  }

  function nip44PaddedLength(length) {
    if (length <= 32) return 32;
    const nextPower = 2 ** (Math.floor(Math.log2(length - 1)) + 1);
    const chunk = nextPower <= 256 ? 32 : nextPower / 8;
    return chunk * (Math.floor((length - 1) / chunk) + 1);
  }

  function nip44Unpad(padded) {
    if (padded.length < 34) throw new Error("NIP-44 padding is invalid");
    const firstTwo = (padded[0] << 8) | padded[1];
    let plaintextLength;
    let prefixLength;
    if (firstTwo === 0) {
      if (padded.length < 6) throw new Error("NIP-44 extended padding is invalid");
      plaintextLength =
        padded[2] * 0x1000000 + ((padded[3] << 16) | (padded[4] << 8) | padded[5]);
      if (plaintextLength < 65536) throw new Error("NIP-44 padding is invalid");
      prefixLength = 6;
    } else {
      plaintextLength = firstTwo;
      prefixLength = 2;
    }
    const paddedLength = nip44PaddedLength(plaintextLength);
    if (!plaintextLength || padded.length !== prefixLength + paddedLength) {
      throw new Error("NIP-44 padding is invalid");
    }
    return new TextDecoder().decode(padded.slice(prefixLength, prefixLength + plaintextLength));
  }

  function nip44Pad(plaintext) {
    const plaintextBytes = new TextEncoder().encode(String(plaintext ?? ""));
    const paddedLength = nip44PaddedLength(plaintextBytes.length);
    let prefixLength;
    if (plaintextBytes.length < 65536) {
      prefixLength = 2;
    } else {
      prefixLength = 6;
    }
    const padded = new Uint8Array(prefixLength + paddedLength);
    if (prefixLength === 2) {
      padded[0] = (plaintextBytes.length >>> 8) & 0xff;
      padded[1] = plaintextBytes.length & 0xff;
    } else {
      padded[2] = (plaintextBytes.length >>> 24) & 0xff;
      padded[3] = (plaintextBytes.length >>> 16) & 0xff;
      padded[4] = (plaintextBytes.length >>> 8) & 0xff;
      padded[5] = plaintextBytes.length & 0xff;
    }
    padded.set(plaintextBytes, prefixLength);
    return padded;
  }

  async function nip44EncryptWithSecret(inviteSecret, recipientHex, plaintext, options = {}) {
    const nonce = options.nonceBytes || crypto.getRandomValues(new Uint8Array(32));
    if (nonce.length !== 32) throw new Error("NIP-44 nonce must be 32 bytes");
    const conversationKey = await nip44ConversationKey(inviteSecret, recipientHex);
    const keys = await nip44MessageKeys(conversationKey, nonce);
    const ciphertext = chacha20Xor(keys.chachaKey, keys.chachaNonce, nip44Pad(plaintext));
    const mac = await hmacSha256(keys.hmacKey, concatBytes(nonce, ciphertext));
    return bytesToBase64(concatBytes(Uint8Array.of(2), nonce, ciphertext, mac));
  }

  async function nip44DecryptWithSecret(inviteSecret, senderHex, payload) {
    const value = String(payload || "");
    if (!value || value[0] === "#" || value.length < 132) throw new Error("NIP-44 payload is invalid");
    const data = base64ToBytes(value);
    if (data.length < 99 || data[0] !== 2) throw new Error("NIP-44 payload version is unsupported");
    const nonce = data.slice(1, 33);
    const ciphertext = data.slice(33, data.length - 32);
    const mac = data.slice(data.length - 32);
    const conversationKey = await nip44ConversationKey(inviteSecret, senderHex);
    const keys = await nip44MessageKeys(conversationKey, nonce);
    const calculatedMac = await hmacSha256(keys.hmacKey, concatBytes(nonce, ciphertext));
    if (!timingSafeEqual(calculatedMac, mac)) throw new Error("NIP-44 payload MAC is invalid");
    return nip44Unpad(chacha20Xor(keys.chachaKey, keys.chachaNonce, ciphertext));
  }

  function inviteSecretDecryptAdapter(inviteSecret) {
    return (senderHex, ciphertext) => nip44DecryptWithSecret(inviteSecret, senderHex, ciphertext);
  }

  function createLocalNip07ProviderFromSecret(secretHex, options = {}) {
    const keypair = inviteUnwrapKeypairFromSecret(secretHex);
    return {
      async getPublicKey() {
        return keypair.publicKeyHex;
      },
      async signEvent(eventTemplate) {
        return signEventWithInviteSecret(eventTemplate, secretHex, options);
      },
      nip44: {
        async encrypt(peerHex, plaintext) {
          return nip44EncryptWithSecret(secretHex, peerHex, plaintext);
        },
        async decrypt(peerHex, ciphertext) {
          return nip44DecryptWithSecret(secretHex, peerHex, ciphertext);
        },
      },
    };
  }

  function createNip07BrainIdentityProvider(provider, options = {}) {
    if (typeof provider?.getPublicKey !== "function" || typeof provider?.signEvent !== "function") {
      throw new Error("NIP-07 compatibility requires getPublicKey and signEvent");
    }
    const brainOrigin = String(options.brainOrigin || window.location?.origin || "").replace(/\/$/, "");
    return Object.freeze({
      version: BRAIN_IDENTITY_PROVIDER_VERSION,
      grantOperationMode: "scoped",
      async identifyMember() {
        const publicKeyHex = await provider.getPublicKey();
        return { publicKeyHex, npub: npubFromHex(publicKeyHex) };
      },
      async authorizeHttpRequest(input) {
        await validateBrainHttpAuthorizationIntent(input, brainOrigin);
        return provider.signEvent.call(provider, input?.eventTemplate);
      },
      async authorizeBrainEvent(input) {
        const publicKeyHex = await provider.getPublicKey();
        validateBrainEventAuthorizationIntent(input, npubFromHex(publicKeyHex));
        return provider.signEvent.call(provider, input?.eventTemplate);
      },
      async openGrantPayload(input) {
        return openNip07FolderKeyGrant(provider, input);
      },
      async wrapGrantPayload(input) {
        return wrapNip07BrainGrant(provider, input, options);
      },
    });
  }

  function scopedBrainGrantRecipient(input, expectedPurpose) {
    if (input?.purpose !== expectedPurpose || !String(input?.vaultId || "").trim()) {
      throw new Error(`Brain ${expectedPurpose} request is invalid`);
    }
    const recipientNpub = String(input?.recipientNpub || "");
    const recipientHex = npubToHex(recipientNpub);
    if (expectedPurpose === "folder-key-grant") {
      if (
        !String(input?.folderId || "").trim() ||
        !Number.isSafeInteger(input?.keyVersion) ||
        input.keyVersion < 1
      ) {
        throw new Error("Brain Folder Key Grant scope is invalid");
      }
    }
    return { recipientHex, recipientNpub };
  }

  function canonicalFolderKeyGrantPayload(payload) {
    return JSON.stringify({
      version: payload.version,
      vaultId: payload.vaultId,
      folderId: payload.folderId,
      keyVersion: payload.keyVersion,
      folderKey: payload.folderKey,
      issuerNpub: payload.issuerNpub,
      recipientNpub: payload.recipientNpub,
      createdAt: payload.createdAt,
    });
  }

  function exactFolderKeyGrantTags(input) {
    return [
      ["d", `finite-folder-key-grant:${input.vaultId}:${input.folderId}:${input.keyVersion}`],
      ["vault", input.vaultId],
      ["folder", input.folderId],
      ["keyVersion", String(input.keyVersion)],
    ];
  }

  async function openNip07FolderKeyGrant(provider, input) {
    const { recipientNpub } = scopedBrainGrantRecipient(input, "folder-key-grant");
    if (typeof input?.wrappedEventJson !== "string" || !input.wrappedEventJson) {
      throw new Error("Brain Folder Key Grant wrapper is required");
    }
    const connectedNpub = npubFromHex(await provider.getPublicKey());
    if (connectedNpub !== recipientNpub) {
      throw new Error("Brain Folder Key Grant recipient is not the connected signer");
    }
    const { rumor } = await openGiftWrappedRumorContent(
      input.wrappedEventJson,
      recipientNpub,
      { provider },
      "Folder Key Grant"
    );
    const plaintext = parseJsonObject(rumor.content, "Folder Key Grant plaintext");
    const issuerNpub = npubFromHex(requireHex64(rumor.pubkey, "Folder Key Grant issuer"));
    if (
      plaintext.version !== "finite-folder-key-grant-v1" ||
      plaintext.vaultId !== input.vaultId ||
      plaintext.folderId !== input.folderId ||
      Number(plaintext.keyVersion) !== input.keyVersion ||
      plaintext.recipientNpub !== recipientNpub ||
      plaintext.issuerNpub !== issuerNpub ||
      canonicalFolderKeyGrantPayload(plaintext) !== rumor.content ||
      JSON.stringify(rumor.tags) !== JSON.stringify(exactFolderKeyGrantTags(input))
    ) {
      throw new Error("Folder Key Grant does not match its requested resource");
    }
    if (base64ToBytes(plaintext.folderKey).length !== 32) {
      throw new Error("Folder Key Grant Folder Key must be 32 bytes");
    }
    return plaintext;
  }

  async function nip07GiftWrapRumor(provider, recipientHex, rumor, createdAtUnix, options = {}) {
    const sealContent = await invokeNip44ProviderMethod(
      provider,
      "encrypt",
      recipientHex,
      JSON.stringify(rumor)
    );
    const seal = await provider.signEvent.call(provider, {
      kind: 13,
      created_at: createdAtUnix,
      tags: [],
      content: sealContent,
    });
    const ephemeralSecret = bytesToHex(randomInviteSecretBytes());
    const wrappedContent = await nip44EncryptWithSecret(
      ephemeralSecret,
      recipientHex,
      JSON.stringify(seal),
      options
    );
    return signEventWithInviteSecret(
      {
        kind: 1059,
        created_at: createdAtUnix,
        tags: [["p", recipientHex]],
        content: wrappedContent,
      },
      ephemeralSecret,
      options
    );
  }

  async function wrapNip07BrainGrant(provider, input, options = {}) {
    if (input?.purpose === "folder-key-grant") {
      const { recipientHex, recipientNpub } = scopedBrainGrantRecipient(
        input,
        "folder-key-grant"
      );
      if (
        !String(input?.id || "").trim() ||
        typeof input?.createdAt !== "string" ||
        !input.createdAt ||
        !Number.isSafeInteger(input?.createdAtUnixSeconds) ||
        input.createdAtUnixSeconds < 0 ||
        typeof input?.folderKey !== "string" ||
        base64ToBytes(input.folderKey).length !== 32
      ) {
        throw new Error("Brain Folder Key Grant payload is invalid");
      }
      const issuerHex = requireHex64(await provider.getPublicKey(), "Folder Key Grant issuer");
      const payload = {
        version: "finite-folder-key-grant-v1",
        vaultId: input.vaultId,
        folderId: input.folderId,
        keyVersion: input.keyVersion,
        folderKey: input.folderKey,
        issuerNpub: npubFromHex(issuerHex),
        recipientNpub,
        createdAt: input.createdAt,
      };
      const rumor = {
        pubkey: issuerHex,
        created_at: input.createdAtUnixSeconds,
        kind: APP_EVENT_KIND,
        tags: exactFolderKeyGrantTags(input),
        content: canonicalFolderKeyGrantPayload(payload),
      };
      rumor.id = await sha256Hex(canonicalNostrEventIdInput(rumor));
      const wrapped = await nip07GiftWrapRumor(
        provider,
        recipientHex,
        rumor,
        input.createdAtUnixSeconds,
        options
      );
      return {
        id: input.id,
        keyVersion: input.keyVersion,
        recipientNpub,
        wrappedEventJson: JSON.stringify(wrapped),
        createdAt: input.createdAt,
      };
    }
    if (input?.purpose === "vault-invite-bootstrap") {
      const { recipientHex } = scopedBrainGrantRecipient(input, "vault-invite-bootstrap");
      if (
        typeof input?.plaintext !== "string" ||
        !input.plaintext ||
        !Number.isSafeInteger(input?.createdAtUnixSeconds) ||
        input.createdAtUnixSeconds < 0
      ) {
        throw new Error("Brain Email Invite Bootstrap payload is invalid");
      }
      const payload = parseJsonObject(input.plaintext, "Email Invite Bootstrap payload");
      if (
        payload.version !== "finite-email-invite-bootstrap-payload-v1" ||
        payload.vaultId !== input.vaultId ||
        payload.inviteUnwrapNpub !== input.recipientNpub ||
        !String(payload.invitedEmail || "").trim() ||
        !Array.isArray(payload.folders) ||
        !Array.isArray(payload.grants) ||
        payload.folders.length !== payload.grants.length ||
        payload.folders.length > MAX_BRAIN_INVITE_BOOTSTRAP_FOLDERS ||
        JSON.stringify(payload) !== input.plaintext
      ) {
        throw new Error("Email Invite Bootstrap does not match its requested resource");
      }
      const grants = new Map();
      for (const entry of payload.grants) {
        if (!entry?.folderId || grants.has(entry.folderId)) {
          throw new Error("Email Invite Bootstrap Folder Key Grant scope is invalid");
        }
        grants.set(entry.folderId, entry.grant);
      }
      const seen = new Set();
      for (const folder of payload.folders) {
        const grant = grants.get(folder?.folderId);
        if (
          !folder?.folderId ||
          seen.has(folder.folderId) ||
          !Number.isSafeInteger(folder.keyVersion) ||
          folder.keyVersion < 1 ||
          !grant?.id ||
          Number(grant.keyVersion) !== folder.keyVersion ||
          grant.recipientNpub !== input.recipientNpub ||
          typeof grant.wrappedEventJson !== "string" ||
          !grant.wrappedEventJson
        ) {
          throw new Error("Email Invite Bootstrap Folder Key Grant metadata is invalid");
        }
        validateGiftWrapShell(
          parseJsonObject(
            grant.wrappedEventJson,
            "Email Invite Bootstrap Folder Key Grant wrapper"
          ),
          recipientHex
        );
        seen.add(folder.folderId);
      }
      const issuerHex = requireHex64(await provider.getPublicKey(), "Email Invite issuer");
      const rumor = {
        pubkey: issuerHex,
        created_at: input.createdAtUnixSeconds,
        kind: APP_EVENT_KIND,
        tags: [
          ["d", `finite-email-invite-bootstrap:${input.vaultId}`],
          ["vault", input.vaultId],
        ],
        content: input.plaintext,
      };
      rumor.id = await sha256Hex(canonicalNostrEventIdInput(rumor));
      return nip07GiftWrapRumor(
        provider,
        recipientHex,
        rumor,
        input.createdAtUnixSeconds,
        options
      );
    }
    throw new Error("unsupported Brain grant purpose");
  }

  function createHostedBrainIdentityProvider(options = {}) {
    const endpoint = String(options.endpoint || "/api/brain/identity-provider");
    const fetchImpl = options.fetch || fetch;
    const clientCapability = String(
      options.clientCapability ||
        (typeof document !== "undefined"
          ? document
              .querySelector('meta[name="finite-brain-client-capability"]')
              ?.getAttribute("content")
          : "") ||
        ""
    );
    const providerState = {
      status: "checking",
      detail: "Checking the hosted Brain identity.",
    };
    const parentOrigin = String(
      options.parentOrigin ||
        (typeof document !== "undefined"
          ? document.querySelector('meta[name="finite-brain-parent-origin"]')?.getAttribute("content")
          : "") ||
        ""
    ).replace(/\/$/, "");
    const trustedParentOrigin = (() => {
      try {
        const parsed = new URL(parentOrigin);
        return parsed.origin === parentOrigin ? parentOrigin : "";
      } catch (_) {
        return "";
      }
    })();
    const requestSessionProof = async (requestHash) => {
      if (typeof options.sessionProofProvider === "function") {
        return options.sessionProofProvider(requestHash);
      }
      if (typeof options.sessionProof === "string" && options.sessionProof) {
        return options.sessionProof;
      }
      if (!trustedParentOrigin || !window.parent || window.parent === window) {
        throw new Error("Open Brain from the signed-in dashboard.");
      }
      const requestId = bytesToHex(crypto.getRandomValues(new Uint8Array(16)));
      return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
          window.removeEventListener("message", handleProof);
          reject(new Error("Your dashboard session could not be verified."));
        }, 5000);
        function handleProof(event) {
          if (
            event.source !== window.parent ||
            event.origin !== trustedParentOrigin ||
            event.data?.type !== BRAIN_SESSION_PROOF_RESPONSE ||
            event.data?.requestId !== requestId
          ) {
            return;
          }
          clearTimeout(timeout);
          window.removeEventListener("message", handleProof);
          if (typeof event.data.proof === "string" && event.data.proof) {
            resolve(event.data.proof);
          } else {
            reject(new Error("Your dashboard session expired. Sign in and open Brain again."));
          }
        }
        window.addEventListener("message", handleProof);
        window.parent.postMessage(
          { type: BRAIN_SESSION_PROOF_REQUEST, requestId, requestHash },
          trustedParentOrigin
        );
      });
    };
    const providerRequest = async (operation, input = null) => {
      const requestBody = JSON.stringify({
        version: BRAIN_IDENTITY_PROVIDER_VERSION,
        operation,
        input,
      });
      let sessionProof;
      try {
        sessionProof = await requestSessionProof(await sha256Hex(requestBody));
      } catch (error) {
        providerState.status = "setup_required";
        providerState.detail = "Your hosted Brain session expired. Sign in and open Brain again.";
        if (state.identityProvider === provider) expireBrainIdentitySession();
        throw error;
      }
      const response = await fetchImpl(endpoint, {
        method: "POST",
        credentials: "omit",
        cache: "no-store",
        headers: {
          "content-type": "application/json",
          ...(clientCapability
            ? { "x-finite-brain-client-capability": clientCapability }
            : {}),
          "x-finite-brain-session-proof": sessionProof,
          "x-finite-brain-provider-version": BRAIN_IDENTITY_PROVIDER_VERSION,
        },
        body: requestBody,
      });
      const body = await response.json().catch(() => ({}));
      if (!response.ok) {
        if (response.status === 428) {
          providerState.status = "setup_required";
          providerState.detail = "Set up your Finite Chat Hosted Device before opening Brain.";
        } else if (response.status === 401 || response.status === 403) {
          providerState.status = "setup_required";
          providerState.detail = "Your hosted Brain session expired. Sign in and open Brain again.";
          if (state.identityProvider === provider) expireBrainIdentitySession();
        }
        throw new Error(body?.error || `Hosted Brain identity request failed with ${response.status}`);
      }
      providerState.status = "ready";
      providerState.detail = "Hosted Brain identity is ready.";
      return body;
    };
    const provider = Object.freeze({
      version: BRAIN_IDENTITY_PROVIDER_VERSION,
      grantOperationMode: "scoped",
      identifyMember() {
        return providerRequest("identifyMember");
      },
      authorizeHttpRequest(input) {
        return providerRequest("authorizeHttpRequest", input);
      },
      authorizeBrainEvent(input) {
        return providerRequest("authorizeBrainEvent", input);
      },
      async openGrantPayload(input) {
        const result = await providerRequest("openGrantPayload", input);
        return result.plaintext;
      },
      async wrapGrantPayload(input) {
        const result = await providerRequest("wrapGrantPayload", input);
        if (input?.purpose === "folder-key-grant") return result.grant;
        if (input?.purpose === "vault-invite-bootstrap") return result.wrappedEventJson;
        throw new Error("Unsupported hosted Brain grant purpose");
      },
    });
    if (trustedParentOrigin && window.parent && window.parent !== window) {
      window.addEventListener("message", (event) => {
        if (
          event.source === window.parent &&
          event.origin === trustedParentOrigin &&
          event.data?.type === BRAIN_SESSION_ENDED
        ) {
          providerState.status = "setup_required";
          providerState.detail = "Your hosted Brain session expired. Sign in and open Brain again.";
          if (state.identityProvider === provider) expireBrainIdentitySession();
        }
      });
    }
    hostedIdentityProviderStates.set(provider, providerState);
    return provider;
  }

  function eventTagValues(eventTemplate, name) {
    if (!Array.isArray(eventTemplate?.tags)) return [];
    return eventTemplate.tags
      .filter((tag) => Array.isArray(tag) && tag[0] === name && typeof tag[1] === "string")
      .map((tag) => tag[1]);
  }

  function requireSingleEventTag(eventTemplate, name, label) {
    const values = eventTagValues(eventTemplate, name);
    if (values.length !== 1) throw new Error(`${label} requires exactly one ${name} tag`);
    return values[0];
  }

  function absoluteUrlPath(url) {
    const value = String(url || "");
    const schemeEnd = value.indexOf("://");
    if (schemeEnd <= 0) return null;
    const pathStart = value.indexOf("/", schemeEnd + 3);
    const pathAndSuffix = pathStart < 0 ? "/" : value.slice(pathStart);
    return pathAndSuffix.split(/[?#]/, 1)[0];
  }

  function absoluteUrlOrigin(url) {
    const value = String(url || "");
    const schemeEnd = value.indexOf("://");
    if (schemeEnd <= 0) return null;
    const pathStart = value.indexOf("/", schemeEnd + 3);
    return (pathStart < 0 ? value : value.slice(0, pathStart)).replace(/\/$/, "");
  }

  async function validateBrainHttpAuthorizationIntent(input, brainOrigin) {
    const eventTemplate = input?.eventTemplate;
    if (eventTemplate?.kind !== 27235) {
      throw new Error("Brain HTTP authorization requires kind 27235");
    }
    const method = String(input?.method || "").toUpperCase();
    if (!["DELETE", "GET", "PATCH", "POST", "PUT"].includes(method)) {
      throw new Error("Brain HTTP authorization method is unsupported");
    }
    const url = String(input?.url || "");
    if (!brainOrigin || absoluteUrlOrigin(url) !== brainOrigin) {
      throw new Error("Brain HTTP authorization requires the official Brain origin");
    }
    const path = absoluteUrlPath(url);
    if (path !== "/_admin" && !path?.startsWith("/_admin/")) {
      throw new Error("Brain HTTP authorization requires a protected Brain route");
    }
    if (requireSingleEventTag(eventTemplate, "u", "Brain HTTP authorization") !== url) {
      throw new Error("Brain HTTP authorization URL tag does not match its request");
    }
    if (requireSingleEventTag(eventTemplate, "method", "Brain HTTP authorization") !== method) {
      throw new Error("Brain HTTP authorization method tag does not match its request");
    }
    const nonce = requireSingleEventTag(eventTemplate, "nonce", "Brain HTTP authorization");
    if (!/^[0-9a-f]{32}$/i.test(nonce)) {
      throw new Error("Brain HTTP authorization nonce tag is invalid");
    }
    const bodyText = String(input?.bodyText || "");
    const payloadTags = eventTagValues(eventTemplate, "payload");
    if (bodyText) {
      if (payloadTags.length !== 1 || payloadTags[0] !== await sha256Hex(bodyText)) {
        throw new Error("Brain HTTP authorization payload tag does not match its request body");
      }
    } else if (payloadTags.length !== 0) {
      throw new Error("Brain HTTP authorization without a body cannot include a payload tag");
    }
    if (eventTemplate.content !== "") {
      throw new Error("Brain HTTP authorization content must be empty");
    }
    const allowedTags = new Set(["method", "nonce", "payload", "u"]);
    if (eventTemplate.tags.some((tag) => !Array.isArray(tag) || !allowedTags.has(tag[0]))) {
      throw new Error("Brain HTTP authorization contains an unsupported tag");
    }
  }

  function requireExactBrainEventTags(eventTemplate, expected, intent) {
    if (JSON.stringify(eventTemplate.tags) !== JSON.stringify(expected)) {
      throw new Error(`${intent} tags differ from its typed payload`);
    }
  }

  function validateBrainEventAuthorizationIntent(input, signerNpub = null) {
    const intent = input?.intent;
    const eventTemplate = input?.eventTemplate;
    const expectedKind = BRAIN_EVENT_KIND_BY_INTENT[intent];
    if (!expectedKind) throw new Error("unsupported Brain identity intent");
    if (eventTemplate?.kind !== expectedKind) {
      throw new Error(`${intent} requires Nostr kind ${expectedKind}`);
    }
    if (typeof eventTemplate.content !== "string" || !eventTemplate.content) {
      throw new Error(`${intent} requires event content`);
    }

    const expectedDPrefix = BRAIN_EVENT_D_PREFIX_BY_INTENT[intent];
    if (expectedDPrefix) {
      let payload;
      try {
        payload = JSON.parse(eventTemplate.content);
      } catch (_) {
        throw new Error(`${intent} payload is not JSON`);
      }
      if (intent === "folder-object-revision") {
        if (
          payload.version !== REVISION_VERSION ||
          payload.cipher !== CIPHER ||
          payload.revision < 1 ||
          payload.keyVersion < 1 ||
          !/^[0-9a-f]{64}$/i.test(String(payload.ciphertextHash || "")) ||
          (signerNpub && payload.authorNpub !== signerNpub) ||
          canonicalRevisionPayload(payload) !== eventTemplate.content
        ) {
          throw new Error("Brain revision payload is invalid");
        }
        requireExactBrainEventTags(eventTemplate, revisionTags(payload), intent);
        return;
      }
      if (intent === "folder-object-tombstone") {
        if (
          payload.version !== TOMBSTONE_VERSION ||
          payload.operation !== "delete" ||
          payload.revision < 1 ||
          (signerNpub && payload.authorNpub !== signerNpub) ||
          canonicalTombstonePayload(payload) !== eventTemplate.content
        ) {
          throw new Error("Brain tombstone payload is invalid");
        }
        requireExactBrainEventTags(eventTemplate, tombstoneTags(payload), intent);
        return;
      }
      if (intent === "vault-access-change") {
        if (
          payload.version !== "finite-vault-admin-access-change-v1" ||
          (signerNpub && payload.adminNpub !== signerNpub) ||
          canonicalAdminAccessChangePayload(payload) !== eventTemplate.content
        ) {
          throw new Error("Brain access-change payload is invalid");
        }
        requireExactBrainEventTags(eventTemplate, adminAccessChangeTags(payload), intent);
        return;
      }
      if (intent === "vault-invite-authorization") {
        const canonical = JSON.stringify({
          version: payload.version,
          vaultId: payload.vaultId,
          invitedEmail: payload.invitedEmail,
          inviteUnwrapNpub: payload.inviteUnwrapNpub,
          bootstrapPayloadHash: payload.bootstrapPayloadHash,
          expiresAt: payload.expiresAt,
          folders: payload.folders,
        });
        if (
          payload.version !== "finite-email-invite-bootstrap-authorization-v1" ||
          !payload.invitedEmail ||
          !Array.isArray(payload.folders) ||
          payload.folders.length > MAX_BRAIN_INVITE_BOOTSTRAP_FOLDERS ||
          payload.folders.some(
            (folder) =>
              !String(folder?.folderId || "").trim() ||
              !["all_members", "restricted"].includes(folder?.access) ||
              !Number.isSafeInteger(folder?.keyVersion) ||
              folder.keyVersion < 1
          ) ||
          new Set(payload.folders.map((folder) => folder.folderId)).size !==
            payload.folders.length ||
          !/^sha256:[0-9a-f]{64}$/i.test(String(payload.bootstrapPayloadHash || "")) ||
          canonical !== eventTemplate.content
        ) {
          throw new Error("Brain email-invite authorization payload is invalid");
        }
        npubToHex(payload.inviteUnwrapNpub);
        requireExactBrainEventTags(eventTemplate, emailInviteAuthorizationTags({
          vaultId: payload.vaultId,
          invitedEmail: payload.invitedEmail,
        }), intent);
        return;
      }
      throw new Error(`Brain event does not match ${intent}`);
    }

  }

  function convertBits(data, fromBits, toBits, pad) {
    let accumulator = 0;
    let bits = 0;
    const result = [];
    const maxValue = (1 << toBits) - 1;
    for (const value of data) {
      if (value < 0 || value >> fromBits !== 0) throw new Error("invalid bech32 source value");
      accumulator = (accumulator << fromBits) | value;
      bits += fromBits;
      while (bits >= toBits) {
        bits -= toBits;
        result.push((accumulator >> bits) & maxValue);
      }
    }
    if (pad && bits > 0) {
      result.push((accumulator << (toBits - bits)) & maxValue);
    } else if (bits >= fromBits || ((accumulator << (toBits - bits)) & maxValue) !== 0) {
      throw new Error("invalid bech32 padding");
    }
    return result;
  }

  function bech32Polymod(values) {
    const generators = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let checksum = 1;
    for (const value of values) {
      const top = checksum >> 25;
      checksum = ((checksum & 0x1ffffff) << 5) ^ value;
      for (let index = 0; index < 5; index += 1) {
        if ((top >> index) & 1) checksum ^= generators[index];
      }
    }
    return checksum;
  }

  function bech32HrpExpand(hrp) {
    const result = [];
    for (let index = 0; index < hrp.length; index += 1) {
      result.push(hrp.charCodeAt(index) >> 5);
    }
    result.push(0);
    for (let index = 0; index < hrp.length; index += 1) {
      result.push(hrp.charCodeAt(index) & 31);
    }
    return result;
  }

  function bech32Encode(hrp, data) {
    const values = [...bech32HrpExpand(hrp), ...data, 0, 0, 0, 0, 0, 0];
    const polymod = bech32Polymod(values) ^ 1;
    const checksum = [];
    for (let index = 0; index < 6; index += 1) {
      checksum.push((polymod >> (5 * (5 - index))) & 31);
    }
    return `${hrp}1${[...data, ...checksum].map((value) => BECH32_CHARSET[value]).join("")}`;
  }

  function bech32Decode(value) {
    const source = String(value || "").trim();
    if (!source) throw new Error("bech32 value is empty");
    if (source !== source.toLowerCase() && source !== source.toUpperCase()) {
      throw new Error("bech32 value mixes upper and lower case");
    }
    const normalized = source.toLowerCase();
    const separator = normalized.lastIndexOf("1");
    if (separator < 1 || separator + 7 > normalized.length) {
      throw new Error("bech32 value is malformed");
    }
    const hrp = normalized.slice(0, separator);
    const data = normalized
      .slice(separator + 1)
      .split("")
      .map((char) => {
        const index = BECH32_CHARSET.indexOf(char);
        if (index === -1) throw new Error("bech32 value has an invalid character");
        return index;
      });
    if (bech32Polymod([...bech32HrpExpand(hrp), ...data]) !== 1) {
      throw new Error("bech32 checksum is invalid");
    }
    return { hrp, data: data.slice(0, -6) };
  }

  function npubFromHex(pubkeyHex) {
    return bech32Encode("npub", convertBits(hexToBytes(pubkeyHex), 8, 5, true));
  }

  function npubToHex(npub) {
    const decoded = bech32Decode(npub);
    if (decoded.hrp !== "npub") throw new Error("expected an npub");
    const bytes = Uint8Array.from(convertBits(decoded.data, 5, 8, false));
    if (bytes.length !== 32) throw new Error("npub must contain a 32-byte public key");
    return bytesToHex(bytes);
  }

  function createClientProjection() {
    return {
      pages: new Map(),
      seenEventIds: new Set(),
      localDrafts: new Map(),
      conflicts: [],
    };
  }

  function clearSessionSecretsAndPlaintext(target) {
    clearSessionKeyring(target.keyring);
    target.projection?.pages?.clear?.();
    target.projection?.seenEventIds?.clear?.();
    target.projection?.localDrafts?.clear?.();
    if (Array.isArray(target.projection?.conflicts)) target.projection.conflicts.length = 0;
    target.identityByNpub?.clear?.();

    target.sessionStatus = SESSION_STATUS.LOCKED;
    target.sessionNotice = null;
    target.visibleVaults = [];
    target.metadata = null;
    target.keyring = null;
    target.projection = createClientProjection();
    target.preparedWrite = null;
    target.preparedWriteTarget = null;
    target.pageSaveInFlight = null;
    if (target === state) {
      clearClientActionFeedback({ render: false });
    } else {
      target.clientActionFeedbackGeneration = (target.clientActionFeedbackGeneration || 0) + 1;
      target.clientActionFeedback = null;
    }
    target.lastError = null;
    target.accessResult = null;
    target.lastShareLinkId = null;
    target.lastVaultInvitationCode = null;
    target.lastVaultInvitationId = null;
    target.lastEmailInviteSecret = null;
    target.lastEmailInviteUrl = null;
    target.lastEmailInvitePostProof = null;
    target.vaultInvitations = null;
    target.folderShareLinks = null;
    target.folderShareLinksFolderId = null;
    target.sharedFolderInvitations = null;
    target.sharedFolderConnections = null;
    target.agentWorkspacePairings = null;
    target.selectedFolderId = null;
    target.selectedPageKey = null;
    target.graphZoom = 1;
    target.searchHighlight = null;
    target.searchHighlightShouldScroll = false;
    target.activeWorkspaceView = "page";
    target.activeSidebarMode = "files";
    target.activeAccessFolderId = null;
    target.activeAccessIntent = "overview";
    target.editorMode = "visual";
    target.expandedFolderIds = new Set();
    target.contextMenuTarget = null;
    target.contextMenuPreviousFocus = null;
    target.commandPaletteOpen = false;
    target.commandPaletteSelectedIndex = 0;
    target.editorSlashOpen = false;
    target.editorSlashQuery = "";
    target.editorSlashRange = null;
    target.accessFolderDropdownOpen = false;
    target.accessFolderFocusedIndex = 0;
    target.readerBusy = false;
    target.accessBusy = false;
    target.manageVaultsReturnToSettings = null;
    return target;
  }

  function sessionStatusView(status) {
    if (status === SESSION_STATUS.UNLOCKED) {
      return {
        action: "Lock session",
        detail: "Readable content and Session Folder Keys are held in memory for this session.",
        locked: false,
        title: "Session unlocked",
      };
    }
    if (status === SESSION_STATUS.RESUMING) {
      return {
        action: "Lock session",
        detail: "Opening encrypted Folder Key Grants and rebuilding the temporary client view.",
        locked: false,
        title: "Unlocking session",
      };
    }
    return {
      action: "Unlock session",
      detail: "Folder Keys and temporary plaintext are cleared. Unlock to reopen encrypted grants.",
      locked: true,
      title: "Session locked",
    };
  }

  function sessionIdentityLabel() {
    if (state.signerStatus === "connected" && state.pubkeyHex) {
      return shortKey(npubFromHex(state.pubkeyHex));
    }
    if (state.signerStatus === "ready") return "Brain identity ready";
    if (state.signerStatus === "checking") return "Checking Brain identity";
    if (state.signerStatus === "setup_required") return "Brain setup required";
    return "Brain identity unavailable";
  }

  const SETTINGS_SECTIONS = Object.freeze(["session", "vault", "access", "invitations"]);

  function settingsSectionsForSession(sessionStatus = state.sessionStatus) {
    return sessionStatus === SESSION_STATUS.UNLOCKED ? SETTINGS_SECTIONS : ["session"];
  }

  function normalizeSettingsSection(section, sessionStatus = state.sessionStatus) {
    return settingsSectionsForSession(sessionStatus).includes(section) ? section : "session";
  }

  function settingsSignerView() {
    const provider = deriveBrainIdentityProviderState(state.identityProvider);
    if (state.signerStatus === "connected" && state.pubkeyHex) {
      return {
        canConnect: false,
        detail: `Connected as ${sessionIdentityLabel()}. Signed Vault requests use this Member Identity.`,
        title: "Brain identity connected",
      };
    }
    if (state.signerStatus === "checking") {
      return {
        canConnect: false,
        detail: "Checking the Brain Identity Provider supplied by Finite Chat.",
        title: "Checking Brain identity",
      };
    }
    return {
      canConnect: provider.canConnect,
      detail: provider.detail,
      title: provider.canConnect ? "Brain identity ready" : provider.status === "setup_required" ? "Brain setup required" : "Brain identity unavailable",
    };
  }

  function isSequentiallyFocusable(element) {
    const tabIndex = Number(element?.tabIndex);
    if (Number.isFinite(tabIndex)) return tabIndex >= 0;
    const tabIndexAttribute = element?.getAttribute?.("tabindex");
    return tabIndexAttribute === null || tabIndexAttribute === undefined || Number(tabIndexAttribute) >= 0;
  }

  function isVisibleSequentiallyFocusable(element) {
    return !element.hidden && !element.closest?.("[hidden]") && isSequentiallyFocusable(element);
  }

  function settingsModalFocusableElements() {
    const modal = $("settingsModal");
    if (!modal) return [];
    return Array.from(
      modal.querySelectorAll(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
      )
    ).filter(isVisibleSequentiallyFocusable);
  }

  function mountAccessPanelInSettings() {
    const panel = $("accessSidebarPanel");
    const mount = $("settingsAccessPanelMount");
    if (!panel || !mount || panel.parentElement === mount) return;
    mount.appendChild(panel);
    panel.classList.remove("sidebar-mode-panel", "sidebar-tool-panel");
    panel.hidden = false;
    panel.removeAttribute("aria-hidden");
  }

  function mountInvitationPanelInSettings() {
    const mount = $("settingsInvitationsPanelMount");
    if (!mount) return;
    const invitationNodes = [
      $("vaultInvitationActionSection"),
      $("vaultInvitationPanel"),
      $("vaultInvitationListSection"),
      $("sharedFolderSection"),
    ].filter(Boolean);
    for (const node of invitationNodes) {
      if (node.parentElement !== mount) mount.appendChild(node);
    }
  }

  function focusSettingsSection(section = state.settingsSection) {
    const sessionOnly = settingsSectionsForSession().length === 1;
    const resumeButton = $("resumeSessionButton");
    const navButton = sessionOnly
      ? null
      : $(
          section === "vault"
            ? "settingsNavVault"
            : section === "access"
              ? "settingsNavAccess"
              : section === "invitations"
                ? "settingsNavInvitations"
                : "settingsNavSession"
        );
    const focusTarget = sessionOnly
      ? (resumeButton && !resumeButton.disabled ? resumeButton : $("closeSettingsButton"))
      : navButton;
    if (typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => focusTarget?.focus?.());
    } else {
      focusTarget?.focus?.();
    }
  }

  function openSettingsModal(section = state.settingsSection) {
    const modal = $("settingsModal");
    if (!modal) return;
    if (state.vaultSwitcherOpen) closeVaultSwitcher({ restoreFocus: false });
    if (state.manageVaultsModalOpen) closeManageVaultsModal();
    if (!state.settingsModalOpen) {
      state.settingsModalPreviousFocus = document.activeElement || null;
    }
    state.settingsSection = normalizeSettingsSection(section);
    state.settingsModalOpen = true;
    closeContextMenu();
    closeCommandPalette();
    closeEditorSlashMenu();
    render();
    focusSettingsSection(state.settingsSection);
  }

  function closeSettingsModal(options = {}) {
    if (!state.settingsModalOpen) return;
    state.settingsModalOpen = false;
    closeAccessFolderDropdown();
    const previousFocus = state.settingsModalPreviousFocus;
    state.settingsModalPreviousFocus = null;
    render();
    if (options.restoreFocus !== false) previousFocus?.focus?.();
  }

  function overlayFocusableElements(id) {
    const overlay = $(id);
    if (!overlay) return [];
    return Array.from(
      overlay.querySelectorAll?.(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
      ) || []
    ).filter(isVisibleSequentiallyFocusable);
  }

  function vaultSwitcherFocusableElements() {
    return overlayFocusableElements("vaultSwitcherMenu");
  }

  function documentFocusableElements(excludedContainer = null) {
    return Array.from(
      document.querySelectorAll?.(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
      ) || []
    ).filter(
      (element) =>
        isVisibleSequentiallyFocusable(element) &&
        !excludedContainer?.contains?.(element)
    );
  }

  function moveVaultSwitcherFocusOut(options = {}) {
    const menu = $("vaultSwitcherMenu");
    const trigger = $("sessionAccountVaultButton");
    const focusable = documentFocusableElements(menu);
    const triggerIndex = focusable.indexOf(trigger);
    const direction = options.backwards ? -1 : 1;
    const nextTarget = triggerIndex >= 0 ? focusable[triggerIndex + direction] : null;
    closeVaultSwitcher({ restoreFocus: false });
    nextTarget?.focus?.();
  }

  function focusVaultSwitcherItem(index = 0) {
    const items = vaultSwitcherFocusableElements();
    if (!items.length) return;
    const nextIndex = Math.min(Math.max(index, 0), items.length - 1);
    if (typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => items[nextIndex]?.focus?.());
    } else {
      items[nextIndex]?.focus?.();
    }
  }

  function openVaultSwitcher() {
    if (state.vaultSwitcherOpen) {
      closeVaultSwitcher();
      return;
    }
    if (state.settingsModalOpen) closeSettingsModal();
    if (state.manageVaultsModalOpen) closeManageVaultsModal();
    state.vaultSwitcherPreviousFocus = document.activeElement || null;
    state.vaultSwitcherOpen = true;
    closeContextMenu();
    closeCommandPalette();
    closeEditorSlashMenu();
    render();
    focusVaultSwitcherItem(0);
  }

  function closeVaultSwitcher(options = {}) {
    if (!state.vaultSwitcherOpen) return;
    state.vaultSwitcherOpen = false;
    const previousFocus = state.vaultSwitcherPreviousFocus;
    state.vaultSwitcherPreviousFocus = null;
    render();
    if (options.restoreFocus !== false) previousFocus?.focus?.();
  }

  function manageVaultsModalFocusableElements() {
    return overlayFocusableElements("manageVaultsModal");
  }

  function focusManageVaultsReturnTarget() {
    if (state.settingsSection !== "vault") {
      focusSettingsSection(state.settingsSection);
      return;
    }
    const target = $("settingsManageVaultsButton");
    if (typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => target?.focus?.());
    } else {
      target?.focus?.();
    }
  }

  function openManageVaultsModal(options = {}) {
    if (state.manageVaultsModalOpen) return;
    const menuFocus = state.vaultSwitcherPreviousFocus;
    const returnToSettings = Boolean(options.returnToSettings && state.settingsModalOpen);
    state.manageVaultsReturnToSettings = returnToSettings
      ? {
          previousFocus: state.settingsModalPreviousFocus,
          section: "vault",
        }
      : null;
    state.manageVaultsModalPreviousFocus = returnToSettings
      ? null
      : menuFocus || document.activeElement || null;
    closeVaultSwitcher({ restoreFocus: false });
    if (state.settingsModalOpen) {
      closeSettingsModal({ restoreFocus: false });
    }
    state.manageVaultsModalOpen = true;
    closeContextMenu();
    closeCommandPalette();
    closeEditorSlashMenu();
    render();
    const focusTarget = $("closeManageVaultsButton");
    if (typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => focusTarget?.focus?.());
    } else {
      focusTarget?.focus?.();
    }
  }

  function closeManageVaultsModal() {
    if (!state.manageVaultsModalOpen) return;
    state.manageVaultsModalOpen = false;
    const returnToSettings = state.manageVaultsReturnToSettings;
    state.manageVaultsReturnToSettings = null;
    const previousFocus = state.manageVaultsModalPreviousFocus;
    state.manageVaultsModalPreviousFocus = null;
    if (returnToSettings) {
      state.settingsSection = returnToSettings.section;
      state.settingsModalPreviousFocus = returnToSettings.previousFocus;
      state.settingsModalOpen = true;
      render();
      focusManageVaultsReturnTarget();
      return;
    }
    render();
    previousFocus?.focus?.();
  }

  function manageVaultsLoadAction() {
    const operation = state.sessionStatus === SESSION_STATUS.LOCKED
      ? resumeSession()
      : loadVaultReader();
    operation.catch((error) => {
      reportClientActionFailure(error);
      log("Failed to load Vault from Manage Vaults.", { error: error.message });
      state.readerBusy = false;
      render();
    });
  }

  function setSettingsSection(section) {
    state.settingsSection = normalizeSettingsSection(section);
    render();
    focusSettingsSection(state.settingsSection);
  }

  function renderSettingsModal() {
    const modal = $("settingsModal");
    if (!modal) return;
    modal.hidden = !state.settingsModalOpen;
    modal.setAttribute("aria-hidden", String(!state.settingsModalOpen));
    const shell = document.querySelector?.(".obsidian-shell");
    if (shell) shell.dataset.settingsOpen = state.settingsModalOpen ? "true" : "false";
    const availableSections = settingsSectionsForSession(state.sessionStatus);
    const sessionOnly = availableSections.length === 1;
    const forcedSessionSection = !availableSections.includes(state.settingsSection);
    state.settingsSection = normalizeSettingsSection(state.settingsSection);
    const settingsNav = $("settingsNav");
    if (settingsNav) {
      settingsNav.hidden = sessionOnly;
      settingsNav.setAttribute("aria-hidden", String(sessionOnly));
    }
    $("settingsModalLayout")?.classList?.toggle("settings-session-only", sessionOnly);
    const sessionNav = $("settingsNavSession");
    const vaultNav = $("settingsNavVault");
    const accessNav = $("settingsNavAccess");
    const invitationsNav = $("settingsNavInvitations");
    const sessionPanel = $("settingsSessionPanel");
    const vaultPanel = $("settingsVaultPanel");
    const accessPanel = $("settingsAccessPanel");
    const invitationsPanel = $("settingsInvitationsPanel");
    const sessionActive = state.settingsSection === "session";
    const vaultActive = state.settingsSection === "vault";
    const accessActive = state.settingsSection === "access";
    const invitationsActive = state.settingsSection === "invitations";
    if (sessionNav) {
      sessionNav.hidden = false;
      sessionNav.className = `settings-nav-item${sessionActive ? " active" : ""}`;
      sessionNav.setAttribute("aria-selected", String(sessionActive));
      sessionNav.tabIndex = sessionActive ? 0 : -1;
    }
    if (vaultNav) {
      vaultNav.hidden = sessionOnly;
      vaultNav.className = `settings-nav-item${vaultActive ? " active" : ""}`;
      vaultNav.setAttribute("aria-selected", String(vaultActive));
      vaultNav.tabIndex = vaultActive ? 0 : -1;
    }
    if (accessNav) {
      accessNav.hidden = sessionOnly;
      accessNav.className = `settings-nav-item${accessActive ? " active" : ""}`;
      accessNav.setAttribute("aria-selected", String(accessActive));
      accessNav.tabIndex = accessActive ? 0 : -1;
    }
    if (invitationsNav) {
      invitationsNav.hidden = sessionOnly;
      invitationsNav.className = `settings-nav-item${invitationsActive ? " active" : ""}`;
      invitationsNav.setAttribute("aria-selected", String(invitationsActive));
      invitationsNav.tabIndex = invitationsActive ? 0 : -1;
    }
    if (sessionPanel) {
      sessionPanel.hidden = !sessionActive;
      sessionPanel.setAttribute("aria-hidden", String(!sessionActive));
    }
    if (vaultPanel) {
      vaultPanel.hidden = !vaultActive;
      vaultPanel.setAttribute("aria-hidden", String(!vaultActive));
    }
    if (accessPanel) {
      accessPanel.hidden = !accessActive;
      accessPanel.setAttribute("aria-hidden", String(!accessActive));
    }
    if (invitationsPanel) {
      invitationsPanel.hidden = !invitationsActive;
      invitationsPanel.setAttribute("aria-hidden", String(!invitationsActive));
    }
    setText("settingsVaultName", activeVaultLabel());
    setText("settingsVaultIdentity", sessionIdentityLabel());
    setText("settingsVaultStatus", sessionStatusView(state.sessionStatus).title);
    const signer = settingsSignerView();
    setText("settingsSignerTitle", signer.title);
    setText("settingsSignerDetail", signer.detail);
    safeSetHidden("settingsConnectSignerButton", !signer.canConnect);
    setOptionalDisabled(
      "settingsConnectSignerButton",
      !signer.canConnect
    );
    if (state.settingsModalOpen && forcedSessionSection) focusSettingsSection("session");
  }

  function renderVaultSwitcher() {
    const menu = $("vaultSwitcherMenu");
    const trigger = $("sessionAccountVaultButton");
    if (!menu || !trigger) return;
    menu.hidden = !state.vaultSwitcherOpen;
    trigger.setAttribute("aria-expanded", String(state.vaultSwitcherOpen));
    setText("vaultSwitcherCount", `${visibleVaultOptions().length}`);
    const rows = visibleVaultOptions();
    const emptyText = state.signerStatus === "connected"
      ? "No Vaults available."
      : "Connect a signer to list Vaults.";
    setList("vaultSwitcherList", rows, emptyText, (item, vault) => {
      const button = vaultSwitchButton(vault, "switcher");
      button.setAttribute("role", "menuitem");
      item.appendChild(button);
    });
  }

  function renderManageVaultsModal() {
    const modal = $("manageVaultsModal");
    if (!modal) return;
    modal.hidden = !state.manageVaultsModalOpen;
    modal.setAttribute("aria-hidden", String(!state.manageVaultsModalOpen));
    const shell = document.querySelector?.(".obsidian-shell");
    if (shell) shell.dataset.manageVaultsOpen = state.manageVaultsModalOpen ? "true" : "false";
    setText("manageVaultsCurrentName", activeVaultLabel());
    const status = sessionStatusView(state.sessionStatus);
    setText(
      "manageVaultsCurrentDetail",
      state.metadata
        ? `${status.title}. ${vaultManagementSummary(state.metadata)}`
        : `${status.title}. Select a Vault, then ${status.locked ? "unlock it" : "load it"} to open encrypted content.`
    );
    const signerConnected = state.signerStatus === "connected";
    safeSetHidden("manageVaultsConnectSignerButton", signerConnected);
    setOptionalDisabled(
      "manageVaultsConnectSignerButton",
      !deriveBrainIdentityProviderState(state.identityProvider).canConnect
    );
    const action = state.sessionStatus === SESSION_STATUS.LOCKED
      ? "Unlock Vault"
      : state.sessionStatus === SESSION_STATUS.RESUMING
        ? "Unlocking…"
        : "Load";
    setText("manageVaultsLoadButton", action);
    setOptionalDisabled(
      "manageVaultsLoadButton",
      state.sessionStatus === SESSION_STATUS.RESUMING || !canLoadVault()
    );
    safeSetHidden("manageVaultCreateDetails", !showsCreateOrganizationControl(state.metadata));
    setOptionalDisabled(
      "manageCreateOrganizationVaultButton",
      state.sessionStatus !== SESSION_STATUS.UNLOCKED || state.signerStatus !== "connected" || state.readerBusy || !state.config
    );
    const rows = visibleVaultOptions();
    const emptyText = signerConnected ? "No Vaults available." : "Connect a signer to list Vaults.";
    setList("manageVaultsList", rows, emptyText, (item, vault) => {
      item.appendChild(vaultSwitchButton(vault, "manage"));
    });
  }

  function sessionGrantOpeningAllowed(status) {
    return status === SESSION_STATUS.RESUMING || status === SESSION_STATUS.UNLOCKED;
  }

  function pageKey(folderId, objectId) {
    return `${folderId}/${objectId}`;
  }

  function createSessionKeyring() {
    return {
      keys: new Map(),
      openedGrants: [],
    };
  }

  function clearSessionKeyring(keyring) {
    keyring?.keys?.clear?.();
    if (Array.isArray(keyring?.openedGrants)) keyring.openedGrants.length = 0;
  }

  function cloneSessionKeyring(keyring) {
    return {
      keys: new Map(keyring?.keys || []),
      openedGrants: [...(keyring?.openedGrants || [])],
    };
  }

  function folderKeyId(vaultId, folderId, keyVersion) {
    return `${vaultId}:${folderId}:${keyVersion}`;
  }

  async function importFolderKey(keyring, { vaultId, folderId, keyVersion, folderKey }, options = {}) {
    options.assertCurrent?.();
    const rawKey = base64ToBytes(folderKey);
    if (rawKey.length !== 32) throw new Error("Folder Key must be 32 bytes");
    const cryptoKey = await crypto.subtle.importKey("raw", rawKey, "AES-GCM", false, [
      "encrypt",
      "decrypt",
    ]);
    options.assertCurrent?.();
    const id = folderKeyId(vaultId, folderId, keyVersion);
    keyring.keys.set(id, {
      cryptoKey,
      folderId,
      keyVersion,
      rawKey,
      vaultId,
    });
    return keyring.keys.get(id);
  }

  async function openFolderKeyGrantPlaintext(keyring, grantPlaintext, options = {}) {
    if (grantPlaintext.version !== "finite-folder-key-grant-v1") {
      throw new Error("unsupported Folder Key Grant version");
    }
    const opened = await importFolderKey(keyring, grantPlaintext, options);
    options.assertCurrent?.();
    const alreadyOpened = keyring.openedGrants.some(
      (grant) =>
        grant.folderId === grantPlaintext.folderId &&
        grant.keyVersion === grantPlaintext.keyVersion &&
        grant.recipientNpub === grantPlaintext.recipientNpub &&
        grant.vaultId === grantPlaintext.vaultId
    );
    if (!alreadyOpened) {
      keyring.openedGrants.push({
        folderId: grantPlaintext.folderId,
        issuerNpub: grantPlaintext.issuerNpub,
        keyVersion: grantPlaintext.keyVersion,
        recipientNpub: grantPlaintext.recipientNpub,
        vaultId: grantPlaintext.vaultId,
      });
    }
    return opened;
  }

  function isHex64(value) {
    return typeof value === "string" && /^[0-9a-f]{64}$/i.test(value);
  }

  function requireHex64(value, field) {
    if (!isHex64(value)) throw new Error(`${field} must be a 64-character hex public key`);
    return value.toLowerCase();
  }

  function parseJsonObject(value, field) {
    let parsed;
    try {
      parsed = JSON.parse(value);
    } catch (_) {
      throw new Error(`${field} is not valid JSON`);
    }
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      throw new Error(`${field} must be a JSON object`);
    }
    return parsed;
  }

  function publicKeyTags(event) {
    return (Array.isArray(event?.tags) ? event.tags : []).filter(
      (tag) => Array.isArray(tag) && tag[0] === "p" && typeof tag[1] === "string"
    );
  }

  function validateGiftWrapShell(event, expectedRecipientHex) {
    if (!event || typeof event !== "object") throw new Error("Folder Key Grant wrapper is missing");
    if (event.kind !== 1059) throw new Error("Folder Key Grant wrapper must be kind 1059");
    requireHex64(event.pubkey, "gift wrap pubkey");
    if (typeof event.content !== "string" || !event.content) {
      throw new Error("Folder Key Grant wrapper content is missing");
    }
    const recipients = publicKeyTags(event).map((tag) => requireHex64(tag[1], "gift wrap recipient tag"));
    if (!recipients.length) throw new Error("Folder Key Grant wrapper is missing a recipient tag");
    if (expectedRecipientHex && !recipients.includes(expectedRecipientHex)) {
      throw new Error("Folder Key Grant wrapper is not addressed to the connected signer");
    }
  }

  function validateSealEvent(event) {
    if (!event || typeof event !== "object") throw new Error("Folder Key Grant seal is missing");
    if (event.kind !== 13) throw new Error("Folder Key Grant seal must be kind 13");
    requireHex64(event.pubkey, "seal pubkey");
    if (typeof event.content !== "string" || !event.content) {
      throw new Error("Folder Key Grant seal content is missing");
    }
  }

  function canonicalNostrEventIdInput(event) {
    return JSON.stringify([
      0,
      event.pubkey,
      Number(event.created_at),
      Number(event.kind),
      Array.isArray(event.tags) ? event.tags : [],
      typeof event.content === "string" ? event.content : "",
    ]);
  }

  async function validateRumorEvent(event, expectedIssuerHex) {
    if (!event || typeof event !== "object") throw new Error("Folder Key Grant rumor is missing");
    if (event.kind !== APP_EVENT_KIND) throw new Error(`Folder Key Grant rumor must be kind ${APP_EVENT_KIND}`);
    const rumorPubkey = requireHex64(event.pubkey, "rumor pubkey");
    if (expectedIssuerHex && rumorPubkey !== expectedIssuerHex) {
      throw new Error("Folder Key Grant rumor issuer does not match the seal");
    }
    if (typeof event.content !== "string" || !event.content) {
      throw new Error("Folder Key Grant rumor content is missing");
    }
    if (event.id !== undefined && event.id !== null) {
      requireHex64(event.id, "rumor id");
      const expectedId = await sha256Hex(canonicalNostrEventIdInput(event));
      if (event.id.toLowerCase() !== expectedId) {
        throw new Error("Folder Key Grant rumor id does not match its content");
      }
    }
  }

  function validateFolderKeyGrantPlaintext(plaintext, expectedRecipientNpub = null, grant = null) {
    if (!plaintext || typeof plaintext !== "object") throw new Error("Folder Key Grant plaintext is missing");
    if (plaintext.version !== "finite-folder-key-grant-v1") throw new Error("unsupported Folder Key Grant version");
    if (!plaintext.folderKey) throw new Error("Folder Key Grant is missing a Folder Key");
    if (expectedRecipientNpub && plaintext.recipientNpub !== expectedRecipientNpub) {
      throw new Error("Folder Key Grant recipient does not match the connected signer");
    }
    if (grant?.folderId && plaintext.folderId !== grant.folderId) {
      throw new Error("Folder Key Grant folder does not match export metadata");
    }
    if (grant?.keyVersion && Number(plaintext.keyVersion) !== Number(grant.keyVersion)) {
      throw new Error("Folder Key Grant key version does not match export metadata");
    }
    if (grant?.recipientNpub && plaintext.recipientNpub !== grant.recipientNpub) {
      throw new Error("Folder Key Grant recipient does not match export metadata");
    }
    return plaintext;
  }

  function plaintextDevelopmentGrantFromExportGrant(grant, expectedRecipientNpub = null) {
    if (!grant?.wrappedEventJson) return null;
    let wrapped;
    try {
      wrapped = JSON.parse(grant.wrappedEventJson);
    } catch (_) {
      return null;
    }
    if (typeof wrapped.content !== "string") return null;
    let plaintext;
    try {
      plaintext = JSON.parse(wrapped.content);
    } catch (_) {
      return null;
    }
    if (plaintext.version !== "finite-folder-key-grant-v1" || !plaintext.folderKey) return null;
    if (expectedRecipientNpub && plaintext.recipientNpub !== expectedRecipientNpub) return null;
    return plaintext;
  }

  function nip44DecryptAdapter(options = {}) {
    if (options.decrypt) return options.decrypt;
    if (options.provider?.nip44 && typeof options.provider.nip44.decrypt === "function") {
      return (pubkeyHex, ciphertext) =>
        invokeNip44ProviderMethod(options.provider, "decrypt", pubkeyHex, ciphertext);
    }
    const provider = options.brainIdentityProvider || state.identityProvider;
    if (provider?.grantOperationMode === "scoped") return null;
    if (typeof provider?.openGrantPayload === "function") {
      return (peerPublicKeyHex, ciphertext) =>
        provider.openGrantPayload({
          purpose: options.grantPurpose || "folder-key-grant",
          peerPublicKeyHex,
          ciphertext,
        });
    }
    return null;
  }

  function nip44EncryptAdapter(options = {}) {
    if (options.encrypt) return options.encrypt;
    if (options.provider?.nip44 && typeof options.provider.nip44.encrypt === "function") {
      return (pubkeyHex, plaintext) =>
        invokeNip44ProviderMethod(options.provider, "encrypt", pubkeyHex, plaintext);
    }
    const provider = options.brainIdentityProvider || state.identityProvider;
    if (provider?.grantOperationMode === "scoped") return null;
    if (typeof provider?.wrapGrantPayload === "function") {
      return (peerPublicKeyHex, plaintext) =>
        provider.wrapGrantPayload({
          purpose: options.grantPurpose || "folder-key-grant",
          peerPublicKeyHex,
          plaintext,
        });
    }
    return null;
  }

  async function invokeNip44ProviderMethod(provider, method, peerHex, payload) {
    const api = provider?.nip44;
    const operation = api?.[method];
    if (typeof operation !== "function") throw new Error(`NIP-44 ${method} is unavailable`);
    try {
      return await operation.call(api, peerHex, payload);
    } catch (error) {
      if (!/reading 'enable'/.test(String(error?.message || error))) throw error;
      const receiver = Object.create(api || null);
      receiver.provider = provider;
      const prototypeOperation = Object.getPrototypeOf(api || {})?.[method];
      const fallbacks =
        typeof prototypeOperation === "function" && prototypeOperation !== operation
          ? [prototypeOperation, operation]
          : [operation];
      let fallbackError = error;
      for (const fallback of fallbacks) {
        try {
          return await fallback.call(receiver, peerHex, payload);
        } catch (nextError) {
          fallbackError = nextError;
        }
      }
      throw fallbackError;
    }
  }

  async function openGiftWrappedRumorContent(wrappedEventJson, expectedRecipientNpub = null, options = {}, label = "Folder Key Grant") {
    if (!wrappedEventJson) throw new Error(`${label} wrapper is missing`);
    const decrypt = nip44DecryptAdapter(options);
    if (!decrypt) throw new Error("NIP-44 decryption is unavailable");
    const expectedRecipientHex = expectedRecipientNpub ? npubToHex(expectedRecipientNpub) : null;
    const giftWrap = parseJsonObject(wrappedEventJson, `${label} wrapper`);
    validateGiftWrapShell(giftWrap, expectedRecipientHex);
    const sealPlaintext = await decrypt(requireHex64(giftWrap.pubkey, "gift wrap pubkey"), giftWrap.content);
    options.assertCurrent?.();
    const seal = parseJsonObject(sealPlaintext, `${label} seal`);
    validateSealEvent(seal);
    const sealIssuerHex = requireHex64(seal.pubkey, "seal pubkey");
    const rumorPlaintext = await decrypt(sealIssuerHex, seal.content);
    options.assertCurrent?.();
    const rumor = parseJsonObject(rumorPlaintext, `${label} rumor`);
    await validateRumorEvent(rumor, sealIssuerHex);
    return {
      giftWrap,
      rumor,
      seal,
    };
  }

  async function plaintextGrantFromGiftWrappedExportGrant(grant, expectedRecipientNpub = null, options = {}) {
    const identityProvider = options.brainIdentityProvider || state.identityProvider;
    if (
      identityProvider?.grantOperationMode === "scoped" &&
      !options.decrypt &&
      !options.provider
    ) {
      const plaintext = await identityProvider.openGrantPayload({
        purpose: "folder-key-grant",
        vaultId: options.expectedVaultId || grant?.vaultId,
        folderId: grant?.folderId,
        keyVersion: Number(grant?.keyVersion),
        recipientNpub: expectedRecipientNpub || grant?.recipientNpub,
        wrappedEventJson: grant?.wrappedEventJson,
      });
      options.assertCurrent?.();
      return validateFolderKeyGrantPlaintext(plaintext, expectedRecipientNpub, grant);
    }
    const { rumor } = await openGiftWrappedRumorContent(
      grant?.wrappedEventJson,
      expectedRecipientNpub,
      options,
      "Folder Key Grant"
    );
    const plaintext = parseJsonObject(rumor.content, "Folder Key Grant plaintext");
    return validateFolderKeyGrantPlaintext(plaintext, expectedRecipientNpub, grant);
  }

  async function openFolderKeyGrants(keyring, exportedVault, expectedRecipientNpub = null, options = {}) {
    const opened = [];
    const skipped = [];
    for (const grant of exportedVault?.keyGrants || []) {
      try {
        options.assertCurrent?.();
        const plaintext = await plaintextGrantFromGiftWrappedExportGrant(grant, expectedRecipientNpub, {
          ...options,
          expectedVaultId: exportedVault?.vaultId,
        });
        options.assertCurrent?.();
        await openFolderKeyGrantPlaintext(keyring, plaintext, options);
        options.assertCurrent?.();
        opened.push({
          folderId: plaintext.folderId,
          keyVersion: plaintext.keyVersion,
        });
      } catch (error) {
        if (typeof options.assertCurrent === "function") {
          try {
            options.assertCurrent();
          } catch (sessionError) {
            clearSessionKeyring(keyring);
            throw sessionError;
          }
        }
        skipped.push({
          id: grant.id || grant.folderId || "unknown-grant",
          error: error.message,
        });
      }
    }
    return { opened, skipped };
  }

  async function openDevelopmentFolderKeyGrants(keyring, exportedVault, expectedRecipientNpub = null) {
    const opened = [];
    const skipped = [];
    for (const grant of exportedVault?.keyGrants || []) {
      const plaintext = plaintextDevelopmentGrantFromExportGrant(grant, expectedRecipientNpub);
      if (!plaintext) {
        skipped.push(grant.id || grant.folderId || "unknown-grant");
        continue;
      }
      await openFolderKeyGrantPlaintext(keyring, plaintext);
      opened.push({
        folderId: plaintext.folderId,
        keyVersion: plaintext.keyVersion,
      });
    }
    return { opened, skipped };
  }

  function canonicalFolderObjectAad({ vaultId, folderId, objectId, keyVersion }) {
    return `{"version":${JSON.stringify(FOLDER_OBJECT_VERSION)},"vaultId":${JSON.stringify(
      vaultId
    )},"folderId":${JSON.stringify(folderId)},"objectId":${JSON.stringify(
      objectId
    )},"keyVersion":${keyVersion}}`;
  }

  function canonicalEnvelope({ keyVersion, nonce, ciphertext }) {
    return `{"version":${JSON.stringify(FOLDER_OBJECT_VERSION)},"cipher":${JSON.stringify(
      CIPHER
    )},"keyVersion":${keyVersion},"nonce":${JSON.stringify(nonce)},"ciphertext":${JSON.stringify(
      ciphertext
    )}}`;
  }

  async function encryptFolderObject(keyring, input) {
    const key = keyring.keys.get(folderKeyId(input.vaultId, input.folderId, input.keyVersion));
    if (!key) throw new Error(`No Folder Key opened for ${input.folderId} v${input.keyVersion}`);
    const nonce = input.nonceBytes || crypto.getRandomValues(new Uint8Array(12));
    if (nonce.length !== 12) throw new Error("AES-GCM nonce must be 12 bytes");
    const aad = new TextEncoder().encode(canonicalFolderObjectAad(input));
    const plaintext = new TextEncoder().encode(input.plaintext);
    const ciphertext = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: nonce, additionalData: aad },
      key.cryptoKey,
      plaintext
    );
    return canonicalEnvelope({
      keyVersion: input.keyVersion,
      nonce: bytesToBase64(nonce),
      ciphertext: bytesToBase64(new Uint8Array(ciphertext)),
    });
  }

  async function openFolderObject(keyring, input) {
    const envelope = typeof input.ciphertext === "string" ? JSON.parse(input.ciphertext) : input.ciphertext;
    const key = keyring.keys.get(folderKeyId(input.vaultId, input.folderId, envelope.keyVersion));
    if (!key) {
      return {
        folderId: input.folderId,
        objectId: input.objectId,
        revision: input.revision,
        status: "locked",
      };
    }
    const aad = new TextEncoder().encode(
      canonicalFolderObjectAad({
        vaultId: input.vaultId,
        folderId: input.folderId,
        objectId: input.objectId,
        keyVersion: envelope.keyVersion,
      })
    );
    const plaintext = await crypto.subtle.decrypt(
      {
        name: "AES-GCM",
        iv: base64ToBytes(envelope.nonce),
        additionalData: aad,
      },
      key.cryptoKey,
      base64ToBytes(envelope.ciphertext)
    );
    const opened = decodeFolderObjectPlaintext(
      new TextDecoder().decode(plaintext),
      input.path || `${input.objectId}.md`
    );
    if (opened.type === "asset") {
      return {
        bytes: opened.bytes,
        bytesBase64: opened.bytesBase64,
        contentHash: opened.contentHash,
        contentType: opened.contentType,
        filename: opened.filename,
        folderId: input.folderId,
        objectId: input.objectId,
        path: opened.path,
        revision: input.revision,
        status: "ready",
        type: "asset",
      };
    }
    return {
      folderId: input.folderId,
      objectId: input.objectId,
      path: opened.path,
      revision: input.revision,
      status: "ready",
      text: opened.markdown,
      type: "page",
    };
  }

  function decodeFolderObjectPagePlaintext(plaintext, fallbackPath) {
    const opened = decodeFolderObjectPlaintext(plaintext, fallbackPath);
    if (opened.type !== "page") throw new Error(`Folder object ${opened.path} is an Asset, not a Page`);
    return { path: opened.path, markdown: opened.markdown };
  }

  function decodeFolderObjectPlaintext(plaintext, fallbackPath) {
    const fallback = normalizeSafeRelativePath(fallbackPath || "page.md", "Page path");
    try {
      const object = JSON.parse(String(plaintext || ""));
      if (object?.type === "asset") {
        const path = normalizeAssetPath(object.path, "Asset path");
        const bytesBase64 = String(object.bytesBase64 || "");
        const bytes = base64ToBytes(bytesBase64);
        const size = Number(object.size ?? bytes.length);
        if (size !== bytes.length) throw new Error("Asset size does not match decoded bytes");
        return {
          bytes,
          bytesBase64,
          contentHash: String(object.contentHash || ""),
          contentType: String(object.contentType || "application/octet-stream"),
          filename: String(object.filename || path.split("/").at(-1) || "asset"),
          path,
          size,
          type: "asset",
        };
      }
      const page = object;
      if (page?.version === FOLDER_OBJECT_PAGE_VERSION) {
        const path = normalizeSafeRelativePath(page.path, "Page path");
        if (!path.toLowerCase().endsWith(".md")) throw new Error("Page path must end in .md");
        if (typeof page.markdown !== "string") throw new Error("Page markdown must be a string");
        return { path, markdown: page.markdown, type: "page" };
      }
    } catch (error) {
      if (error instanceof SyntaxError) return { path: fallback, markdown: String(plaintext || ""), type: "page" };
      throw error;
    }
    return { path: fallback, markdown: String(plaintext || ""), type: "page" };
  }

  function encodeFolderObjectPagePlaintext(path, markdown) {
    const safePath = normalizeSafeRelativePath(path || "page.md", "Page path");
    if (!safePath.toLowerCase().endsWith(".md")) throw new Error("Page path must end in .md");
    return JSON.stringify({
      version: FOLDER_OBJECT_PAGE_VERSION,
      path: safePath,
      markdown: String(markdown || ""),
    });
  }

  async function encodeFolderObjectAssetPlaintext(path, bytes, contentType = "application/octet-stream") {
    const safePath = normalizeAssetPath(path || "raw/assets/asset.bin", "Asset path");
    const rawBytes = bytes instanceof Uint8Array ? bytes : base64ToBytes(String(bytes || ""));
    const filename = safePath.split("/").at(-1) || "asset";
    return JSON.stringify({
      type: "asset",
      path: safePath,
      filename,
      contentType: String(contentType || "application/octet-stream"),
      size: rawBytes.length,
      contentHash: await sha256HexBytes(rawBytes),
      bytesBase64: bytesToBase64(rawBytes),
    });
  }

  async function ciphertextHash(envelopeJson) {
    return sha256Hex(envelopeJson);
  }

  function revisionCreatedAt(createdAtUnix) {
    return new Date(createdAtUnix * 1000).toISOString().replace(".000Z", "Z");
  }

  function accessChangeCreatedAt(createdAtUnix) {
    return new Date(createdAtUnix * 1000).toISOString().replace(".000Z", "Z");
  }

  function canonicalRevisionPayload(input) {
    const baseRevision = input.baseRevision === undefined ? null : input.baseRevision;
    return `{"version":${JSON.stringify(REVISION_VERSION)},"vaultId":${JSON.stringify(
      input.vaultId
    )},"folderId":${JSON.stringify(input.folderId)},"objectId":${JSON.stringify(
      input.objectId
    )},"operation":${JSON.stringify(input.operation)},"revision":${
      input.revision
    },"baseRevision":${baseRevision === null ? "null" : baseRevision},"keyVersion":${
      input.keyVersion
    },"cipher":${JSON.stringify(CIPHER)},"ciphertextHash":${JSON.stringify(
      input.ciphertextHash
    )},"authorNpub":${JSON.stringify(input.authorNpub)},"createdAt":${JSON.stringify(
      input.createdAt
    )}}`;
  }

  function revisionTags(input) {
    return [
      [
        "d",
        `finite-folder-object-revision:${input.vaultId}:${input.folderId}:${input.objectId}:${input.revision}`,
      ],
      ["vault", input.vaultId],
      ["folder", input.folderId],
      ["object", input.objectId],
      ["operation", input.operation],
      ["keyVersion", String(input.keyVersion)],
    ];
  }

  function canonicalTombstonePayload(input) {
    return `{"version":${JSON.stringify(TOMBSTONE_VERSION)},"vaultId":${JSON.stringify(
      input.vaultId
    )},"folderId":${JSON.stringify(input.folderId)},"objectId":${JSON.stringify(
      input.objectId
    )},"operation":"delete","revision":${input.revision},"baseRevision":${
      input.baseRevision
    },"authorNpub":${JSON.stringify(input.authorNpub)},"deletedAt":${JSON.stringify(
      input.deletedAt
    )}}`;
  }

  function tombstoneTags(input) {
    return [
      [
        "d",
        `finite-folder-object-tombstone:${input.vaultId}:${input.folderId}:${input.objectId}:${input.revision}`,
      ],
      ["vault", input.vaultId],
      ["folder", input.folderId],
      ["object", input.objectId],
      ["operation", "delete"],
    ];
  }

  async function buildPageWriteRequest(keyring, input) {
    const baseRevision =
      input.baseRevision === "" || input.baseRevision === undefined || input.baseRevision === null
        ? null
        : Number(input.baseRevision);
    const revision = baseRevision === null ? 1 : baseRevision + 1;
    const envelopeJson = await encryptFolderObject(keyring, {
      folderId: input.folderId,
      keyVersion: input.keyVersion,
      nonceBytes: input.nonceBytes,
      objectId: input.objectId,
      plaintext: input.plaintext,
      vaultId: input.vaultId,
    });
    const createdAtUnix = input.createdAtUnix || Math.floor(Date.now() / 1000);
    const payload = canonicalRevisionPayload({
      authorNpub: input.authorNpub,
      baseRevision,
      ciphertextHash: await ciphertextHash(envelopeJson),
      createdAt: revisionCreatedAt(createdAtUnix),
      folderId: input.folderId,
      keyVersion: input.keyVersion,
      objectId: input.objectId,
      operation: input.operation || (baseRevision === null ? "create" : "update"),
      revision,
      vaultId: input.vaultId,
    });
    const eventTemplate = {
      kind: APP_EVENT_KIND,
      created_at: createdAtUnix,
      tags: revisionTags({
        folderId: input.folderId,
        objectId: input.objectId,
        operation: input.operation || (baseRevision === null ? "create" : "update"),
        keyVersion: input.keyVersion,
        revision,
        vaultId: input.vaultId,
      }),
      content: payload,
    };
    const revisionEvent = await input.signEvent(eventTemplate);
    return {
      baseRevision,
      keyVersion: input.keyVersion,
      cipher: CIPHER,
      ciphertext: envelopeJson,
      revisionEvent,
    };
  }

  async function buildPageDeleteRequest(input) {
    const baseRevision = Number(input.baseRevision);
    if (!Number.isInteger(baseRevision) || baseRevision < 1) {
      throw new Error("Page delete requires a positive base revision");
    }
    const revision = baseRevision + 1;
    const createdAtUnix = input.createdAtUnix || Math.floor(Date.now() / 1000);
    const deletedAt = revisionCreatedAt(createdAtUnix);
    const payload = canonicalTombstonePayload({
      authorNpub: input.authorNpub,
      baseRevision,
      deletedAt,
      folderId: input.folderId,
      objectId: input.objectId,
      revision,
      vaultId: input.vaultId,
    });
    const eventTemplate = {
      kind: APP_EVENT_KIND,
      created_at: createdAtUnix,
      tags: tombstoneTags({
        folderId: input.folderId,
        objectId: input.objectId,
        revision,
        vaultId: input.vaultId,
      }),
      content: payload,
    };
    const tombstoneEvent = await input.signEvent(eventTemplate);
    return {
      baseRevision,
      tombstoneEvent,
    };
  }

  function mergeSyncProjection(projection, sync) {
    const next = {
      pages: new Map(projection.pages),
      seenEventIds: new Set(projection.seenEventIds),
      localDrafts: new Map(projection.localDrafts),
      conflicts: [...projection.conflicts],
    };
    for (const record of sync.records || []) {
      if (next.seenEventIds.has(record.recordEventId)) continue;
      next.seenEventIds.add(record.recordEventId);
    }
    for (const object of sync.objects || []) {
      const key = pageKey(object.folderId, object.objectId);
      const localDraft = next.localDrafts.get(key);
      if (localDraft && object.revision > localDraft.baseRevision) {
        next.conflicts.push({
          folderId: object.folderId,
          objectId: object.objectId,
          localBaseRevision: localDraft.baseRevision,
          serverRevision: object.revision,
          status: "conflict",
        });
        continue;
      }
      if (object.deleted) {
        next.pages.delete(key);
        next.localDrafts.delete(key);
        continue;
      }
      next.pages.set(key, object);
    }
    return next;
  }

  async function openSyncObjects(keyring, sync) {
    if (!keyring) return sync;
    const objects = await Promise.all(
      (sync.objects || []).map(async (object) => {
        if (object.deleted) return object;
        try {
          const opened = await openFolderObject(keyring, object);
          return {
            ...object,
            ...opened,
            title: opened.text
              ? pageTitleFromText(opened.text, pageTitleFromPath(opened.path || object.path, object.objectId))
              : object.title,
          };
        } catch (error) {
          return {
            ...object,
            error: error.message,
            status: "locked",
          };
        }
      })
    );
    return {
      ...sync,
      objects,
    };
  }

  function pageTitleFromText(text, fallback) {
    const heading = String(text || "").match(/^#\s+(.+)$/m);
    return heading ? heading[1].trim() : fallback;
  }

  function pageTitleFromPath(path, fallback) {
    const filename = String(path || "")
      .split("/")
      .filter(Boolean)
      .pop();
    return filename ? filename.replace(/\.md$/i, "") : fallback;
  }

  function pageTitleForPage(page) {
    return page.title || pageTitleFromText(page.text ?? "", pageTitleFromPath(page.path, page.objectId));
  }

  function pageKeyForPage(page) {
    return page.key || pageKey(page.folderId, page.objectId);
  }

  function normalizePageReference(value) {
    return String(value || "")
      .trim()
      .replace(/^\.?\//, "")
      .replace(/\.md$/i, "")
      .replace(/^#/, "")
      .toLowerCase();
  }

  function extractPageLinks(text) {
    const links = new Set();
    const wikiPattern = /\[\[([^\]|#]+)(?:[|#][^\]]*)?\]\]/g;
    const markdownPattern = /\[[^\]]+\]\(([^)]+)\)/g;
    for (const match of String(text || "").matchAll(wikiPattern)) {
      links.add(normalizePageReference(match[1]));
    }
    for (const match of String(text || "").matchAll(markdownPattern)) {
      const target = match[1].split("#")[0];
      if (!/^https?:\/\//i.test(target)) links.add(normalizePageReference(target));
    }
    return [...links].filter(Boolean);
  }

  function pageReferencesForPage(page) {
    return [
      pageTitleForPage(page),
      page.path || `${page.objectId}.md`,
      String(page.path || `${page.objectId}.md`).split("/").pop(),
    ]
      .map(normalizePageReference)
      .filter(Boolean);
  }

  function pageReferenceMap(pages = readablePages()) {
    const byReference = new Map();
    for (const page of pages.filter(isReadablePage)) {
      for (const reference of pageReferencesForPage(page)) {
        if (!byReference.has(reference)) byReference.set(reference, page);
      }
    }
    return byReference;
  }

  function pageForReference(reference, pages = readablePages()) {
    return pageReferenceMap(pages).get(normalizePageReference(reference)) || null;
  }

  function pageKeyForReference(reference, pages = readablePages()) {
    const page = pageForReference(reference, pages);
    return page ? pageKeyForPage(page) : null;
  }

  function inlineLinkSegments(text) {
    const source = String(text || "");
    const segments = [];
    const pattern = /\[\[([^\]|#]+)(?:#[^\]|]*)?(?:\|([^\]]+))?\]\]|\[([^\]]+)\]\(([^)]+)\)/g;
    let cursor = 0;
    for (const match of source.matchAll(pattern)) {
      if (match.index > cursor) {
        segments.push({ kind: "text", text: source.slice(cursor, match.index) });
      }
      if (match[1]) {
        segments.push({
          kind: "internal",
          target: normalizePageReference(match[1]),
          text: String(match[2] || match[1]).trim(),
        });
      } else {
        const target = String(match[4] || "").trim();
        const external = /^https?:\/\//i.test(target);
        segments.push({
          kind: external ? "external" : "internal",
          target: external ? target : normalizePageReference(target.split("#")[0]),
          text: String(match[3] || target).trim(),
        });
      }
      cursor = match.index + match[0].length;
    }
    if (cursor < source.length) {
      segments.push({ kind: "text", text: source.slice(cursor) });
    }
    return segments.filter((segment) => segment.text || segment.target);
  }

  function splitMarkdownTableRow(line) {
    let source = String(line || "").trim();
    if (!source.includes("|")) return null;
    if (source.startsWith("|")) source = source.slice(1);
    if (source.endsWith("|")) source = source.slice(0, -1);
    const cells = [];
    let cell = "";
    let escaped = false;
    for (const char of source) {
      if (char === "\\" && !escaped) {
        escaped = true;
        cell += char;
        continue;
      }
      if (char === "|" && !escaped) {
        cells.push(cell.trim().replaceAll("\\|", "|"));
        cell = "";
        continue;
      }
      escaped = false;
      cell += char;
    }
    cells.push(cell.trim().replaceAll("\\|", "|"));
    return cells.length > 1 ? cells : null;
  }

  function tableDelimiterAlignments(cells) {
    const alignments = [];
    for (const cell of cells || []) {
      const value = String(cell || "").trim();
      if (!/^:?-{3,}:?$/.test(value)) return null;
      if (value.startsWith(":") && value.endsWith(":")) alignments.push("center");
      else if (value.endsWith(":")) alignments.push("right");
      else if (value.startsWith(":")) alignments.push("left");
      else alignments.push("");
    }
    return alignments.length ? alignments : null;
  }

  function parseMarkdownListItem(line) {
    const trimmed = String(line || "").trim();
    const ordered = trimmed.match(/^(\d+)[.)]\s+(.+)$/);
    if (ordered) {
      return {
        checked: null,
        ordered: true,
        start: Number(ordered[1]) || 1,
        text: ordered[2].trim(),
      };
    }
    const unordered = trimmed.match(/^[-*+]\s+(.+)$/);
    if (!unordered) return null;
    const task = unordered[1].match(/^\[([ xX])\]\s+(.+)$/);
    return {
      checked: task ? task[1].toLowerCase() === "x" : null,
      ordered: false,
      start: null,
      text: (task ? task[2] : unordered[1]).trim(),
    };
  }

  function normalizeMarkdownTableRow(cells, width) {
    return Array.from({ length: width }, (_, index) => String(cells[index] || "").trim());
  }

  function normalizeCodeBlockText(value) {
    let lines = Array.isArray(value)
      ? value.map((line) => String(line || ""))
      : String(value || "").replace(/\r\n/g, "\n").split("\n");
    while (lines.length && !lines[0].trim()) lines.shift();
    while (lines.length && !lines[lines.length - 1].trim()) lines.pop();
    const indentedLines = lines.filter((line) => line.trim()).map((line) => line.match(/^[ \t]*/)?.[0] || "");
    if (indentedLines.length && indentedLines.every((indent) => indent.length > 0)) {
      let sharedIndent = indentedLines[0];
      for (const indent of indentedLines.slice(1)) {
        while (sharedIndent && !indent.startsWith(sharedIndent)) sharedIndent = sharedIndent.slice(0, -1);
      }
      if (sharedIndent) {
        lines = lines.map((line) => (line.startsWith(sharedIndent) ? line.slice(sharedIndent.length) : line));
      }
    }
    return lines.join("\n");
  }

  function markdownPreviewBlocks(markdown, options = {}) {
    const lines = String(markdown || "").replace(/\r\n/g, "\n").split("\n");
    const blocks = [];
    let paragraph = [];

    function flushParagraph() {
      if (!paragraph.length) return;
      blocks.push({ text: paragraph.join(" "), type: "paragraph" });
      paragraph = [];
    }

    for (let index = 0; index < lines.length; index += 1) {
      const line = lines[index];
      const trimmed = line.trim();
      if (!trimmed) {
        flushParagraph();
        continue;
      }
      const fence = trimmed.match(/^(```|~~~)\s*([A-Za-z0-9_+.#-]+)?\s*$/);
      if (fence) {
        flushParagraph();
        const code = [];
        const fenceMarker = fence[1];
        const language = fence[2] || "";
        index += 1;
        while (index < lines.length && !lines[index].trim().startsWith(fenceMarker)) {
          code.push(lines[index]);
          index += 1;
        }
        blocks.push({ language, text: normalizeCodeBlockText(code), type: "code" });
        continue;
      }
      const heading = trimmed.match(/^(#{1,6})\s+(.+?)\s*#*$/);
      if (heading) {
        flushParagraph();
        blocks.push({ level: heading[1].length, text: heading[2].trim(), type: "heading" });
        continue;
      }
      const headerCells = splitMarkdownTableRow(trimmed);
      const delimiterCells = splitMarkdownTableRow(lines[index + 1] || "");
      const tableAlignments = tableDelimiterAlignments(delimiterCells);
      if (headerCells && tableAlignments && headerCells.length === tableAlignments.length) {
        flushParagraph();
        const width = headerCells.length;
        const rows = [];
        index += 2;
        while (index < lines.length) {
          const rowCells = splitMarkdownTableRow(lines[index]);
          if (!rowCells) break;
          rows.push(normalizeMarkdownTableRow(rowCells, width));
          index += 1;
        }
        index -= 1;
        blocks.push({
          alignments: tableAlignments,
          headers: normalizeMarkdownTableRow(headerCells, width),
          rows,
          type: "table",
        });
        continue;
      }
      const listItem = parseMarkdownListItem(trimmed);
      if (listItem) {
        flushParagraph();
        const ordered = listItem.ordered;
        const items = [];
        let start = listItem.start;
        while (index < lines.length) {
          const item = parseMarkdownListItem(lines[index]);
          if (!item || item.ordered !== ordered) break;
          if (start === null) start = item.start;
          const itemRecord = {
            checked: item.checked,
            text: item.text,
          };
          if (options.includeSourcePositions) itemRecord.sourceLineIndex = index;
          items.push(itemRecord);
          index += 1;
        }
        index -= 1;
        blocks.push({ items, ordered, start, type: "list" });
        continue;
      }
      if (/^>\s?/.test(trimmed)) {
        flushParagraph();
        const quotes = [];
        while (index < lines.length && /^>\s?/.test(lines[index].trim())) {
          quotes.push(lines[index].trim().replace(/^>\s?/, ""));
          index += 1;
        }
        index -= 1;
        blocks.push({ text: quotes.join(" "), type: "quote" });
        continue;
      }
      if (/^([-*_])(?:\s*\1){2,}$/.test(trimmed)) {
        flushParagraph();
        blocks.push({ type: "rule" });
        continue;
      }
      paragraph.push(trimmed);
    }
    flushParagraph();
    return blocks;
  }

  function pageStatsForText(text) {
    const clean = String(text || "").trim();
    const words = clean ? clean.split(/\s+/).filter(Boolean).length : 0;
    return {
      links: extractPageLinks(clean).length,
      words,
    };
  }

  function normalizeSafeRelativePath(value, label = "path") {
    const normalized = String(value || "")
      .trim()
      .replace(/^\.\/+/, "");
    if (
      !normalized ||
      normalized.startsWith("/") ||
      normalized.includes("\\") ||
      normalized.split("/").some((segment) => !segment || segment === "." || segment === "..") ||
      [".finitebrain", "_admin", ".git"].includes(normalized.split("/")[0])
    ) {
      throw new Error(`${label} must be a safe relative path`);
    }
    return normalized;
  }

  function targetPathFromBundlePath(path) {
    const safePath = normalizeSafeRelativePath(path, "OKF object path");
    const parts = safePath.split("/");
    if (parts[0] === "content" && parts.length >= 3) return parts.slice(2).join("/");
    return safePath;
  }

  function normalizeAssetPath(value, label = "Asset path") {
    const safePath = normalizeSafeRelativePath(value || "raw/assets/asset.bin", label);
    if (!safePath.startsWith("raw/assets/")) throw new Error(`${label} must live under raw/assets/`);
    return safePath;
  }

  function assetTargetPathFromBundlePath(path) {
    const safePath = targetPathFromBundlePath(path);
    if (safePath.startsWith("raw/assets/")) return safePath;
    const filename = safePath.split("/").filter(Boolean).pop() || "asset";
    return normalizeAssetPath(`raw/assets/${filename}`, "OKF asset target path");
  }

  function parseOkfBundle(input, options = {}) {
    const source = typeof input === "string" ? JSON.parse(input) : input;
    if (!source || typeof source !== "object") throw new Error("OKF bundle must be a JSON object");

    const sourceFiles = source.files || source;
    const files = new Map();
    for (const [path, content] of Object.entries(sourceFiles || {})) {
      if (typeof content === "string" && (path.endsWith(".md") || path === "okf-vault.json")) {
        files.set(normalizeSafeRelativePath(path, "OKF file path"), content);
      }
    }

    const manifest =
      source.manifest ||
      (files.has("okf-vault.json") ? JSON.parse(files.get("okf-vault.json")) : null);
    const pages = [];
    const assets = [];
    if (Array.isArray(source.pages)) {
      source.pages.forEach((page, index) => {
        const sourcePath = normalizeSafeRelativePath(
          page.sourcePath || page.path || page.targetPath || `import/page-${index + 1}.md`,
          "OKF page source path"
        );
        const targetPath = normalizeSafeRelativePath(
          page.targetPath || page.pagePath || targetPathFromBundlePath(page.path || sourcePath),
          "OKF page target path"
        );
        const markdown = page.markdown ?? page.content;
        if (typeof markdown !== "string") throw new Error(`OKF page ${sourcePath} is missing content`);
        pages.push({
          sourceFolderId: page.folderId || null,
          sourceObjectId: page.objectId || null,
          sourcePath,
          folderId: options.destinationFolderId || page.targetFolderId || page.folderId || DEFAULT_CLIENT_FOLDER_ID,
          targetPath,
          markdown,
          contentType: page.contentType || "text/markdown",
          links: extractPageLinks(markdown),
        });
      });
    }
    if (Array.isArray(source.assets)) {
      source.assets.forEach((asset, index) => {
        const sourcePath = normalizeSafeRelativePath(
          asset.sourcePath || asset.path || asset.targetPath || `attachments/asset-${index + 1}`,
          "OKF asset source path"
        );
        const targetPath = normalizeAssetPath(
          asset.targetPath || asset.assetPath || assetTargetPathFromBundlePath(asset.path || sourcePath),
          "OKF asset target path"
        );
        const bytesBase64 = String(asset.bytesBase64 || "");
        assets.push({
          sourceFolderId: asset.folderId || null,
          sourceObjectId: asset.objectId || null,
          sourcePath,
          folderId: options.destinationFolderId || asset.targetFolderId || asset.folderId || DEFAULT_CLIENT_FOLDER_ID,
          targetPath,
          bytesBase64,
          contentHash: asset.contentHash || "",
          contentType: asset.contentType || "application/octet-stream",
          size: asset.size === undefined ? base64ToBytes(bytesBase64).length : Number(asset.size),
        });
      });
    } else if (manifest?.objects) {
      for (const object of manifest.objects) {
        const sourcePath = normalizeSafeRelativePath(object.path, "OKF manifest object path");
        const contentType = object.contentType || "text/markdown";
        if (contentType === "text/markdown" && !Array.isArray(source.pages)) {
          const markdown = files.get(sourcePath);
          if (typeof markdown !== "string") throw new Error(`OKF file missing for ${sourcePath}`);
          pages.push({
            sourceFolderId: object.folderId || null,
            sourceObjectId: object.objectId || null,
            sourcePath,
            folderId: options.destinationFolderId || object.targetFolderId || object.folderId || DEFAULT_CLIENT_FOLDER_ID,
            targetPath: normalizeSafeRelativePath(
              object.targetPath || object.pagePath || targetPathFromBundlePath(sourcePath),
              "OKF page target path"
            ),
            markdown,
            contentType,
            links: extractPageLinks(markdown),
          });
        } else {
          const rawAsset = sourceFiles?.[sourcePath];
          const bytesBase64 =
            typeof object.bytesBase64 === "string"
              ? object.bytesBase64
              : typeof rawAsset === "string"
                ? rawAsset
                : String(rawAsset?.bytesBase64 || "");
          assets.push({
            sourceFolderId: object.folderId || null,
            sourceObjectId: object.objectId || null,
            sourcePath,
            folderId: options.destinationFolderId || object.targetFolderId || object.folderId || DEFAULT_CLIENT_FOLDER_ID,
            targetPath: normalizeAssetPath(
              object.targetPath || object.assetPath || assetTargetPathFromBundlePath(sourcePath),
              "OKF asset target path"
            ),
            bytesBase64,
            contentHash: object.contentHash || rawAsset?.contentHash || "",
            contentType,
            size: object.size === undefined ? base64ToBytes(bytesBase64).length : Number(object.size),
          });
        }
      }
    } else {
      for (const [sourcePath, markdown] of files.entries()) {
        if (sourcePath === "okf-vault.json" || sourcePath.startsWith("_wiki/")) continue;
        pages.push({
          sourceFolderId: null,
          sourceObjectId: null,
          sourcePath,
          folderId: options.destinationFolderId || DEFAULT_CLIENT_FOLDER_ID,
          targetPath: targetPathFromBundlePath(sourcePath),
          markdown,
          contentType: "text/markdown",
          links: extractPageLinks(markdown),
        });
      }
    }

    return {
      version: manifest?.version || source.version || "finite-okf-vault-import-v1",
      assets,
      pages,
      omissions: manifest?.omissions || source.omissions || [],
    };
  }

  function normalizeExistingPageRecord(record) {
    const folderId = record.folderId || DEFAULT_CLIENT_FOLDER_ID;
    const path =
      record.path ||
      record.pagePath ||
      record.targetPath ||
      (record.title ? `${slugForObjectId(record.title)}.md` : `${record.objectId}.md`);
    return {
      folderId,
      objectId: record.objectId,
      revision: Number(record.revision || 0),
      targetPath: normalizeSafeRelativePath(path, "existing Page path"),
    };
  }

  function targetKey(folderId, targetPath) {
    return `${folderId}\n${targetPath}`;
  }

  function slugForObjectId(value) {
    return String(value || "page")
      .trim()
      .toLowerCase()
      .replace(/\.md$/i, "")
      .replace(/[^a-z0-9_-]+/g, "_")
      .replace(/^_+|_+$/g, "")
      .slice(0, 88) || "page";
  }

  function validObjectId(value) {
    return /^[A-Za-z0-9_-]{16,128}$/.test(value || "") && !String(value).includes(".");
  }

  function objectIdForTargetPath(targetPath, occupiedObjectIds) {
    const base = `obj_${slugForObjectId(targetPath)}`.padEnd(16, "0").slice(0, 112);
    let candidate = base;
    let index = 2;
    while (occupiedObjectIds.has(candidate) || !validObjectId(candidate)) {
      if (index > MAX_OBJECT_ID_ATTEMPTS) {
        throw new Error(`could not allocate import object id for ${targetPath}`);
      }
      candidate = `${base}_${index}`.slice(0, 128);
      index += 1;
    }
    occupiedObjectIds.add(candidate);
    return candidate;
  }

  function uniqueImportedCopyPath(folderId, targetPath, occupiedTargets) {
    const safePath = normalizeSafeRelativePath(targetPath, "copy target path");
    const slash = safePath.lastIndexOf("/");
    const dot = safePath.lastIndexOf(".");
    const hasExtension = dot > slash;
    const stem = hasExtension ? safePath.slice(0, dot) : safePath;
    const extension = hasExtension ? safePath.slice(dot) : "";
    for (let index = 1; index <= 1000; index += 1) {
      const suffix = index === 1 ? " imported" : ` imported ${index}`;
      const candidate = normalizeSafeRelativePath(`${stem}${suffix}${extension}`, "copy target path");
      if (!occupiedTargets.has(targetKey(folderId, candidate))) return candidate;
    }
    throw new Error(`Could not allocate copy path for ${targetPath}`);
  }

  function resolveRelativePath(fromPath, target) {
    if (!target || target.startsWith("#") || /^https?:\/\//i.test(target) || target.startsWith("mailto:")) {
      return null;
    }
    const cleanTarget = target.split("#")[0];
    if (cleanTarget.startsWith("/") || cleanTarget.includes("\\")) return null;
    const parts = fromPath.split("/");
    parts.pop();
    for (const segment of cleanTarget.split("/")) {
      if (!segment || segment === ".") continue;
      if (segment === "..") {
        if (!parts.length) return null;
        parts.pop();
      } else {
        parts.push(segment);
      }
    }
    try {
      return normalizeSafeRelativePath(parts.join("/"), "OKF link target");
    } catch (_) {
      return null;
    }
  }

  function relativePathBetween(fromPath, toPath) {
    const from = fromPath.split("/");
    from.pop();
    const to = toPath.split("/");
    let common = 0;
    while (common < from.length && common < to.length && from[common] === to[common]) common += 1;
    return [...Array(from.length - common).fill(".."), ...to.slice(common)].join("/") || toPath;
  }

  function rewriteOkfMarkdownLinks(markdown, sourcePath, targetPath, sourcePathToEntry) {
    return String(markdown || "").replace(/\[([^\]]+)\]\(([^)]+)\)/g, (original, label, href) => {
      const resolved = resolveRelativePath(sourcePath, href);
      if (!resolved) return original;
      const target = sourcePathToEntry.get(resolved);
      if (!target || target.action === "skip") return original;
      return `[${label}](${relativePathBetween(targetPath, target.targetPath)})`;
    });
  }

  function planOkfImport(bundleOrInput, existingPages = [], options = {}) {
    const bundle = bundleOrInput?.pages ? bundleOrInput : parseOkfBundle(bundleOrInput, options);
    const mode = options.conflictMode || "skip";
    if (!["skip", "copy", "overwrite"].includes(mode)) {
      throw new Error("OKF conflict mode must be skip, copy, or overwrite");
    }

    const existingByPath = new Map();
    const occupiedTargets = new Set();
    const plannedTargets = new Set();
    const occupiedObjectIds = new Set();
    for (const page of existingPages.map(normalizeExistingPageRecord)) {
      existingByPath.set(targetKey(page.folderId, page.targetPath), page);
      occupiedTargets.add(targetKey(page.folderId, page.targetPath));
      if (page.objectId) occupiedObjectIds.add(page.objectId);
    }

    const entries = [];
    for (const page of bundle.pages) {
      const folderId = page.folderId || options.destinationFolderId || DEFAULT_CLIENT_FOLDER_ID;
      let targetPath = normalizeSafeRelativePath(page.targetPath, "OKF page target path");
      const existing = existingByPath.get(targetKey(folderId, targetPath));
      let action = "create";
      let objectId = null;
      let baseRevision = null;
      if (existing) {
        if (mode === "skip") {
          action = "skip";
          objectId = existing.objectId || null;
        }
        if (mode === "copy") {
          action = "copy";
          targetPath = uniqueImportedCopyPath(folderId, targetPath, occupiedTargets);
          objectId = objectIdForTargetPath(targetPath, occupiedObjectIds);
        }
        if (mode === "overwrite") {
          action = "overwrite";
          objectId = existing.objectId;
          baseRevision = existing.revision;
        }
      } else {
        objectId = objectIdForTargetPath(targetPath, occupiedObjectIds);
      }
      occupiedTargets.add(targetKey(folderId, targetPath));
      plannedTargets.add(targetKey(folderId, targetPath));
      entries.push({
        action,
        baseRevision,
        contentType: page.contentType || "text/markdown",
        folderId,
        kind: "page",
        links: [...(page.links || extractPageLinks(page.markdown))],
        markdown: page.markdown,
        objectId,
        sourcePath: page.sourcePath,
        targetPath,
      });
    }
    for (const asset of bundle.assets || []) {
      const folderId = asset.folderId || options.destinationFolderId || DEFAULT_CLIENT_FOLDER_ID;
      let targetPath = normalizeAssetPath(asset.targetPath, "OKF asset target path");
      const existing = existingByPath.get(targetKey(folderId, targetPath));
      const alreadyPlanned = plannedTargets.has(targetKey(folderId, targetPath));
      let action = "create";
      let objectId = null;
      let baseRevision = null;
      if (alreadyPlanned) {
        action = "copy";
        targetPath = uniqueImportedCopyPath(folderId, targetPath, occupiedTargets);
        objectId = objectIdForTargetPath(targetPath, occupiedObjectIds);
      } else if (existing) {
        if (mode === "skip") {
          action = "skip";
          objectId = existing.objectId || null;
        }
        if (mode === "copy") {
          action = "copy";
          targetPath = uniqueImportedCopyPath(folderId, targetPath, occupiedTargets);
          objectId = objectIdForTargetPath(targetPath, occupiedObjectIds);
        }
        if (mode === "overwrite") {
          action = "overwrite";
          objectId = existing.objectId;
          baseRevision = existing.revision;
        }
      } else {
        objectId = objectIdForTargetPath(targetPath, occupiedObjectIds);
      }
      occupiedTargets.add(targetKey(folderId, targetPath));
      plannedTargets.add(targetKey(folderId, targetPath));
      entries.push({
        action,
        baseRevision,
        bytesBase64: asset.bytesBase64 || "",
        contentHash: asset.contentHash || "",
        contentType: asset.contentType || "application/octet-stream",
        folderId,
        kind: "asset",
        links: [],
        objectId,
        size: asset.size,
        sourcePath: asset.sourcePath,
        targetPath,
      });
    }

    const sourcePathToEntry = new Map(entries.map((entry) => [entry.sourcePath, entry]));
    for (const entry of entries) {
      if (entry.action !== "skip" && entry.kind !== "asset") {
        entry.markdown = rewriteOkfMarkdownLinks(
          entry.markdown,
          entry.sourcePath,
          entry.targetPath,
          sourcePathToEntry
        );
        entry.links = extractPageLinks(entry.markdown);
      }
    }

    return {
      mode,
      entries,
      summary: {
        create: entries.filter((entry) => entry.action === "create").length,
        copy: entries.filter((entry) => entry.action === "copy").length,
        overwrite: entries.filter((entry) => entry.action === "overwrite").length,
        skip: entries.filter((entry) => entry.action === "skip").length,
      },
    };
  }

  function folderKeyVersionForImport(folderId, options = {}) {
    if (options.keyVersionByFolderId instanceof Map && options.keyVersionByFolderId.has(folderId)) {
      return options.keyVersionByFolderId.get(folderId);
    }
    if (options.keyVersionByFolderId?.[folderId]) return options.keyVersionByFolderId[folderId];
    if (typeof options.currentKeyVersion === "function") return options.currentKeyVersion(folderId);
    return options.keyVersion || 1;
  }

  async function prepareOkfImportWrites(keyring, plan, options) {
    if (!keyring) throw new Error("Open destination Folder Keys before importing OKF");
    if (!options?.vaultId) throw new Error("OKF import requires a destination Vault");
    if (!options?.authorNpub) throw new Error("OKF import requires a connected signer");
    if (typeof options.signEvent !== "function") throw new Error("OKF import requires event signing");

    const writes = [];
    const skipped = [];
    let nonceIndex = 0;
    for (const entry of plan.entries) {
      if (entry.action === "skip") {
        skipped.push(entry);
        continue;
      }
      const keyVersion = folderKeyVersionForImport(entry.folderId, options);
      const keyId = folderKeyId(options.vaultId, entry.folderId, keyVersion);
      if (!keyring.keys.has(keyId)) {
        throw new Error(
          `Folder Key is not open for ${entry.folderId}; OKF import cannot write locked destination Folder`
        );
      }
      const nonceBytes =
        typeof options.nonceFactory === "function" ? options.nonceFactory(nonceIndex, entry) : undefined;
      nonceIndex += 1;
      const plaintext =
        entry.kind === "asset"
          ? await encodeFolderObjectAssetPlaintext(
              entry.targetPath,
              base64ToBytes(entry.bytesBase64 || ""),
              entry.contentType || "application/octet-stream"
            )
          : encodeFolderObjectPagePlaintext(entry.targetPath, entry.markdown);
      const body = await buildPageWriteRequest(keyring, {
        authorNpub: options.authorNpub,
        baseRevision: entry.baseRevision,
        createdAtUnix: options.createdAtUnix,
        folderId: entry.folderId,
        keyVersion,
        nonceBytes,
        objectId: entry.objectId,
        operation: entry.action === "overwrite" ? "update" : "create",
        plaintext,
        signEvent: options.signEvent,
        vaultId: options.vaultId,
      });
      writes.push({
        action: entry.action,
        body,
        folderId: entry.folderId,
        objectId: entry.objectId,
        path: `/_admin/vaults/${encodeURIComponent(options.vaultId)}/folders/${encodeURIComponent(
          entry.folderId
        )}/objects/${encodeURIComponent(entry.objectId)}`,
        sourcePath: entry.sourcePath,
        targetPath: entry.targetPath,
      });
    }
    return { skipped, writes };
  }

  function buildGraphProjection(pages) {
    const visiblePages = [...pages].filter(isReadablePage);
    const nodes = visiblePages.map((page) => {
      const id = pageKey(page.folderId, page.objectId);
      const title = pageTitleForPage(page);
      return {
        id,
        folderId: page.folderId,
        objectId: page.objectId,
        title,
        normalizedTitle: normalizePageReference(title),
      };
    });
    const titleToNode = new Map(nodes.map((node) => [node.normalizedTitle, node]));
    const edges = [];
    for (const page of visiblePages) {
      const source = nodes.find((node) => node.id === pageKey(page.folderId, page.objectId));
      if (!source) continue;
      for (const targetRef of extractPageLinks(page.text)) {
        const target = titleToNode.get(targetRef);
        if (!target) continue;
        edges.push({
          id: `${source.id}->${target.id}`,
          source: source.id,
          target: target.id,
        });
      }
    }
    return {
      nodes,
      edges,
    };
  }

  function graphStats(graph) {
    return {
      edgeCount: graph.edges.length,
      nodeCount: graph.nodes.length,
    };
  }

  function graphNeighborIds(graph, nodeId) {
    const neighbors = new Set(nodeId ? [nodeId] : []);
    if (!nodeId) return neighbors;
    for (const edge of graph.edges || []) {
      if (edge.source === nodeId) neighbors.add(edge.target);
      if (edge.target === nodeId) neighbors.add(edge.source);
    }
    return neighbors;
  }

  function stableGraphHash(value) {
    let hash = 2166136261;
    for (const char of String(value || "")) {
      hash ^= char.charCodeAt(0);
      hash = Math.imul(hash, 16777619);
    }
    return hash >>> 0;
  }

  function stableUnitInterval(value) {
    return stableGraphHash(value) / 0xffffffff;
  }

  function graphLayout(graph, options = {}) {
    const width = Number(options.width || graphViewport.width);
    const height = Number(options.height || graphViewport.height);
    const margin = Number(options.margin || 44);
    const centerX = width / 2;
    const centerY = height / 2;
    const positions = new Map();
    if (!graph.nodes.length) return positions;

    const degree = new Map(graph.nodes.map((node) => [node.id, 0]));
    for (const edge of graph.edges) {
      degree.set(edge.source, (degree.get(edge.source) || 0) + 1);
      degree.set(edge.target, (degree.get(edge.target) || 0) + 1);
    }
    const orderedNodes = [...graph.nodes].sort((left, right) => {
      const degreeDelta = (degree.get(right.id) || 0) - (degree.get(left.id) || 0);
      if (degreeDelta) return degreeDelta;
      return left.title.localeCompare(right.title);
    });
    const radiusX = Math.max(70, width / 2 - margin);
    const radiusY = Math.max(70, height / 2 - margin);
    if (orderedNodes.length === 1) {
      positions.set(orderedNodes[0].id, { x: centerX, y: centerY });
      return positions;
    }

    const folderIds = [...new Set(orderedNodes.map((node) => node.folderId || ""))].sort();
    const folderCenters = new Map();
    folderIds.forEach((folderId, index) => {
      const angle =
        (Math.PI * 2 * index) / Math.max(1, folderIds.length) +
        stableUnitInterval(`folder-angle:${folderId}`) * 0.82;
      const radius = 0.42 + stableUnitInterval(`folder-radius:${folderId}`) * 0.38;
      folderCenters.set(folderId, {
        x: centerX + Math.cos(angle) * radiusX * radius,
        y: centerY + Math.sin(angle) * radiusY * radius,
      });
    });

    const hasHub = orderedNodes.length > 4 && (degree.get(orderedNodes[0].id) || 0) > 1;
    const nodeState = orderedNodes.map((node) => {
      const nodeDegree = degree.get(node.id) || 0;
      const folderCenter = folderCenters.get(node.folderId || "") || { x: centerX, y: centerY };
      const jitterAngle = stableUnitInterval(`node-angle:${node.id}`) * Math.PI * 2;
      const jitterRadius = Math.sqrt(stableUnitInterval(`node-radius:${node.id}`)) * 188;
      const scatterAngle = stableUnitInterval(`loose-angle:${node.id}`) * Math.PI * 2;
      const scatterRadius = 0.2 + stableUnitInterval(`loose-radius:${node.id}`) * 0.72;
      const looseX = centerX + Math.cos(scatterAngle) * radiusX * scatterRadius;
      const looseY = centerY + Math.sin(scatterAngle) * radiusY * scatterRadius;
      const fixed = hasHub && node.id === orderedNodes[0].id && orderedNodes.length < 18;
      return {
        fixed,
        id: node.id,
        loose: nodeDegree === 0,
        x: fixed
          ? centerX
          : nodeDegree === 0
            ? looseX
            : folderCenter.x + Math.cos(jitterAngle) * jitterRadius,
        y: fixed
          ? centerY
          : nodeDegree === 0
            ? looseY
            : folderCenter.y + Math.sin(jitterAngle) * jitterRadius,
        vx: 0,
        vy: 0,
      };
    });
    const byId = new Map(nodeState.map((node) => [node.id, node]));
    const links = graph.edges
      .map((edge) => ({ source: byId.get(edge.source), target: byId.get(edge.target) }))
      .filter((edge) => edge.source && edge.target);
    const iterations = Math.min(260, Math.max(130, orderedNodes.length * 6));
    const linkDistance = Math.max(92, Math.min(168, 152 - Math.sqrt(orderedNodes.length) * 1.1));
    const repulsion = Math.max(780, Math.min(2600, 18000 / Math.sqrt(orderedNodes.length)));
    for (let iteration = 0; iteration < iterations; iteration += 1) {
      for (let leftIndex = 0; leftIndex < nodeState.length; leftIndex += 1) {
        for (let rightIndex = leftIndex + 1; rightIndex < nodeState.length; rightIndex += 1) {
          const left = nodeState[leftIndex];
          const right = nodeState[rightIndex];
          let dx = right.x - left.x;
          let dy = right.y - left.y;
          let distanceSq = dx * dx + dy * dy;
          if (distanceSq < 0.01) {
            dx = stableUnitInterval(`overlap-x:${left.id}:${right.id}`) - 0.5;
            dy = stableUnitInterval(`overlap-y:${left.id}:${right.id}`) - 0.5;
            distanceSq = dx * dx + dy * dy;
          }
          const distance = Math.sqrt(distanceSq);
          const force = repulsion / Math.max(distanceSq, 160);
          const fx = (dx / distance) * force;
          const fy = (dy / distance) * force;
          if (!left.fixed) {
            left.vx -= fx;
            left.vy -= fy;
          }
          if (!right.fixed) {
            right.vx += fx;
            right.vy += fy;
          }
        }
      }
      for (const link of links) {
        const dx = link.target.x - link.source.x;
        const dy = link.target.y - link.source.y;
        const distance = Math.max(1, Math.sqrt(dx * dx + dy * dy));
        const force = (distance - linkDistance) * 0.012;
        const fx = (dx / distance) * force;
        const fy = (dy / distance) * force;
        if (!link.source.fixed) {
          link.source.vx += fx;
          link.source.vy += fy;
        }
        if (!link.target.fixed) {
          link.target.vx -= fx;
          link.target.vy -= fy;
        }
      }
      for (const node of nodeState) {
        if (node.fixed) {
          node.x = centerX;
          node.y = centerY;
          node.vx = 0;
          node.vy = 0;
          continue;
        }
        const centerForce = node.loose ? 0.00022 : 0.00068;
        node.vx += (centerX - node.x) * centerForce;
        node.vy += (centerY - node.y) * centerForce;
        node.vx *= 0.9;
        node.vy *= 0.9;
        node.x = Math.min(width - margin, Math.max(margin, node.x + node.vx));
        node.y = Math.min(height - margin, Math.max(margin, node.y + node.vy));
      }
    }

    for (const node of nodeState) {
      positions.set(node.id, {
        x: Math.round(node.fixed ? centerX : node.x),
        y: Math.round(node.fixed ? centerY : node.y),
      });
    }
    return positions;
  }

  function decryptedPagesForGraph() {
    const pages = [];
    for (const [key, draft] of state.projection.localDrafts.entries()) {
      const [folderId, objectId] = key.split("/");
      pages.push({
        folderId,
        objectId,
        path: draft.path || `${objectId}.md`,
        status: "ready",
        text: draft.text,
        title: pageTitleFromText(draft.text, pageTitleFromPath(draft.path, objectId)),
      });
    }
    for (const [key, page] of state.projection.pages.entries()) {
      if (isReadablePage(page)) {
        const [folderId, objectId] = key.split("/");
        pages.push({
          folderId,
          objectId,
          path: page.path || `${objectId}.md`,
          status: "ready",
          text: page.text,
          title: pageTitleForPage({ ...page, objectId }),
        });
      }
    }
    return pages;
  }

  function projectionPagesFromProjection(projection) {
    const pages = new Map(
      [...projection.pages.entries()].map(([key, page]) => [
        key,
        {
          ...page,
          key,
          title: pageTitleForPage(page),
        },
      ])
    );
    for (const [key, draft] of projection.localDrafts.entries()) {
      const [folderId, objectId] = key.split("/");
      pages.set(key, {
        baseRevision: draft.baseRevision || 0,
        folderId,
        key,
        localDraft: true,
        objectId,
        path: draft.path || `${objectId}.md`,
        revision: draft.baseRevision || 0,
        status: "ready",
        text: draft.text,
        title: pageTitleFromText(draft.text, pageTitleFromPath(draft.path, objectId)),
      });
    }
    return [...pages.values()];
  }

  function projectionPages() {
    return projectionPagesFromProjection(state.projection);
  }

  function pageTextIsPresent(page) {
    return page?.text !== undefined && page?.text !== null;
  }

  function isAssetObject(page) {
    return page?.type === "asset";
  }

  function isReadablePage(page) {
    return page?.status === "ready" && !isAssetObject(page) && pageTextIsPresent(page);
  }

  function readablePages() {
    return projectionPages().filter(isReadablePage);
  }

  function readerFolderRows(metadata, pages = projectionPages()) {
    const pageCounts = new Map();
    const readableCounts = new Map();
    for (const page of pages) {
      if (isAssetObject(page)) continue;
      pageCounts.set(page.folderId, (pageCounts.get(page.folderId) || 0) + 1);
      if (isReadablePage(page)) {
        readableCounts.set(page.folderId, (readableCounts.get(page.folderId) || 0) + 1);
      }
    }
    return metadataFolderRows(metadata).map((folder) => ({
      ...folder,
      pageCount: pageCounts.get(folder.id) || 0,
      readableCount: readableCounts.get(folder.id) || 0,
    }));
  }

  function readerPageRows(folderId, pages = projectionPages()) {
    return pages
      .filter((page) => !folderId || page.folderId === folderId)
      .filter((page) => !isAssetObject(page))
      .map((page) => {
        const title = pageTitleForPage(page);
        return {
          ...page,
          title,
          label: title,
          detail: readerPageDetail({ ...page, title }),
        };
      })
      .sort((left, right) => left.title.localeCompare(right.title));
  }

  function pageLinkContext(page, pages = readablePages()) {
    if (!isReadablePage(page)) return { backlinks: [], outgoing: [] };
    const readable = [...pages].filter(isReadablePage);
    const byReference = pageReferenceMap(readable);
    const currentKey = pageKeyForPage(page);
    const currentReferences = new Set(pageReferencesForPage(page));
    const outgoing = extractPageLinks(page.text).map((targetRef) => {
      const target = byReference.get(targetRef);
      if (!target) {
        return {
          detail: "unresolved",
          key: null,
          label: targetRef,
          status: "missing",
        };
      }
      return {
        detail: target.folderId,
        key: pageKeyForPage(target),
        label: pageTitleForPage(target),
        status: "resolved",
      };
    });
    const backlinks = readable
      .filter((candidate) => pageKeyForPage(candidate) !== currentKey)
      .filter((candidate) =>
        extractPageLinks(candidate.text).some((targetRef) => currentReferences.has(targetRef))
      )
      .map((candidate) => ({
        detail: candidate.folderId,
        key: pageKeyForPage(candidate),
        label: pageTitleForPage(candidate),
        status: "resolved",
      }))
      .sort((left, right) => left.label.localeCompare(right.label));
    return { backlinks, outgoing };
  }

  function pageCountLabel(count) {
    return `${count} ${count === 1 ? "page" : "pages"}`;
  }

  function pagePathLabel(page) {
    if (!page) return "No page path loaded";
    return `${page.folderId}/${page.path || `${page.objectId}.md`}`;
  }

  function readerPageDetail(page) {
    if (!page) return "";
    if (page.status === "ready") {
      return page.path || `${page.objectId}.md`;
    }
    return `locked - ${page.folderId}/${page.objectId}`;
  }

  function readerFolderDetail(row) {
    if (!row.pageCount) return "Empty";
    if (row.readableCount === row.pageCount) {
      return pageCountLabel(row.pageCount);
    }
    if (!row.readableCount) {
      return "Locked";
    }
    return `${row.readableCount}/${row.pageCount}`;
  }

  function selectDefaultReaderTargets() {
    const folders = readerFolderRows(state.metadata);
    const folderStillExists = folders.some((folder) => folder.id === state.selectedFolderId);
    let selectedFolderChanged = false;
    if (!folderStillExists) {
      const folderWithReadablePages = folders.find((folder) => folder.readableCount > 0);
      state.selectedFolderId = folderWithReadablePages?.id || folders[0]?.id || null;
      selectedFolderChanged = Boolean(state.selectedFolderId);
    }
    if (selectedFolderChanged) state.expandedFolderIds.add(state.selectedFolderId);

    const pages = readerPageRows(state.selectedFolderId);
    const pageStillExists = pages.some((page) => page.key === state.selectedPageKey);
    if (!pageStillExists) {
      const readablePage = pages.find((page) => page.status === "ready");
      state.selectedPageKey = readablePage?.key || pages[0]?.key || null;
    }
  }

  function syncReaderInputsFromSelectedPage() {
    const page = selectedReaderPage();
    $("pageFolderIdInput").value = page?.folderId || state.selectedFolderId || DEFAULT_CLIENT_FOLDER_ID;
    $("pageObjectIdInput").value = page?.objectId || "";
    $("pageBaseRevisionInput").value = page ? String(page.revision || "") : "";
    setEditorDraftText(page && pageTextIsPresent(page) ? page.text : "");
    return page;
  }

  function clearSearchHighlight() {
    state.searchHighlight = null;
    state.searchHighlightShouldScroll = false;
  }

  function selectedReaderPage() {
    if (!state.selectedPageKey) return null;
    return projectionPages().find((page) => page.key === state.selectedPageKey) || null;
  }

  function workspaceTabTitle(metadata, page) {
    return page?.title || metadata?.name || "Open a Vault";
  }

  function sidebarModeLabel(mode) {
    return (
      {
        files: "Files",
        search: "Search",
      }[normalizeSidebarMode(mode)] || "Files"
    );
  }

  function normalizeSidebarMode(mode) {
    return ["files", "search"].includes(mode) ? mode : "files";
  }

  function commandPaletteCommands() {
    return [
      { id: "files", kind: "command", label: "Files", detail: "Sidebar", target: "files" },
      { id: "search", kind: "command", label: "Search", detail: "Sidebar", target: "search" },
      { id: "access", kind: "command", label: "Vault access", detail: "Settings", target: "access" },
      { id: "graph", kind: "command", label: "Graph View", detail: "Workspace", target: "graph" },
      { id: "new-page", kind: "command", label: "New Page", detail: "Current Folder", target: "new-page" },
      { id: "refresh", kind: "command", label: "Refresh Vault", detail: "Sync", target: "refresh" },
    ];
  }

  function commandPaletteRows(query, pages = readablePages()) {
    const needle = String(query || "").trim().toLowerCase();
    const pageRows = pages.filter(isReadablePage).map((page) => ({
      detail: pagePathLabel(page),
      id: page.key || pageKey(page.folderId, page.objectId),
      kind: "page",
      label: pageTitleForPage(page),
      pageKey: page.key || pageKey(page.folderId, page.objectId),
    }));
    const rows = [...commandPaletteCommands(), ...pageRows];
    if (!needle) return rows.slice(0, 12);
    return rows
      .filter((row) =>
        [row.label, row.detail, row.kind].filter(Boolean).join("\n").toLowerCase().includes(needle)
      )
      .slice(0, 12);
  }

  function searchHighlightSegments(text, query) {
    const source = String(text || "");
    const needle = String(query || "").trim();
    if (!source) return [];
    if (!needle) return [{ match: false, text: source }];

    const haystack = source.toLowerCase();
    const normalizedNeedle = needle.toLowerCase();
    const segments = [];
    let cursor = 0;
    let matchIndex = haystack.indexOf(normalizedNeedle, cursor);

    while (matchIndex >= 0) {
      if (matchIndex > cursor) {
        segments.push({ match: false, text: source.slice(cursor, matchIndex) });
      }
      segments.push({
        match: true,
        text: source.slice(matchIndex, matchIndex + needle.length),
      });
      cursor = matchIndex + needle.length;
      matchIndex = haystack.indexOf(normalizedNeedle, cursor);
    }

    if (cursor < source.length) {
      segments.push({ match: false, text: source.slice(cursor) });
    }
    return segments.length ? segments : [{ match: false, text: source }];
  }

  function readerSearchHighlightForPage(pageKeyValue, searchHighlight) {
    const query = String(searchHighlight?.query || "").trim();
    return searchHighlight?.pageKey === pageKeyValue ? query : "";
  }

  function searchTextSnippet(text, query, maxLength = 96) {
    const source = String(text || "").replace(/\s+/g, " ").trim();
    const needle = String(query || "").trim();
    if (!source || !needle) return "";

    const matchIndex = source.toLowerCase().indexOf(needle.toLowerCase());
    if (matchIndex < 0) return "";

    const snippetLength = Math.max(maxLength, needle.length);
    const start = Math.max(0, matchIndex - Math.floor((snippetLength - needle.length) / 2));
    const end = Math.min(source.length, start + snippetLength);
    const prefix = start > 0 ? "…" : "";
    const suffix = end < source.length ? "…" : "";
    return `${prefix}${source.slice(start, end).trim()}${suffix}`;
  }

  function searchResultSnippet(page, query) {
    return [page?.text, page?.path, page?.folderId, page?.title]
      .map((text) => searchTextSnippet(text, query))
      .find(Boolean) || "";
  }

  function searchPageRows(query, pages = readablePages()) {
    const needle = String(query || "").trim().toLowerCase();
    if (!needle) return [];
    return pages
      .filter(isReadablePage)
      .filter((page) => {
        const haystack = [page.title, page.path, page.folderId, page.text].filter(Boolean).join("\n").toLowerCase();
        return haystack.includes(needle);
      })
      .sort((left, right) =>
        pageTitleForPage(left).localeCompare(pageTitleForPage(right))
      )
      .map((page) => ({
        ...page,
        label: pageTitleForPage(page),
        detail: `${page.folderId}/${page.path || `${page.objectId}.md`}`,
        matchSnippet: searchResultSnippet(page, needle),
      }));
  }

  function contextMenuItemsForTarget(target) {
    if (!target) return [];
    if (target.type === "page") {
      const discardLocalDraft = pageDeletionDisposition(target) === "discard-local";
      const pageKeyValue = target.pageKey || pageKey(target.folderId, target.objectId);
      const saveInFlight = state.pageSaveInFlight?.key === pageKeyValue;
      return [
        { action: "open-page", label: "Open Page" },
        { action: "new-page", label: "New Page in Folder" },
        { action: "open-graph", label: "Show in Graph View" },
        { separator: true },
        { action: "copy-page-id", label: "Copy Page ID" },
        { action: "copy-folder-id", label: "Copy Folder ID" },
        { separator: true },
        {
          action: "delete-page",
          label: saveInFlight ? "Saving Page…" : discardLocalDraft ? "Discard unsaved Page" : "Delete Page",
          disabled: saveInFlight,
          danger: true,
        },
      ];
    }
    return [
      { action: "open-folder", label: "Open Folder" },
      { action: "new-page", label: "New Page" },
      { action: "new-folder", label: "New Folder Inside" },
      { separator: true },
      { action: "copy-folder-id", label: "Copy Folder ID" },
      { action: "manage-access", label: "Manage Access" },
      { action: "share-folder", label: "Share Folder" },
    ];
  }

  function setSidebarMode(mode) {
    const nextMode = normalizeSidebarMode(mode);
    state.activeSidebarMode = nextMode;
    closeContextMenu();
    render();
  }

  function setWorkspaceView(view) {
    state.activeWorkspaceView = view === "graph" ? "graph" : "page";
    render();
  }

  function workspaceChromeState(view) {
    const pageActive = view !== "graph";
    return {
      graphHidden: pageActive,
      pageHidden: !pageActive,
      ribbonGraphClass: `ribbon-button${pageActive ? "" : " active"}`,
      shellView: pageActive ? "page" : "graph",
    };
  }

  function renderWorkspaceChrome(page = selectedReaderPage()) {
    const chrome = workspaceChromeState(state.activeWorkspaceView);
    const workspaceTitle = workspaceTabTitle(state.metadata, page);
    const shell = document.querySelector(".obsidian-shell");
    shell.dataset.workspaceView = chrome.shellView;
    shell.dataset.vaultLoaded = state.metadata ? "true" : "false";
    $("pageWorkspace").hidden = chrome.pageHidden;
    $("graphWorkspace").hidden = chrome.graphHidden;
    $("ribbonGraphButton").className = chrome.ribbonGraphClass;
    setPressed("ribbonGraphButton", !chrome.graphHidden);
    document.title = chrome.shellView === "graph" ? "Graph View - FiniteBrain" : `${workspaceTitle} - FiniteBrain`;
  }

  function nextDraftObjectId() {
    return `obj_${Date.now().toString(36)}`.padEnd(16, "0").slice(0, 128);
  }

  function visualEditorElement() {
    return $("readerPageContent");
  }

  function focusInlineEditor() {
    const focusDraft = () => {
      if (state.editorMode === "markdown") {
        const draft = $("pageDraftInput");
        draft.focus?.();
        draft.setSelectionRange?.(draft.value.length, draft.value.length);
      } else {
        visualEditorElement()?.focus?.();
      }
    };
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(focusDraft);
    else focusDraft();
  }

  function startNewPageDraft(folderIdOverride = null) {
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED) return;
    clearSearchHighlight();
    const folderId = folderIdOverride || state.selectedFolderId || DEFAULT_CLIENT_FOLDER_ID;
    const objectId = nextDraftObjectId();
    const draftKey = pageKey(folderId, objectId);
    const draftText = "# New Page\n\nStart writing here.";
    state.selectedFolderId = folderId;
    state.selectedPageKey = draftKey;
    state.preparedWrite = null;
    state.preparedWriteTarget = null;
    state.activeWorkspaceView = "page";
    state.expandedFolderIds.add(folderId);
    $("pageFolderIdInput").value = folderId;
    $("pageObjectIdInput").value = objectId;
    $("pageBaseRevisionInput").value = "";
    setEditorDraftText(draftText);
    state.projection.localDrafts.set(draftKey, {
      baseRevision: 0,
      path: `${objectId}.md`,
      text: draftText,
    });
    log("Started a new Page draft.", { folderId, objectId });
    render();
    focusInlineEditor();
  }

  function pageFromContextTarget(target) {
    if (!target || target.type !== "page") return null;
    const key = target.pageKey || pageKey(target.folderId, target.objectId);
    return projectionPages().find((page) => page.key === key) || null;
  }

  function pageDeletionDisposition(page) {
    const revision = Number(page?.revision || 0);
    return page?.localDraft && (!Number.isInteger(revision) || revision < 1)
      ? "discard-local"
      : "tombstone";
  }

  function discardLocalPageDraft(projection, page) {
    if (pageDeletionDisposition(page) !== "discard-local") return false;
    const key = page.key || pageKey(page.folderId, page.objectId);
    return projection?.localDrafts?.delete(key) || false;
  }

  function pageSaveIsInFlight(page) {
    if (!page) return false;
    const key = page.key || pageKey(page.folderId, page.objectId);
    return state.pageSaveInFlight?.key === key;
  }

  async function deletePageFromContextTarget(target) {
    const sessionEpoch = captureSessionOperationEpoch();
    const vaultId = state.activeVaultId;
    const page = pageFromContextTarget(target);
    if (!page || !isReadablePage(page)) throw new Error("Select a readable Page before deleting");
    if (pageSaveIsInFlight(page)) {
      setClientActionFeedback("error", "Page is still saving. Wait for it to finish, then delete it.");
      render();
      return;
    }
    const title = pageTitleForPage(page);
    const disposition = pageDeletionDisposition(page);
    if (
      window.confirm &&
      !window.confirm(
        disposition === "discard-local"
          ? `Discard unsaved "${title}"? This only removes the local draft.`
          : `Delete "${title}"? This writes a signed tombstone.`
      )
    ) {
      return;
    }
    const key = page.key || pageKey(page.folderId, page.objectId);
    if (disposition === "discard-local") {
      discardLocalPageDraft(state.projection, page);
      if (state.preparedWriteTarget && pageKey(state.preparedWriteTarget.folderId, state.preparedWriteTarget.objectId) === key) {
        state.preparedWrite = null;
        state.preparedWriteTarget = null;
      }
      if (state.selectedPageKey === key) state.selectedPageKey = null;
      selectDefaultReaderTargets();
      log("Discarded unsaved local Page draft.", {
        folderId: page.folderId,
        objectId: page.objectId,
      });
      render();
      return;
    }
    if (!page.revision) throw new Error("Page delete requires a saved revision");
    if (state.signerStatus !== "connected") throw new Error("Connect your Brain identity before deleting");
    const body = await buildPageDeleteRequest({
      authorNpub: currentActorNpub(),
      baseRevision: page.revision,
      folderId: page.folderId,
      objectId: page.objectId,
      signEvent: requireBrainEventAuthorizer("folder-object-tombstone"),
      vaultId,
    });
    requireCurrentSessionEpoch(sessionEpoch);
    const route = `/_admin/vaults/${encodeURIComponent(vaultId)}/folders/${encodeURIComponent(
      page.folderId
    )}/objects/${encodeURIComponent(page.objectId)}`;
    const result = await protectedRequest(route, {
      method: "DELETE",
      body: JSON.stringify(body),
    });
    requireCurrentSessionEpoch(sessionEpoch);
    state.projection.pages.delete(key);
    state.projection.localDrafts.delete(key);
    if (state.selectedPageKey === key) state.selectedPageKey = null;
    selectDefaultReaderTargets();
    log("Deleted Page through signed tombstone.", {
      folderId: page.folderId,
      objectId: page.objectId,
      revision: result.revision,
      sequence: result.sequence,
    });
    render();
  }

  function selectReaderFolder(folderId, options = {}) {
    clearSearchHighlight();
    state.selectedFolderId = folderId;
    state.expandedFolderIds.add(folderId);
    if (options.selectFirstPage !== false) {
      const firstPage = readerPageRows(folderId).find((page) => page.status === "ready");
      state.selectedPageKey = firstPage?.key || null;
    }
    state.activeWorkspaceView = "page";
    $("pageFolderIdInput").value = folderId;
    render();
  }

  function selectAccessFolder(folderId, intent = "overview") {
    if (state.activeAccessFolderId !== folderId || state.activeAccessIntent !== intent) {
      state.accessResult = null;
    }
    const folderChanged = state.activeAccessFolderId !== folderId;
    state.activeAccessFolderId = folderId;
    state.activeAccessIntent = intent;
    selectReaderFolder(folderId, { selectFirstPage: false });
    if (folderChanged && state.folderShareLinksFolderId !== folderId) {
      refreshFolderShareLinks(folderId)
        .then(() => render())
        .catch((error) => {
          log("Failed to refresh Folder share links.", { error: error.message });
        });
    }
  }

  function toggleReaderFolder(folderId) {
    clearSearchHighlight();
    const isExpanded = state.expandedFolderIds.has(folderId);
    state.selectedFolderId = folderId;
    $("pageFolderIdInput").value = folderId;
    if (isExpanded) {
      state.expandedFolderIds.delete(folderId);
      state.selectedPageKey = null;
    } else {
      state.expandedFolderIds.add(folderId);
      const firstPage = readerPageRows(folderId).find((page) => page.status === "ready");
      state.selectedPageKey = firstPage?.key || null;
    }
    state.activeWorkspaceView = "page";
    closeContextMenu();
    render();
  }

  function selectReaderPage(pageKeyValue, options = {}) {
    const searchQuery = String(options.searchQuery || "").trim();
    state.searchHighlight = searchQuery ? { pageKey: pageKeyValue, query: searchQuery } : null;
    state.searchHighlightShouldScroll = Boolean(searchQuery);
    state.selectedPageKey = pageKeyValue;
    state.activeWorkspaceView = "page";
    const page = selectedReaderPage();
    if (page) {
      state.selectedFolderId = page.folderId;
      state.expandedFolderIds.add(page.folderId);
    }
    syncReaderInputsFromSelectedPage();
    render();
  }

  function linkElementFromEventTarget(target) {
    const node = target?.nodeType === 3 ? target.parentElement : target;
    return node?.closest?.(".internal-link, .external-link") || null;
  }

  function openInternalPageReference(reference) {
    const key = pageKeyForReference(reference);
    if (!key) return false;
    selectReaderPage(key);
    return true;
  }

  function activatePageContentLink(event) {
    const link = linkElementFromEventTarget(event?.target);
    const content = $("readerPageContent");
    if (!link || !content?.contains?.(link)) return false;
    const target = link.dataset?.target || "";
    if (!target) return false;
    event.preventDefault?.();
    if (String(link.className || "").includes("external-link")) {
      window.open?.(target, "_blank", "noopener,noreferrer");
      return true;
    }
    if (openInternalPageReference(target)) return true;
    state.lastError = `Page link not found: ${target}`;
    log("Page link target was not found.", { target });
    render();
    return true;
  }

  function handlePageContentLinkKeydown(event) {
    if (event.key !== "Enter" && event.key !== " ") return false;
    return activatePageContentLink(event);
  }

  function existingPagesForImport() {
    const pages = [];
    for (const [key, draft] of state.projection.localDrafts.entries()) {
      const [folderId, objectId] = key.split("/");
      pages.push({
        folderId,
        objectId,
        revision: draft.baseRevision || 0,
        path: draft.path || `${objectId}.md`,
        title: pageTitleFromText(draft.text, pageTitleFromPath(draft.path, objectId)),
      });
    }
    for (const [key, page] of state.projection.pages.entries()) {
      const [folderId, objectId] = key.split("/");
      pages.push({
        folderId,
        objectId,
        revision: page.revision || 0,
        path: page.path || `${objectId}.md`,
        title: pageTitleForPage({ ...page, objectId }),
      });
    }
    return pages;
  }

  function graphEmptyStateCopy(options = {}) {
    const readablePageCount = Number(options.readablePageCount || 0);
    if (readablePageCount <= 0) {
      return {
        title: "No graph yet",
        copy: "Open a vault to build the local graph.",
      };
    }
    return {
      title: "No links yet",
      copy: "Readable pages are open, but none link to another page yet.",
    };
  }

  function graphViewBoxForZoom(zoom) {
    const requestedZoom = Number(zoom);
    const normalizedZoom = Math.min(
      GRAPH_ZOOM_MAX,
      Math.max(GRAPH_ZOOM_MIN, Number.isFinite(requestedZoom) ? requestedZoom : 1)
    );
    const width = graphViewport.width / normalizedZoom;
    const height = graphViewport.height / normalizedZoom;
    return {
      height,
      width,
      x: (graphViewport.width - width) / 2,
      y: (graphViewport.height - height) / 2,
      zoom: normalizedZoom,
    };
  }

  function graphViewBoxString(zoom) {
    const viewBox = graphViewBoxForZoom(zoom);
    return `${viewBox.x} ${viewBox.y} ${viewBox.width} ${viewBox.height}`;
  }

  function updateGraphZoomControls() {
    const viewBox = graphViewBoxForZoom(state.graphZoom);
    state.graphZoom = viewBox.zoom;
    setText("graphZoomValue", `${Math.round(viewBox.zoom * 100)}%`);
    setOptionalDisabled("zoomInGraphButton", viewBox.zoom >= GRAPH_ZOOM_MAX);
    setOptionalDisabled("zoomOutGraphButton", viewBox.zoom <= GRAPH_ZOOM_MIN);
    setOptionalDisabled("fitGraphButton", viewBox.zoom === 1);
  }

  function setGraphZoom(zoom) {
    const viewBox = graphViewBoxForZoom(zoom);
    state.graphZoom = viewBox.zoom;
    const svg = $("graphCanvas");
    svg?.setAttribute("viewBox", graphViewBoxString(viewBox.zoom));
    updateGraphZoomControls();
  }

  function zoomGraphView(direction) {
    setGraphZoom(state.graphZoom * (direction > 0 ? GRAPH_ZOOM_STEP : 1 / GRAPH_ZOOM_STEP));
    log("Updated graph zoom.", { zoom: state.graphZoom });
  }

  function graphFullscreenSupported() {
    return Boolean($("graphWorkspace")?.requestFullscreen && document.exitFullscreen);
  }

  function updateGraphFullscreenControl() {
    const button = $("fullscreenGraphButton");
    if (!button) return;
    const isFullscreen = document.fullscreenElement === $("graphWorkspace");
    button.disabled = !graphFullscreenSupported();
    button.setAttribute("aria-pressed", String(isFullscreen));
    button.setAttribute("aria-label", isFullscreen ? "Exit full screen" : "Enter full screen");
    button.setAttribute("title", isFullscreen ? "Exit full screen" : "Enter full screen");
  }

  async function toggleGraphFullscreen() {
    const workspace = $("graphWorkspace");
    if (!graphFullscreenSupported() || !workspace) return;
    try {
      if (document.fullscreenElement === workspace) await document.exitFullscreen();
      else await workspace.requestFullscreen();
    } catch (error) {
      state.lastError = "Full screen is unavailable in this browser.";
      log("Failed to change graph full screen state.", { error: error.message });
    } finally {
      updateGraphFullscreenControl();
    }
  }

  function drawGraph(graph, options = {}) {
    const svg = $("graphCanvas");
    const emptyState = $("graphEmptyState");
    svg.replaceChildren();
    svg.classList.remove("is-hovering");
    setGraphZoom(state.graphZoom);
    if (!graph.nodes.length) {
      if (emptyState) {
        const copy = graphEmptyStateCopy(options);
        setText("graphEmptyTitle", copy.title);
        setText("graphEmptyCopy", copy.copy);
        emptyState.hidden = false;
      }
      return;
    }
    if (emptyState) emptyState.hidden = true;
    const positions = graphLayout(graph);
    const edgeDegree = new Map(graph.nodes.map((node) => [node.id, 0]));
    for (const edge of graph.edges) {
      edgeDegree.set(edge.source, (edgeDegree.get(edge.source) || 0) + 1);
      edgeDegree.set(edge.target, (edgeDegree.get(edge.target) || 0) + 1);
    }
    for (const edge of graph.edges) {
      const source = positions.get(edge.source);
      const target = positions.get(edge.target);
      if (!source || !target) continue;
      const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
      line.setAttribute("class", "edge");
      line.dataset.source = edge.source;
      line.dataset.target = edge.target;
      line.setAttribute("x1", String(source.x));
      line.setAttribute("y1", String(source.y));
      line.setAttribute("x2", String(target.x));
      line.setAttribute("y2", String(target.y));
      svg.appendChild(line);
    }
    for (const node of graph.nodes) {
      const position = positions.get(node.id);
      const circle = document.createElementNS("http://www.w3.org/2000/svg", "circle");
      const degree = edgeDegree.get(node.id) || 0;
      const isSelected =
        state.selectedPageKey && node.id === state.selectedPageKey;
      circle.setAttribute(
        "class",
        `node${degree > 1 ? " focus" : ""}${isSelected ? " selected" : ""}`
      );
      circle.dataset.baseClass = circle.getAttribute("class");
      circle.dataset.nodeId = node.id;
      circle.setAttribute("cx", String(position.x));
      circle.setAttribute("cy", String(position.y));
      circle.setAttribute("r", String(Math.min(4.9, 2.15 + Math.sqrt(degree) * 0.52)));
      circle.setAttribute("data-folder-id", node.folderId);
      circle.addEventListener("mouseenter", () => setGraphHover(svg, graph, node.id));
      circle.addEventListener("mouseleave", () => clearGraphHover(svg));
      svg.appendChild(circle);

      const label = document.createElementNS("http://www.w3.org/2000/svg", "text");
      label.setAttribute("class", `node-label${isSelected ? " selected" : ""}`);
      label.dataset.baseClass = label.getAttribute("class");
      label.dataset.nodeId = node.id;
      label.setAttribute("x", String(position.x + 11));
      label.setAttribute("y", String(position.y + 3.5));
      label.textContent = node.title;
      svg.appendChild(label);
    }
  }

  function setGraphHover(svg, graph, nodeId) {
    const neighbors = graphNeighborIds(graph, nodeId);
    svg.classList.add("is-hovering");
    for (const edge of svg.querySelectorAll(".edge")) {
      const connected = edge.dataset.source === nodeId || edge.dataset.target === nodeId;
      edge.className.baseVal = `edge${connected ? " hover-connected" : " hover-faded"}`;
    }
    for (const node of svg.querySelectorAll(".node")) {
      const id = node.dataset.nodeId;
      const baseClass = node.dataset.baseClass || "node";
      const hoverClass =
        id === nodeId ? " hover-active" : neighbors.has(id) ? " hover-connected" : " hover-faded";
      node.className.baseVal = `${baseClass}${hoverClass}`;
    }
    for (const label of svg.querySelectorAll(".node-label")) {
      const id = label.dataset.nodeId;
      const baseClass = label.dataset.baseClass || "node-label";
      const labelClass =
        id === nodeId ? " hover-active" : neighbors.has(id) ? " hover-connected" : "";
      label.className.baseVal = `${baseClass}${labelClass}`;
    }
  }

  function clearGraphHover(svg) {
    svg.classList.remove("is-hovering");
    for (const edge of svg.querySelectorAll(".edge")) edge.className.baseVal = "edge";
    for (const node of svg.querySelectorAll(".node")) {
      node.className.baseVal = node.dataset.baseClass || "node";
    }
    for (const label of svg.querySelectorAll(".node-label")) {
      label.className.baseVal = label.dataset.baseClass || "node-label";
    }
  }

  function setPill(id, text, tone) {
    const element = $(id);
    if (!element) return;
    element.textContent = text;
    element.className = `pill ${tone || "muted"}`;
  }

  function openedGrantFolderKeys() {
    return new Set(
      (state.keyring?.openedGrants || []).map((grant) =>
        folderKeyVersionKey(grant.folderId, grant.keyVersion)
      )
    );
  }

  function appendAccessBadges(parent, badges) {
    if (!badges.length) return;
    const row = document.createElement("span");
    row.className = "access-badge-row";
    for (const badge of badges) {
      const element = document.createElement("span");
      element.className = `access-badge ${badge.tone || "muted"}`;
      element.textContent = badge.label;
      row.appendChild(element);
    }
    parent.appendChild(row);
  }

  function setText(id, text) {
    const element = $(id);
    if (element) element.textContent = text;
  }

  function setPressed(id, pressed) {
    const element = $(id);
    if (element) element.setAttribute("aria-pressed", String(Boolean(pressed)));
  }

  function appendFormattedText(parent, text) {
    const source = String(text || "");
    const pattern = /`([^`]+)`|\*\*([^*]+)\*\*|__([^_]+)__|~~([^~]+)~~|\*([^*]+)\*|_([^_]+)_/g;
    let cursor = 0;
    for (const match of source.matchAll(pattern)) {
      if (match.index > cursor) {
        parent.appendChild(document.createTextNode(source.slice(cursor, match.index)));
      }
      if (match[1]) {
        const code = document.createElement("code");
        code.textContent = match[1];
        parent.appendChild(code);
      } else if (match[2] || match[3]) {
        const strong = document.createElement("strong");
        strong.textContent = match[2] || match[3];
        parent.appendChild(strong);
      } else if (match[4]) {
        const strike = document.createElement("del");
        strike.textContent = match[4];
        parent.appendChild(strike);
      } else if (match[5] || match[6]) {
        const emphasis = document.createElement("em");
        emphasis.textContent = match[5] || match[6];
        parent.appendChild(emphasis);
      }
      cursor = match.index + match[0].length;
    }
    if (cursor < source.length) parent.appendChild(document.createTextNode(source.slice(cursor)));
  }

  function appendInlineSegments(parent, text) {
    for (const segment of inlineLinkSegments(text)) {
      if (segment.kind === "text") {
        appendFormattedText(parent, segment.text);
        continue;
      }
      const link = document.createElement("span");
      link.className = segment.kind === "external" ? "external-link" : "internal-link";
      appendFormattedText(link, segment.text || segment.target);
      if (segment.target) link.dataset.target = segment.target;
      link.tabIndex = 0;
      link.setAttribute?.("role", "link");
      parent.appendChild(link);
    }
  }

  function renderMarkdownPreview(container, markdown, options = {}) {
    container.replaceChildren();
    let taskIndex = 0;
    for (const block of markdownPreviewBlocks(markdown)) {
      if (block.type === "heading") {
        const heading = document.createElement(`h${block.level}`);
        appendInlineSegments(heading, block.text);
        container.appendChild(heading);
        continue;
      }
      if (block.type === "list") {
        const list = document.createElement(block.ordered ? "ol" : "ul");
        if (block.ordered && block.start && block.start !== 1) list.start = block.start;
        if (!block.ordered && block.items.some((item) => item.checked !== null)) {
          list.className = "task-list";
        }
        for (const itemBlock of block.items) {
          const item = document.createElement("li");
          if (itemBlock.checked !== null) {
            item.className = "task-list-item";
            const checkbox = document.createElement("input");
            checkbox.type = "checkbox";
            checkbox.checked = Boolean(itemBlock.checked);
            checkbox.disabled = !options.editable;
            checkbox.dataset.taskIndex = String(taskIndex);
            checkbox.dataset.taskChecked = String(Boolean(itemBlock.checked));
            checkbox.setAttribute?.("aria-label", taskCheckboxAriaLabel(itemBlock.text, checkbox.checked));
            checkbox.setAttribute?.("contenteditable", "false");
            taskIndex += 1;
            item.appendChild(checkbox);
          }
          appendInlineSegments(item, itemBlock.text);
          list.appendChild(item);
        }
        container.appendChild(list);
        continue;
      }
      if (block.type === "quote") {
        const quote = document.createElement("blockquote");
        appendInlineSegments(quote, block.text);
        container.appendChild(quote);
        continue;
      }
      if (block.type === "code") {
        const pre = document.createElement("pre");
        pre.className = "code-block";
        if (block.language) {
          pre.dataset.language = block.language;
          pre.setAttribute?.("data-language", block.language);
        }
        const code = document.createElement("code");
        if (block.language) code.className = `language-${block.language}`;
        code.textContent = block.text;
        pre.appendChild(code);
        container.appendChild(pre);
        continue;
      }
      if (block.type === "table") {
        const table = document.createElement("table");
        const thead = document.createElement("thead");
        const headerRow = document.createElement("tr");
        block.headers.forEach((header, index) => {
          const cell = document.createElement("th");
          if (block.alignments[index] && cell.style) cell.style.textAlign = block.alignments[index];
          appendInlineSegments(cell, header);
          headerRow.appendChild(cell);
        });
        thead.appendChild(headerRow);
        table.appendChild(thead);
        const tbody = document.createElement("tbody");
        for (const row of block.rows) {
          const tableRow = document.createElement("tr");
          row.forEach((value, index) => {
            const cell = document.createElement("td");
            if (block.alignments[index] && cell.style) cell.style.textAlign = block.alignments[index];
            appendInlineSegments(cell, value);
            tableRow.appendChild(cell);
          });
          tbody.appendChild(tableRow);
        }
        table.appendChild(tbody);
        container.appendChild(table);
        continue;
      }
      if (block.type === "rule") {
        container.appendChild(document.createElement("hr"));
        continue;
      }
      const paragraph = document.createElement("p");
      appendInlineSegments(paragraph, block.text);
      container.appendChild(paragraph);
    }
  }

  function renderMarkdownEditor(container, markdown, options = {}) {
    renderMarkdownPreview(container, markdown, options);
    if (!container.childNodes.length) {
      const paragraph = document.createElement("p");
      paragraph.appendChild(document.createElement("br"));
      container.appendChild(paragraph);
    }
  }

  function escapeMarkdownCode(value) {
    return String(value || "").replaceAll("`", "\\`");
  }

  function inlineMarkdownFromEditorNode(node) {
    if (!node) return "";
    if (node.nodeType === 3) return String(node.nodeValue || "").replace(/\u00a0/g, " ");
    if (node.nodeType !== 1) return "";
    const tag = String(node.tagName || "").toLowerCase();
    if (tag === "br") return "\n";
    if (tag === "input") return "";
    const text = Array.from(node.childNodes || []).map(inlineMarkdownFromEditorNode).join("");
    if (!text && tag !== "img") return "";
    if (tag === "strong" || tag === "b") return `**${text}**`;
    if (tag === "em" || tag === "i") return `*${text}*`;
    if (tag === "del" || tag === "s") return `~~${text}~~`;
    if (tag === "code") return `\`${escapeMarkdownCode(text)}\``;
    if (tag === "a") {
      const target = node.getAttribute?.("href") || node.dataset?.target || "";
      return target ? `[${text || target}](${target})` : text;
    }
    const className = String(node.className || "");
    if (className.includes("internal-link") && node.dataset?.target) {
      return text && text !== node.dataset.target
        ? `[[${node.dataset.target}|${text}]]`
        : `[[${node.dataset.target}]]`;
    }
    if (className.includes("external-link") && node.dataset?.target) {
      return `[${text || node.dataset.target}](${node.dataset.target})`;
    }
    return text;
  }

  function markdownTableCellFromNode(node) {
    return inlineMarkdownFromEditorNode(node)
      .replace(/\n+/g, " ")
      .replaceAll("|", "\\|")
      .trim();
  }

  function tableRowsFromSection(section, cellTag) {
    const rows = [];
    for (const row of Array.from(section?.children || [])) {
      const cells = Array.from(row.children || []).filter(
        (cell) => String(cell.tagName || "").toLowerCase() === cellTag
      );
      if (cells.length) rows.push(cells.map(markdownTableCellFromNode));
    }
    return rows;
  }

  function tableMarkdownFromEditorNode(node) {
    const sections = Array.from(node.children || []);
    const head = sections.find((child) => String(child.tagName || "").toLowerCase() === "thead");
    const body = sections.find((child) => String(child.tagName || "").toLowerCase() === "tbody");
    let headers = tableRowsFromSection(head, "th")[0] || [];
    let rows = tableRowsFromSection(body, "td");
    if (!headers.length && rows.length) {
      headers = rows.shift();
    }
    if (!headers.length) return "";
    const width = Math.max(headers.length, ...rows.map((row) => row.length));
    const paddedHeaders = normalizeMarkdownTableRow(headers, width);
    const paddedRows = rows.map((row) => normalizeMarkdownTableRow(row, width));
    return [
      `| ${paddedHeaders.join(" | ")} |`,
      `| ${Array.from({ length: width }, () => "---").join(" | ")} |`,
      ...paddedRows.map((row) => `| ${row.join(" | ")} |`),
    ].join("\n");
  }

  function editorBlockMarkdown(node) {
    if (!node) return "";
    if (node.nodeType === 3) return String(node.nodeValue || "").trim();
    if (node.nodeType !== 1) return "";
    const tag = String(node.tagName || "").toLowerCase();
    if (/^h[1-6]$/.test(tag)) {
      return `${"#".repeat(Number(tag.slice(1)))} ${inlineMarkdownFromEditorNode(node).trim()}`;
    }
    if (tag === "ul" || tag === "ol") {
      return Array.from(node.children || [])
        .filter((child) => String(child.tagName || "").toLowerCase() === "li")
        .map((child, index) => {
          const checkbox = Array.from(child.children || []).find(
            (candidate) => String(candidate.tagName || "").toLowerCase() === "input"
          );
          const text = inlineMarkdownFromEditorNode(child).trim();
          if (!text) return "";
          if (checkbox) return `- [${checkbox.checked ? "x" : " "}] ${text}`;
          return tag === "ol" ? `${index + 1}. ${text}` : `- ${text}`;
        })
        .filter(Boolean)
        .join("\n");
    }
    if (tag === "blockquote") {
      const quote = inlineMarkdownFromEditorNode(node).trim();
      return quote
        .split("\n")
        .filter(Boolean)
        .map((line) => `> ${line}`)
        .join("\n");
    }
    if (tag === "pre") {
      const code = normalizeCodeBlockText(String(node.textContent || "").replace(/\n$/g, ""));
      const language = String(node.dataset?.language || node.getAttribute?.("data-language") || "").trim();
      return `\`\`\`${language}\n${code}\n\`\`\``;
    }
    if (tag === "table") return tableMarkdownFromEditorNode(node);
    if (tag === "hr") return "---";
    return inlineMarkdownFromEditorNode(node).trim();
  }

  function markdownFromEditorElement(editor) {
    const blocks = Array.from(editor?.childNodes || [])
      .map(editorBlockMarkdown)
      .map((block) => block.trim())
      .filter(Boolean);
    if (blocks.length) return blocks.join("\n\n");
    return String(editor?.textContent || "").trim();
  }

  function toggleMarkdownTask(markdown, taskIndex, checked) {
    const source = String(markdown ?? "");
    const lineEnding = source.includes("\r\n") ? "\r\n" : "\n";
    const selectedIndex = Number(taskIndex);
    const taskSourceLines = [];
    for (const block of markdownPreviewBlocks(source, { includeSourcePositions: true })) {
      if (block.type !== "list") continue;
      for (const item of block.items) {
        if (item.checked !== null) taskSourceLines.push(item.sourceLineIndex);
      }
    }
    const sourceLineIndex = taskSourceLines[selectedIndex];
    if (!Number.isInteger(sourceLineIndex)) return source;
    const lines = source.split(/\r?\n/);
    const task = lines[sourceLineIndex]?.match(/^(\s*[-*+]\s+\[)[ xX](\]\s+.+)$/);
    if (!task) return source;
    lines[sourceLineIndex] = `${task[1]}${checked ? "x" : " "}${task[2]}`;
    return lines.join(lineEnding);
  }

  function taskCheckboxAriaLabel(taskText, checked) {
    const text = String(taskText || "Task").replace(/\s+/g, " ").trim() || "Task";
    return `${checked ? "Mark task incomplete" : "Mark task complete"}: ${text}`;
  }

  function setEditorDraftText(markdown, options = {}) {
    const draft = $("pageDraftInput");
    if (draft) draft.value = markdown;
    if (options.syncVisual && state.editorMode === "visual") {
      renderMarkdownEditor(visualEditorElement(), markdown, { editable: true });
    }
    updateEditorChrome();
  }

  function invalidatePreparedWrite() {
    state.preparedWrite = null;
    state.preparedWriteTarget = null;
    updateSaveControls();
  }

  function activePageKeyFromInputs() {
    const folderId = $("pageFolderIdInput")?.value.trim() || state.selectedFolderId || DEFAULT_CLIENT_FOLDER_ID;
    const objectId = $("pageObjectIdInput")?.value.trim() || selectedReaderPage()?.objectId || nextDraftObjectId();
    return { folderId, objectId, key: pageKey(folderId, objectId) };
  }

  function activeLocalDraft() {
    return state.projection.localDrafts.get(activePageKeyFromInputs().key) || null;
  }

  function canSaveActiveDraft() {
    return Boolean(
      !state.pageSaveInFlight && state.signerStatus === "connected" && state.keyring && activeLocalDraft()
    );
  }

  function updateSaveControls() {
    const canSave = canSaveActiveDraft();
    setOptionalDisabled("savePageButton", !canSave);
    return canSave;
  }

  function rememberActiveDraft(markdown) {
    const { folderId, objectId, key } = activePageKeyFromInputs();
    const baseRevision = Number($("pageBaseRevisionInput")?.value.trim() || 0) || 0;
    const existingDraft = state.projection.localDrafts.get(key);
    const existingPage = state.projection.pages.get(key);
    state.projection.localDrafts.set(key, {
      baseRevision,
      path: existingDraft?.path || existingPage?.path || `${objectId}.md`,
      text: markdown,
    });
    const draft = $("pageDraftInput");
    if (draft) draft.value = markdown;
    invalidatePreparedWrite();
    updateEditorChrome();
  }

  function syncDraftFromVisualEditor(options = {}) {
    const editor = visualEditorElement();
    const markdown = markdownFromEditorElement(editor);
    const draft = $("pageDraftInput");
    if (draft) draft.value = markdown;
    if (options.remember) rememberActiveDraft(markdown);
    updateEditorChrome();
    return markdown;
  }

  function updateActiveTaskDraft(taskCheckbox) {
    const editor = visualEditorElement();
    const restoreInitialState = () => {
      if (taskCheckbox) taskCheckbox.checked = taskCheckbox.dataset?.taskChecked === "true";
    };
    if (
      !taskCheckbox ||
      taskCheckbox.type !== "checkbox" ||
      taskCheckbox.disabled ||
      !editor?.contains?.(taskCheckbox) ||
      editor.getAttribute?.("contenteditable") !== "true"
    ) {
      restoreInitialState();
      return false;
    }
    const taskIndex = Array.from(editor.querySelectorAll?.("input[data-task-index]") || []).indexOf(taskCheckbox);
    if (taskIndex < 0) {
      restoreInitialState();
      return false;
    }
    const currentMarkdown = $("pageDraftInput")?.value || "";
    const markdown = toggleMarkdownTask(currentMarkdown, taskIndex, taskCheckbox.checked);
    if (markdown !== currentMarkdown) rememberActiveDraft(markdown);
    taskCheckbox.dataset.taskChecked = String(Boolean(taskCheckbox.checked));
    taskCheckbox.setAttribute?.(
      "aria-label",
      taskCheckboxAriaLabel(taskCheckbox.closest?.("li")?.textContent || "Task", taskCheckbox.checked)
    );
    return true;
  }

  function setEditorMode(mode) {
    state.editorMode = mode === "markdown" ? "markdown" : "visual";
    if (state.editorMode === "markdown") {
      syncDraftFromVisualEditor();
      setPageContentEditable(visualEditorElement(), false);
    } else {
      renderMarkdownEditor(visualEditorElement(), $("pageDraftInput").value, { editable: true });
      if (isReadablePage(selectedReaderPage())) {
        setPageContentEditable(visualEditorElement(), true);
      }
    }
    updateEditorChrome();
  }

  function updateEditorChrome() {
    const markdownEditor = $("pageMarkdownEditorLabel");
    const page = selectedReaderPage();
    const canEditInline = isReadablePage(page) && state.editorMode === "visual";
    if (markdownEditor) markdownEditor.hidden = state.editorMode !== "markdown";
    let statusText = "Reading mode";
    if (!isReadablePage(page)) {
      statusText = "No page loaded";
    } else if (state.editorMode === "markdown") {
      statusText = "Markdown editor";
    } else if (canEditInline) {
      statusText = "Click to edit inline";
    }
    setText("editorStatusText", statusText);
    updateSaveControls();
  }

  function selectedTextForEditor() {
    return String(window.getSelection?.()?.toString?.() || "");
  }

  function editorSlashCommandRows(query) {
    const needle = String(query || "").trim().toLowerCase();
    return EDITOR_SLASH_COMMANDS.filter((command) => {
      if (!needle) return true;
      const haystack = [command.id, command.label, command.detail, ...(command.aliases || [])]
        .join(" ")
        .toLowerCase();
      return haystack.includes(needle);
    });
  }

  function escapeHtml(value) {
    return String(value || "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;");
  }

  function editorBlockForNode(node) {
    const editor = visualEditorElement();
    let current = node?.nodeType === 1 ? node : node?.parentElement;
    while (current && current !== editor) {
      const tag = String(current.tagName || "").toLowerCase();
      if (/^(p|h[1-6]|li|blockquote|pre|td|th)$/.test(tag)) return current;
      current = current.parentElement;
    }
    return editor;
  }

  function textOffsetRange(root, startOffset, endOffset) {
    const range = document.createRange?.();
    if (!range || !root) return null;
    let cursor = 0;
    let startSet = false;
    let endSet = false;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      const next = cursor + String(node.nodeValue || "").length;
      if (!startSet && startOffset <= next) {
        range.setStart(node, Math.max(0, startOffset - cursor));
        startSet = true;
      }
      if (!endSet && endOffset <= next) {
        range.setEnd(node, Math.max(0, endOffset - cursor));
        endSet = true;
        break;
      }
      cursor = next;
    }
    if (!startSet) range.setStart(root, root.childNodes.length);
    if (!endSet) range.setEnd(root, root.childNodes.length);
    return range;
  }

  function slashMenuRectForRange(range) {
    const editorRect = visualEditorElement()?.getBoundingClientRect?.();
    const collapsed = range?.cloneRange?.();
    if (collapsed) {
      collapsed.collapse(false);
      const caretRect = collapsed.getBoundingClientRect?.();
      if (caretRect && (caretRect.width || caretRect.height)) return caretRect;
      const tokenRect = range.getBoundingClientRect?.();
      if (tokenRect && (tokenRect.width || tokenRect.height)) return tokenRect;
    }
    return editorRect || { bottom: 96, left: 96 };
  }

  function currentEditorSlashContext() {
    const editor = visualEditorElement();
    if (!editor || editor.getAttribute?.("contenteditable") !== "true") return null;
    const selection = window.getSelection?.();
    if (!selection || !selection.rangeCount || !selection.isCollapsed) return null;
    if (!editor.contains(selection.anchorNode)) return null;
    const block = editorBlockForNode(selection.anchorNode);
    if (!block) return null;
    const beforeRange = document.createRange?.();
    if (!beforeRange) return null;
    beforeRange.selectNodeContents(block);
    try {
      beforeRange.setEnd(selection.anchorNode, selection.anchorOffset);
    } catch (_error) {
      return null;
    }
    const beforeText = beforeRange.toString();
    const match = beforeText.match(/(^|[\s\u00a0])\/([A-Za-z0-9_-]*)$/);
    if (!match) return null;
    const query = match[2] || "";
    const startOffset = beforeText.length - query.length - 1;
    const tokenRange = textOffsetRange(block, startOffset, beforeText.length);
    if (!tokenRange) return null;
    return {
      query,
      range: tokenRange,
      rect: slashMenuRectForRange(tokenRange),
      rows: editorSlashCommandRows(query),
    };
  }

  function closeEditorSlashMenu() {
    state.editorSlashOpen = false;
    state.editorSlashQuery = "";
    state.editorSlashRange = null;
    state.editorSlashSelectedIndex = 0;
    const menu = $("editorSlashMenu");
    if (menu) {
      menu.hidden = true;
      menu.replaceChildren();
    }
  }

  function renderEditorSlashMenu(context) {
    const menu = $("editorSlashMenu");
    if (!menu || !context || !context.rows.length) {
      closeEditorSlashMenu();
      return;
    }
    state.editorSlashOpen = true;
    state.editorSlashQuery = context.query;
    state.editorSlashRange = context.range.cloneRange?.() || context.range;
    state.editorSlashSelectedIndex = Math.min(
      Math.max(0, state.editorSlashSelectedIndex),
      Math.max(0, context.rows.length - 1)
    );
    menu.hidden = false;
    menu.replaceChildren();
    menu.setAttribute("aria-activedescendant", `editorSlashCommand-${context.rows[state.editorSlashSelectedIndex].id}`);
    for (const [index, command] of context.rows.entries()) {
      const button = document.createElement("button");
      button.id = `editorSlashCommand-${command.id}`;
      button.type = "button";
      button.className = `editor-slash-row${index === state.editorSlashSelectedIndex ? " active" : ""}`;
      button.setAttribute("role", "option");
      button.setAttribute("aria-selected", String(index === state.editorSlashSelectedIndex));
      button.dataset.editorSlashCommand = command.id;
      const label = document.createElement("strong");
      label.textContent = command.label;
      const detail = document.createElement("span");
      detail.textContent = command.detail;
      button.append(label, detail);
      button.addEventListener("mousedown", (event) => event.preventDefault());
      button.addEventListener("click", () => applyEditorSlashCommand(command.id));
      menu.appendChild(button);
    }
    menu.querySelector(".editor-slash-row.active")?.scrollIntoView?.({ block: "nearest" });
    const menuWidth = Math.min(280, Math.max(220, menu.offsetWidth || 260));
    const left = Math.max(8, Math.min(context.rect.left, window.innerWidth - menuWidth - 8));
    const top = Math.max(8, Math.min(context.rect.bottom + 8, window.innerHeight - 280));
    menu.style.left = `${left}px`;
    menu.style.top = `${top}px`;
  }

  function refreshEditorSlashMenu() {
    const context = currentEditorSlashContext();
    if (!context) {
      closeEditorSlashMenu();
      return;
    }
    if (context.query !== state.editorSlashQuery) state.editorSlashSelectedIndex = 0;
    renderEditorSlashMenu(context);
  }

  function applyEditorSlashCommand(command) {
    const range = state.editorSlashRange?.cloneRange?.();
    closeEditorSlashMenu();
    const editor = visualEditorElement();
    editor?.focus?.();
    if (range) {
      const selection = window.getSelection?.();
      selection?.removeAllRanges?.();
      range.deleteContents();
      range.collapse(true);
      selection?.addRange?.(range);
    }
    runEditorCommand(command);
  }

  function handleEditorSlashKeydown(event) {
    if (!state.editorSlashOpen) return false;
    const rows = editorSlashCommandRows(state.editorSlashQuery);
    if (!rows.length) {
      closeEditorSlashMenu();
      return false;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      state.editorSlashSelectedIndex = (state.editorSlashSelectedIndex + 1) % rows.length;
      renderEditorSlashMenu({ query: state.editorSlashQuery, range: state.editorSlashRange, rect: slashMenuRectForRange(state.editorSlashRange), rows });
      return true;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      state.editorSlashSelectedIndex = (state.editorSlashSelectedIndex - 1 + rows.length) % rows.length;
      renderEditorSlashMenu({ query: state.editorSlashQuery, range: state.editorSlashRange, rect: slashMenuRectForRange(state.editorSlashRange), rows });
      return true;
    }
    if (event.key === "Enter" || event.key === "Tab") {
      event.preventDefault();
      applyEditorSlashCommand(rows[state.editorSlashSelectedIndex]?.id);
      return true;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      closeEditorSlashMenu();
      return true;
    }
    return false;
  }

  function runEditorCommand(command) {
    if (!command) return;
    if (state.editorMode !== "visual") setEditorMode("visual");
    const editor = visualEditorElement();
    editor.focus?.();
    const exec = (name, value = null) => document.execCommand?.(name, false, value);
    if (command === "paragraph") exec("formatBlock", "p");
    if (command === "heading1") exec("formatBlock", "h1");
    if (command === "heading2") exec("formatBlock", "h2");
    if (command === "bold") exec("bold");
    if (command === "italic") exec("italic");
    if (command === "bullet") exec("insertUnorderedList");
    if (command === "quote") exec("formatBlock", "blockquote");
    if (command === "codeblock") exec("formatBlock", "pre");
    if (command === "rule") exec("insertHorizontalRule");
    if (command === "code") {
      exec("insertHTML", `<code>${escapeHtml(selectedTextForEditor() || "code")}</code>`);
    }
    if (command === "link") {
      const target = window.prompt?.("Link target")?.trim();
      if (target) exec("createLink", target);
    }
    syncDraftFromVisualEditor({ remember: true });
  }

  function setNoteEmptyState(isEmpty) {
    $("readerPageContent").className = isEmpty ? "note-content note-content-empty" : "note-content";
  }

  function setPageContentEditable(content, enabled) {
    if (!content) return;
    if (enabled) {
      content.setAttribute("contenteditable", "true");
      content.setAttribute("spellcheck", "true");
      content.setAttribute("role", "textbox");
      content.setAttribute("aria-label", "Page editor");
      content.setAttribute("aria-multiline", "true");
      return;
    }
    content.removeAttribute("contenteditable");
    content.removeAttribute("spellcheck");
    content.removeAttribute("role");
    content.removeAttribute("aria-label");
    content.removeAttribute("aria-multiline");
  }

  function highlightReaderSearchMatches(container, query) {
    if (!container || !query) return [];

    const textNodes = [];
    const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      if (node.parentElement?.closest?.(".reader-search-match")) continue;
      const segments = searchHighlightSegments(node.nodeValue, query);
      if (segments.some((segment) => segment.match)) textNodes.push({ node, segments });
    }

    const matches = [];
    for (const { node, segments } of textNodes) {
      const fragment = document.createDocumentFragment();
      for (const segment of segments) {
        if (!segment.match) {
          fragment.appendChild(document.createTextNode(segment.text));
          continue;
        }
        const match = document.createElement("mark");
        match.className = "reader-search-match";
        match.textContent = segment.text;
        fragment.appendChild(match);
        matches.push(match);
      }
      node.replaceWith(fragment);
    }
    return matches;
  }

  function scrollReaderSearchMatchIntoView(match) {
    if (!match) return;
    const behavior = window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches ? "auto" : "smooth";
    const scroll = () => match.scrollIntoView?.({ behavior, block: "center", inline: "nearest" });
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(scroll);
    else scroll();
  }

  function renderPageContent(page) {
    const content = $("readerPageContent");
    content.replaceChildren();
    setPageContentEditable(content, false);
    if (!page) {
      content.className = "note-content note-content-empty";
      content.textContent =
        state.sessionStatus === SESSION_STATUS.UNLOCKED
          ? "Open a vault to read pages."
          : "Session locked. Unlock to reopen encrypted Folder Key Grants.";
      return;
    }
    if (!isReadablePage(page)) {
      content.className = "note-content note-content-empty";
      content.textContent = "This page is locked in this session.";
      return;
    }
    content.className = `note-content note-markdown inline-page-editor${page.localDraft ? " inline-page-editor-dirty" : ""}`;
    setPageContentEditable(content, state.editorMode === "visual");
    renderMarkdownEditor(content, page.text || "", { editable: state.editorMode === "visual" });
    const searchQuery = readerSearchHighlightForPage(page.key, state.searchHighlight);
    if (!searchQuery) return;
    const matches = highlightReaderSearchMatches(content, searchQuery);
    if (state.searchHighlightShouldScroll) {
      state.searchHighlightShouldScroll = false;
      scrollReaderSearchMatchIntoView(matches[0]);
    }
  }

  function renderPageStatus(page) {
    return page;
  }

  function renderLinkContext(page) {
    return page;
  }

  function setGraphStats(graph) {
    const stats = graphStats(graph);
    setPill(
      "graphStats",
      `${stats.nodeCount} ${stats.nodeCount === 1 ? "node" : "nodes"} / ${stats.edgeCount} ${
        stats.edgeCount === 1 ? "link" : "links"
      }`,
      stats.nodeCount ? "ready" : "muted"
    );
  }

  function setList(id, rows, emptyText, renderRow) {
    const list = $(id);
    if (!list) return;
    list.replaceChildren();
    if (!rows.length) {
      const item = document.createElement("li");
      item.className = "empty-row";
      item.textContent = emptyText;
      list.appendChild(item);
      return;
    }
    for (const row of rows) {
      const item = document.createElement("li");
      renderRow(item, row);
      list.appendChild(item);
    }
  }

  function keyboardListNavigationIndex(key, currentIndex, itemCount) {
    const count = Number.isInteger(itemCount) ? itemCount : 0;
    if (count < 1) return null;
    const hasCurrentIndex = Number.isInteger(currentIndex) && currentIndex >= 0 && currentIndex < count;
    if (!hasCurrentIndex) {
      if (key === "ArrowDown" || key === "Home") return 0;
      if (key === "ArrowUp" || key === "End") return count - 1;
      return null;
    }
    const current = Math.min(Math.max(Number.isInteger(currentIndex) ? currentIndex : 0, 0), count - 1);
    if (key === "ArrowDown") return (current + 1) % count;
    if (key === "ArrowUp") return (current - 1 + count) % count;
    if (key === "Home") return 0;
    if (key === "End") return count - 1;
    return null;
  }

  function commandPaletteSelectionIndex(rows, currentIndex = state.commandPaletteSelectedIndex) {
    const count = Array.isArray(rows) ? rows.length : 0;
    if (count < 1) return -1;
    return Math.min(Math.max(Number.isInteger(currentIndex) ? currentIndex : 0, 0), count - 1);
  }

  function primaryFormActionForInput(inputId) {
    return (
      {
        accessShareTargetInput: "createShareLinkButton",
        accessShareExpiresAtInput: "createShareLinkButton",
        accessShareLinkInput: "acceptShareLinkButton",
        vaultInviteTargetNpubInput: "createVaultInvitationButton",
        vaultInviteFoldersInput: "createVaultInvitationButton",
        vaultInviteExpiresAtInput: "createVaultInvitationButton",
        vaultInviteCodeInput: "getVaultInvitationButton",
        vaultInviteEmailInput: "getEmailInviteInstructionsButton",
        vaultInviteEmailProofCreatedAtInput: "getEmailInviteInstructionsButton",
        vaultInviteSecretInput: "getEmailInviteInstructionsButton",
      }[inputId] || null
    );
  }

  function shouldRunPrimaryFormAction(event, button) {
    return Boolean(
      event?.key === "Enter" &&
        !event.isComposing &&
        event.keyCode !== 229 &&
        !event.currentTarget?.disabled &&
        button &&
        !button.disabled
    );
  }

  function bindPrimaryFormAction(inputId) {
    const input = $(inputId);
    const buttonId = primaryFormActionForInput(inputId);
    if (!input || !buttonId) return;
    input.addEventListener("keydown", (event) => {
      const button = $(buttonId);
      if (!shouldRunPrimaryFormAction(event, button)) return;
      event.preventDefault();
      button.click();
    });
  }

  function log(message, _value) {
    // Event labels are useful during development, but values can contain
    // decrypted titles, paths, identity metadata, or invite material. Never
    // retain those objects in the browser console beyond Session Lock.
    console.debug(`[FiniteBrain] ${message}`);
  }

  function contextMenuFocusableElements() {
    const menu = $("contextMenu");
    if (!menu) return [];
    return Array.from(menu.querySelectorAll?.('button[role="menuitem"]:not([disabled])') || []);
  }

  function focusContextMenuItem(index = 0) {
    const items = contextMenuFocusableElements();
    if (!items.length) return;
    const nextIndex = Math.min(Math.max(index, 0), items.length - 1);
    items[nextIndex]?.focus?.();
  }

  function handleContextMenuKeydown(event) {
    const menu = $("contextMenu");
    if (!menu || menu.hidden || event.isComposing || event.keyCode === 229) return false;
    if (event.key === "Escape") {
      event.preventDefault();
      closeContextMenu({ restoreFocus: true });
      return true;
    }
    const items = contextMenuFocusableElements();
    const currentIndex = items.indexOf(document.activeElement);
    const nextIndex = keyboardListNavigationIndex(event.key, currentIndex, items.length);
    if (nextIndex !== null) {
      event.preventDefault();
      focusContextMenuItem(nextIndex);
      return true;
    }
    if (event.key !== "Enter" && event.key !== " " && event.key !== "Spacebar") return false;
    const item = items[currentIndex];
    if (!item) return false;
    event.preventDefault();
    item.click();
    return true;
  }

  function closeContextMenu(options = {}) {
    const previousFocus = state.contextMenuPreviousFocus;
    state.contextMenuTarget = null;
    state.contextMenuPreviousFocus = null;
    const menu = $("contextMenu");
    if (menu) {
      menu.hidden = true;
      menu.replaceChildren();
    }
    if (options.restoreFocus) previousFocus?.focus?.();
  }

  function commandPaletteFocusableElements() {
    return overlayFocusableElements("commandPalette");
  }

  function closeCommandPalette(options = {}) {
    state.commandPaletteOpen = false;
    state.commandPaletteSelectedIndex = 0;
    const palette = $("commandPalette");
    if (palette) {
      palette.hidden = true;
      const input = $("commandPaletteInput");
      input?.setAttribute("aria-expanded", "false");
      input?.removeAttribute("aria-activedescendant");
      setPressed("ribbonCommandButton", false);
      $("ribbonCommandButton").className = "ribbon-button";
    }
    if (options.restoreFocus) $("ribbonCommandButton")?.focus?.();
  }

  function handleCommandPaletteKeydown(event) {
    if (!state.commandPaletteOpen) return false;
    if (event.key === "Escape") {
      event.preventDefault();
      closeCommandPalette({ restoreFocus: true });
      return true;
    }
    if (event.key === "Tab") {
      const focusable = commandPaletteFocusableElements();
      if (!focusable.length) {
        event.preventDefault();
        return true;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
      return true;
    }
    if ((event.metaKey || event.ctrlKey) && ["p", "s"].includes(event.key.toLowerCase())) {
      event.preventDefault();
    }
    return true;
  }

  function runCommandPaletteRow(row) {
    if (!row) return;
    closeCommandPalette();
    if (row.kind === "page") {
      selectReaderPage(row.pageKey);
      return;
    }
    if (row.target === "files" || row.target === "search") {
      setSidebarMode(row.target);
      return;
    }
    if (row.target === "access") {
      openSettingsModal("access");
      return;
    }
    if (row.target === "graph") {
      setWorkspaceView("graph");
      return;
    }
    if (row.target === "new-page") {
      startNewPageDraft();
      return;
    }
    if (row.target === "refresh") {
      refreshReader().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to refresh Vault reader.", { error: error.message });
        state.readerBusy = false;
        render();
      });
    }
  }

  function renderCommandPalette() {
    const palette = $("commandPalette");
    if (!palette) return;
    palette.hidden = !state.commandPaletteOpen;
    setPressed("ribbonCommandButton", state.commandPaletteOpen);
    $("ribbonCommandButton").className = `ribbon-button${state.commandPaletteOpen ? " utility-active" : ""}`;
    if (!state.commandPaletteOpen) return;
    const list = $("commandPaletteList");
    const input = $("commandPaletteInput");
    const rows = commandPaletteRows(input.value);
    const selectedIndex = commandPaletteSelectionIndex(rows);
    state.commandPaletteSelectedIndex = Math.max(selectedIndex, 0);
    input.setAttribute("aria-expanded", "true");
    list.replaceChildren();
    if (!rows.length) {
      input.removeAttribute("aria-activedescendant");
      const item = document.createElement("li");
      item.className = "empty-row";
      item.textContent = "No matching commands or Pages";
      list.appendChild(item);
      return;
    }
    rows.forEach((row, index) => {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      button.id = `commandPaletteOption-${index}`;
      button.tabIndex = -1;
      button.setAttribute("role", "option");
      button.setAttribute("aria-selected", String(index === selectedIndex));
      button.className = `command-palette-row${index === selectedIndex ? " active" : ""}`;
      const copy = document.createElement("span");
      const title = document.createElement("span");
      title.className = "command-palette-row-title";
      title.textContent = row.label;
      const detail = document.createElement("span");
      detail.className = "command-palette-row-detail";
      detail.textContent = row.detail || "";
      copy.appendChild(title);
      copy.appendChild(detail);
      const kind = document.createElement("span");
      kind.className = "command-palette-row-kind";
      kind.textContent = row.kind;
      button.appendChild(copy);
      button.appendChild(kind);
      button.addEventListener("click", () => runCommandPaletteRow(row));
      item.appendChild(button);
      list.appendChild(item);
    });
    input.setAttribute("aria-activedescendant", `commandPaletteOption-${selectedIndex}`);
  }

  function openCommandPalette(seed = "") {
    state.commandPaletteOpen = true;
    state.commandPaletteSelectedIndex = 0;
    closeContextMenu();
    $("commandPaletteInput").value = seed;
    renderCommandPalette();
    if (typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => $("commandPaletteInput").focus());
    } else {
      $("commandPaletteInput").focus?.();
    }
  }

  function positionContextMenu(menu, x, y, itemCount) {
    const estimatedWidth = 240;
    const estimatedHeight = Math.max(40, itemCount * 34 + 14);
    const maxLeft = Math.max(8, window.innerWidth - estimatedWidth - 8);
    const maxTop = Math.max(8, window.innerHeight - estimatedHeight - 8);
    menu.style.left = `${Math.min(Math.max(8, x), maxLeft)}px`;
    menu.style.top = `${Math.min(Math.max(8, y), maxTop)}px`;
  }

  function clipboardFeedbackFor(kind) {
    if (kind === "folder-id") {
      return {
        failure: CLIENT_ACTION_FEEDBACK.folderIdCopyFailure,
        success: CLIENT_ACTION_FEEDBACK.folderIdCopySuccess,
      };
    }
    if (kind === "invite-link") {
      return {
        failure: CLIENT_ACTION_FEEDBACK.inviteLinkCopyFailure,
        success: CLIENT_ACTION_FEEDBACK.inviteLinkCopySuccess,
      };
    }
    return {
      failure: CLIENT_ACTION_FEEDBACK.pageIdCopyFailure,
      success: CLIENT_ACTION_FEEDBACK.pageIdCopySuccess,
    };
  }

  function clipboardFeedbackOperationIsCurrent(sessionEpoch, feedbackGeneration) {
    return (
      state.sessionEpoch === sessionEpoch &&
      state.sessionStatus === SESSION_STATUS.UNLOCKED &&
      state.clientActionFeedbackGeneration === feedbackGeneration
    );
  }

  async function copyToClipboard(text, kind = "page-id") {
    const feedback = clipboardFeedbackFor(kind);
    const sessionEpoch = state.sessionEpoch;
    const feedbackGeneration = clearClientActionFeedback();
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED) return false;
    try {
      if (typeof navigator === "undefined" || typeof navigator.clipboard?.writeText !== "function") {
        throw new Error("Clipboard unavailable");
      }
      await navigator.clipboard.writeText(text);
      if (clipboardFeedbackOperationIsCurrent(sessionEpoch, feedbackGeneration)) {
        setClientActionFeedback("success", feedback.success, { generation: feedbackGeneration });
      }
      return true;
    } catch (_) {
      if (clipboardFeedbackOperationIsCurrent(sessionEpoch, feedbackGeneration)) {
        setClientActionFeedback("error", feedback.failure, { generation: feedbackGeneration });
      }
      return false;
    }
  }

  async function copyVaultInviteUrl() {
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED || !state.lastEmailInviteUrl) {
      setClientActionFeedback("error", CLIENT_ACTION_FEEDBACK.inviteLinkCopyFailure);
      return false;
    }
    return copyToClipboard(state.lastEmailInviteUrl, "invite-link");
  }

  function handleContextMenuAction(item, target) {
    if (item.disabled) return;
    const accessRoute = accessActionRoute(item.action, target);
    if (accessRoute) {
      closeContextMenu({ restoreFocus: true });
      state.accessResult = null;
      state.activeAccessFolderId = accessRoute.folderId;
      state.activeAccessIntent = accessRoute.intent;
      state.selectedFolderId = accessRoute.folderId;
      state.expandedFolderIds.add(accessRoute.folderId);
      $("pageFolderIdInput").value = accessRoute.folderId;
      openSettingsModal(accessRoute.settingsSection);
      log(accessIntentValue(accessRoute.intent) === "links" ? "Opened Folder links panel." : "Opened Folder access panel.", {
        folderId: accessRoute.folderId,
        intent: accessRoute.intent,
      });
      return;
    }
    closeContextMenu();
    if (item.action === "open-folder") {
      selectReaderFolder(target.folderId);
      return;
    }
    if (item.action === "open-page") {
      selectReaderPage(target.pageKey);
      return;
    }
    if (item.action === "new-page") {
      startNewPageDraft(target.folderId);
      return;
    }
    if (item.action === "new-folder") {
      createFolderFromToolbar(target.folderId).catch((error) => {
        state.lastError = error.message;
        log("Failed to create Folder from context menu.", { error: error.message });
        render();
      });
      return;
    }
    if (item.action === "open-graph") {
      setWorkspaceView("graph");
      return;
    }
    if (item.action === "copy-page-id") {
      void copyToClipboard(target.objectId, "page-id");
      return;
    }
    if (item.action === "copy-folder-id") {
      void copyToClipboard(target.folderId, "folder-id");
      return;
    }
    if (item.action === "delete-page") {
      deletePageFromContextTarget(target).catch((error) => {
        state.lastError = error.message;
        log("Failed to delete Page.", { error: error.message });
        render();
      });
      return;
    }
  }

  function openContextMenu(target, x, y, previousFocus = document.activeElement || null) {
    const menu = $("contextMenu");
    if (!menu) return;
    state.contextMenuTarget = target;
    state.contextMenuPreviousFocus = previousFocus;
    menu.replaceChildren();
    const items = contextMenuItemsForTarget(target);
    for (const item of items) {
      if (item.separator) {
        const separator = document.createElement("div");
        separator.className = "context-menu-separator";
        separator.setAttribute("role", "separator");
        menu.appendChild(separator);
        continue;
      }
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = item.label;
      button.disabled = Boolean(item.disabled);
      button.className = item.danger ? "danger" : "";
      button.setAttribute("role", "menuitem");
      button.addEventListener("click", () => handleContextMenuAction(item, target));
      menu.appendChild(button);
    }
    menu.hidden = false;
    positionContextMenu(menu, x, y, items.length);
    focusContextMenuItem(0);
  }

  function appendObsidianDetail(button, detail) {
    if (!detail) return;
    const detailElement = document.createElement("span");
    detailElement.className = "obsidian-file-detail";
    detailElement.textContent = detail;
    button.appendChild(detailElement);
  }

  function appendSearchHighlightedText(parent, text, query) {
    for (const segment of searchHighlightSegments(text, query)) {
      if (!segment.match) {
        parent.appendChild(document.createTextNode(segment.text));
        continue;
      }
      const match = document.createElement("mark");
      match.className = "search-match";
      match.textContent = segment.text;
      parent.appendChild(match);
    }
  }

  function appendSearchMatchSnippet(button, snippet, query) {
    if (!snippet) return;
    const snippetElement = document.createElement("span");
    snippetElement.className = "search-match-snippet";
    appendSearchHighlightedText(snippetElement, snippet, query);
    button.appendChild(snippetElement);
  }

  function obsidianTreeButton(label, detail, className, onClick, options = {}) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = className;
    const title = document.createElement("span");
    title.className = "obsidian-file-title";
    if (options.highlightQuery) {
      appendSearchHighlightedText(title, label, options.highlightQuery);
    } else {
      title.textContent = label;
    }
    button.appendChild(title);
    appendObsidianDetail(button, detail);
    appendSearchMatchSnippet(button, options.matchSnippet, options.highlightQuery);
    button.addEventListener("click", onClick);
    if (options.contextTarget) {
      button.addEventListener("contextmenu", (event) => {
        event.preventDefault();
        openContextMenu(options.contextTarget, event.clientX, event.clientY, button);
      });
      button.addEventListener("keydown", (event) => {
        if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
        event.preventDefault();
        const bounds = button.getBoundingClientRect?.();
        openContextMenu(
          options.contextTarget,
          bounds?.left || 8,
          bounds?.bottom || 8,
          button
        );
      });
    }
    return button;
  }

  function accessFolderOptionButton(row, options = {}) {
    const { index = 0, isActive = false, isFocused = false, openedFolders } = options;
    const button = document.createElement("button");
    button.type = "button";
    button.className = `folder-option-button ${row.status}${isActive ? " active" : ""}`;
    button.id = `accessFolderOption-${index}`;
    button.dataset.folderId = row.id;
    button.dataset.folderIndex = String(index);
    button.setAttribute("role", "option");
    button.setAttribute("aria-selected", String(isActive));
    button.tabIndex = isFocused ? 0 : -1;

    const title = document.createElement("span");
    title.className = "folder-option-title";
    title.textContent = row.path;

    const meta = document.createElement("span");
    meta.className = "folder-option-meta";
    meta.textContent = `${row.accessLabel} · ${accessKeySummary(row, openedFolders).toLowerCase()}`;

    button.appendChild(title);
    button.appendChild(meta);
    return button;
  }

  function renderSidebarMode() {
    const mode = normalizeSidebarMode(state.activeSidebarMode);
    state.activeSidebarMode = mode;
    $("filesSidebarPanel").hidden = mode !== "files";
    $("searchSidebarPanel").hidden = mode !== "search";
    $("ribbonFilesButton").className = `ribbon-button${mode === "files" ? " active" : ""}`;
    $("ribbonSearchButton").className = `ribbon-button${mode === "search" ? " active" : ""}`;
    const accessActive = state.settingsModalOpen && state.settingsSection === "access";
    $("ribbonAccessButton").className = `ribbon-button${accessActive ? " active" : ""}`;
    setText("sidebarModeTitle", sidebarModeLabel(mode));
    setPressed("ribbonFilesButton", mode === "files");
    setPressed("ribbonSearchButton", mode === "search");
    setPressed("ribbonAccessButton", accessActive);
  }

  function renderSearchPanel() {
    const query = $("sidebarSearchInput").value;
    const rows = searchPageRows(query);
    setPill("searchResultCount", `${rows.length}`, rows.length ? "ready" : "muted");
    setList(
      "sidebarSearchResults",
      rows,
      query.trim() ? "No matching pages" : "Search pages",
      (item, row) => {
        const button = obsidianTreeButton(
          row.label,
          row.detail,
          `obsidian-page-button ${row.key === state.selectedPageKey ? " active" : ""}`,
          () => selectReaderPage(row.key, { searchQuery: query }),
          {
            contextTarget: {
              type: "page",
              folderId: row.folderId,
              localDraft: Boolean(row.localDraft),
              objectId: row.objectId,
              pageKey: row.key,
              revision: row.revision,
              title: row.title,
            },
            highlightQuery: query,
            matchSnippet: row.matchSnippet,
          }
        );
        item.appendChild(button);
      }
    );
  }

  function renderAccessResultPanel() {
    const panel = $("accessResultPanel");
    const result = state.accessResult;
    panel.hidden = !result;
    panel.className = `access-result ${result?.tone || ""}`;
    panel.replaceChildren();
    if (!result) return;
    const title = document.createElement("strong");
    title.textContent = result.title;
    panel.appendChild(title);
    const detail = document.createElement("span");
    detail.textContent = result.detail;
    panel.appendChild(detail);
    if (result.meta) {
      for (const [key, value] of Object.entries(result.meta)) {
        const line = document.createElement("span");
        line.textContent = `${key}: ${value}`;
        panel.appendChild(line);
      }
    }
  }

  function renderAccessShareControls() {
    // Keep visible share controls initialized without retaining hidden legacy
    // controls that used to proxy Folder grants and removals.
    if (!$("accessShareExpiresAtInput").value) {
      $("accessShareExpiresAtInput").value = defaultShareExpiryDateTimeLocal();
    }
    if (state.lastShareLinkId && !$("accessShareLinkInput").value) {
      $("accessShareLinkInput").value = state.lastShareLinkId;
    }

    renderAccessResultPanel();
  }

  function renderVaultInvitationPanel() {
    if (!$("vaultInviteExpiresAtInput").value) {
      $("vaultInviteExpiresAtInput").value = defaultShareExpiryDateTimeLocal();
    }
    if ($("vaultInviteEmailProofCreatedAtInput") && !$("vaultInviteEmailProofCreatedAtInput").value) {
      $("vaultInviteEmailProofCreatedAtInput").value = dateTimeLocalFromIso(new Date().toISOString());
    }
    if (state.lastEmailInviteSecret && $("vaultInviteSecretInput") && !$("vaultInviteSecretInput").value) {
      $("vaultInviteSecretInput").value = state.lastEmailInviteSecret;
    }
    const inviteUrlVisible =
      state.sessionStatus === SESSION_STATUS.UNLOCKED && Boolean(state.lastEmailInviteUrl);
    safeSetHidden("vaultInviteUrlOutput", !inviteUrlVisible);
    const inviteUrlInput = $("vaultInviteUrlInput");
    if (inviteUrlInput) inviteUrlInput.value = inviteUrlVisible ? state.lastEmailInviteUrl : "";
    setOptionalDisabled("copyVaultInviteUrlButton", !inviteUrlVisible);
    const controls = vaultInvitationPanelState({
      activeVaultAvailable: Boolean(state.activeVaultId),
      busy: state.accessBusy,
      code: $("vaultInviteCodeInput").value.trim() || state.lastVaultInvitationCode || "",
      email: $("vaultInviteEmailInput")?.value,
      inviteSecret: $("vaultInviteSecretInput")?.value,
      organizationVault: state.metadata?.kind === "organization",
      sessionStatus: state.sessionStatus,
      signerCanConnect: deriveBrainIdentityProviderState(state.identityProvider).canConnect,
      signerStatus: state.signerStatus,
    });
    safeSetHidden("vaultInviteConnectSignerButton", controls.connected);
    setOptionalDisabled("vaultInviteConnectSignerButton", controls.connectDisabled);
    $("createVaultInvitationButton").disabled = controls.createDisabled;
    $("getVaultInvitationButton").disabled = controls.inspectDisabled;
    setOptionalDisabled("getEmailInviteInstructionsButton", controls.emailScopeDisabled);
    $("acceptVaultInvitationButton").disabled = controls.acceptDisabled;
    $("revokeVaultInvitationButton").disabled = controls.revokeDisabled;
    setText("vaultInvitationHint", controls.hint);
  }

  function renderAccessPanel() {
    const rows = readerFolderRows(state.metadata);
    const openedFolders = openedGrantFolderKeys();
    const activeFolderId = state.activeAccessFolderId || state.selectedFolderId;
    const activeRow = rows.find((row) => row.id === activeFolderId) || rows[0] || null;
    if (activeRow && !state.activeAccessFolderId && !state.selectedFolderId) {
      state.activeAccessFolderId = activeRow.id;
    }

    renderAccessSidebarCount(rows);
    renderAccessBusyChrome();

    // Render folder selector
    renderFolderSelector(activeRow, rows, openedFolders);

    // Render main access inspector
    renderAccessInspector(activeRow, state.metadata, openedFolders);

    renderVaultAccessManagement(state.metadata);

    // Update access result panel (for feedback)
    renderAccessResultPanel();

    // Render vault admin panel
    renderVaultInvitationPanel();
  }

  function renderAccessSidebarCount(folderRows) {
    const folderCount = folderRows.length;
    setPill("accessSidebarCount", `${folderCount}`, folderCount ? "ready" : "muted");
  }

  function renderAccessBusyChrome() {
    const busy = state.accessBusy;
    safeSetHidden("accessBusyStatus", !busy);
    safeSetElement("accessFolderPanel", (panel) => panel.classList.toggle("is-busy", busy));
  }

  function actorIsVaultAdmin(metadata) {
    const actorNpub = state.pubkeyHex ? npubFromHex(state.pubkeyHex) : null;
    if (metadata?.kind === "personal") return Boolean(actorNpub && metadata.ownerUserId === actorNpub);
    return Boolean(actorNpub && (metadata?.admins || []).includes(actorNpub));
  }

  function hasOrganizationVaultControls(metadata) {
    return metadata?.kind === "organization";
  }

  function showsCreateOrganizationControl(metadata) {
    return !hasOrganizationVaultControls(metadata);
  }

  function canManageVaultPeople(metadata) {
    return (
      Boolean(metadata) &&
      hasOrganizationVaultControls(metadata) &&
      state.signerStatus === "connected" &&
      actorIsVaultAdmin(metadata) &&
      !state.accessBusy
    );
  }

  function linkStatusRank(status) {
    if (status === "pending" || status === "active") return 0;
    if (status === "accepted") return 1;
    return 2;
  }

  function vaultInvitationRows(invitations) {
    return [...(invitations || [])]
      .sort(
        (left, right) =>
          linkStatusRank(left.status) - linkStatusRank(right.status) ||
          String(right.createdAt).localeCompare(String(left.createdAt))
      )
      .map((invitation) => ({
        expiresAt: invitation.expiresAt,
        id: invitation.id,
        inviteCode: invitation.inviteCode,
        revocable: invitation.status === "pending",
        status: invitation.status,
        targetNpub: invitation.userId,
      }));
  }

  function folderShareLinkRows(shareLinks) {
    return [...(shareLinks || [])]
      .sort(
        (left, right) =>
          linkStatusRank(left.status) - linkStatusRank(right.status) ||
          String(right.createdAt).localeCompare(String(left.createdAt))
      )
      .map((shareLink) => ({
        expiresAt: shareLink.expiresAt,
        id: shareLink.id,
        recipientNpub: shareLink.recipientNpub,
        revocable: shareLink.status === "pending",
        status: shareLink.status,
      }));
  }

  function sharedFolderRelationshipRows(invitationLists, connectionLists) {
    const rows = [];
    for (const direction of ["outgoing", "incoming"]) {
      for (const connection of connectionLists?.[direction] || []) {
        rows.push({
          acceptable: false,
          counterpartVaultId:
            direction === "outgoing" ? connection.destinationVaultId : connection.sourceVaultId,
          direction,
          folderId: connection.sourceFolderId,
          id: connection.id,
          kind: "connection",
          memberCount: (connection.memberNpubs || []).length,
          revocable: false,
          status: connection.status,
        });
      }
    }
    for (const direction of ["outgoing", "incoming"]) {
      for (const invitation of invitationLists?.[direction] || []) {
        rows.push({
          acceptable: direction === "incoming" && invitation.status === "pending",
          counterpartVaultId:
            direction === "outgoing" ? invitation.destinationVaultId : invitation.sourceVaultId,
          direction,
          folderId: invitation.sourceFolderId,
          id: invitation.id,
          kind: "invitation",
          memberCount: null,
          revocable: direction === "outgoing" && invitation.status === "pending",
          status: invitation.status,
        });
      }
    }
    return rows.sort(
      (left, right) => linkStatusRank(left.status) - linkStatusRank(right.status)
    );
  }

  function vaultPeopleRows(metadata) {
    if (!metadata) return [];
    const vaultPersonRow = (npub, role, type, removable) => {
      const identity = identityMetadataForNpub(npub);
      return {
        details: identity.details,
        id: npub,
        name: identity.display,
        npub: identity.npub,
        role,
        status: identity.status,
        tooltip: identity.tooltip,
        type,
        removable,
      };
    };
    if (metadata.kind === "personal") {
      const owner = metadata.ownerUserId || metadata.owner_user_id || null;
      return owner ? [vaultPersonRow(owner, "owner", "owner", false)] : [];
    }
    const rows = [];
    const admins = uniqueNpubs(metadata.admins || []);
    const members = uniqueNpubs(metadata.members || []);
    for (const admin of admins) {
      rows.push(vaultPersonRow(admin, "admin", "admin", true));
    }
    for (const member of members) {
      if (admins.includes(member)) continue;
      rows.push(vaultPersonRow(member, "member", "member", true));
    }
    return rows;
  }

  function vaultHealthBadges(metadata, signerStatus = state.signerStatus) {
    if (!metadata) {
      return [{ label: "no vault", tone: "muted" }];
    }
    const badges = [
      { label: metadata.kind === "organization" ? "organization" : "personal", tone: "ready" },
      { label: `${(metadata.folders || []).length} folders`, tone: "muted" },
      { label: `${metadata.grantCount || 0} grants`, tone: "muted" },
    ];
    badges.unshift(
      signerStatus === "connected"
        ? { label: "signer connected", tone: "ready" }
        : { label: "signer missing", tone: "warn" }
    );
    if ((metadata.mountedFolders || []).length) {
      badges.push({ label: `${metadata.mountedFolders.length} mounts`, tone: "muted" });
    }
    if (state.lastVaultInvitationCode) {
      badges.push({ label: "invite ready", tone: "ready" });
    }
    return badges;
  }

  function vaultManagementSummary(metadata) {
    if (!metadata) {
      return state.sessionStatus === SESSION_STATUS.LOCKED
        ? "Choose a Vault, then unlock it to open encrypted content."
        : "Choose a Vault, then load it to decrypt its readable Folders.";
    }
    if (metadata.kind === "personal") {
      return "Personal vault loaded. Use Access for Folder permissions and share links.";
    }
    return `Organization loaded. ${countLabel((metadata.members || []).length, "member")} • ${countLabel(
      (metadata.admins || []).length,
      "admin"
    )} • ${countLabel((metadata.folders || []).length, "Folder")}`;
  }

  function vaultSwitchRowMeta(vault, isLoaded) {
    const kind = vault.kind === "personal" ? "personal" : "organization";
    const role = vault.role || (vault.kind === "personal" ? "owner" : "member");
    return `${kind} - ${role}${isLoaded ? " - loaded" : ""}`;
  }

  function vaultSwitchButton(vault, surface = "management") {
    const isSelected = vault.vaultId === state.activeVaultId;
    const isLoaded = state.metadata?.vaultId === vault.vaultId;
    const isLocked = state.sessionStatus === SESSION_STATUS.LOCKED && !state.visibleVaults.length;
    const statusText = isLoaded
      ? "loaded"
      : isLocked
        ? "locked"
        : isSelected
          ? "selected"
          : "available";
    const button = document.createElement("button");
    button.type = "button";
    button.className = `vault-switch-button${isSelected ? " selected" : ""}${isLoaded ? " loaded" : ""}${isLocked ? " locked" : ""}`;
    button.setAttribute("aria-pressed", String(isSelected));
    button.setAttribute(
      "aria-label",
      `${vault.name || vault.vaultId}, ${vaultSwitchRowMeta(vault, isLoaded)}, ${
        statusText
      }`
    );
    button.addEventListener("click", () => {
      if (vault.vaultId === state.activeVaultId) {
        if (surface === "switcher") closeVaultSwitcher();
        return;
      }
      setActiveVaultId(vault.vaultId);
      log("Selected Vault.", { vaultId: vault.vaultId });
      if (surface === "switcher") closeVaultSwitcher();
      else render();
    });

    const title = document.createElement("span");
    title.className = "vault-switch-title";
    title.textContent = vault.name || vault.vaultId;

    const meta = document.createElement("span");
    meta.className = "vault-switch-meta";
    meta.textContent = vaultSwitchRowMeta(vault, isLoaded);

    const status = document.createElement("span");
    status.className = `pill ${isLoaded ? "ready" : isLocked ? "warn" : isSelected ? "warn" : "muted"}`;
    status.textContent = statusText;
    status.setAttribute("aria-hidden", "true");

    button.appendChild(title);
    button.appendChild(meta);
    button.appendChild(status);
    return button;
  }

  function renderVaultAccessManagement(metadata) {
    const organizationVault = hasOrganizationVaultControls(metadata);
    const inviteInProgress = Boolean(
      state.lastVaultInvitationCode ||
        $("vaultInviteCodeInput")?.value.trim() ||
        $("vaultInviteSecretInput")?.value.trim()
    );
    safeSetHidden("vaultInvitationActionSection", !organizationVault);
    safeSetElement("vaultPeopleActionPanel", (panel) => {
      panel.hidden = !organizationVault;
      if (!organizationVault) panel.open = false;
    });
    safeSetHidden("vaultPeopleSection", !organizationVault);
    safeSetHidden("vaultInvitationListSection", !organizationVault);
    safeSetHidden("sharedFolderSection", !organizationVault);
    safeSetElement("vaultInvitationPanel", (panel) => {
      panel.hidden = false;
      if (inviteInProgress) {
        panel.open = true;
      }
    });
    renderVaultPeopleList(metadata);
    renderVaultPeopleControls(metadata);
    renderAgentWorkspacePairings(metadata);
    renderVaultInvitationList();
    renderSharedFolderList();
  }

  function renderAgentWorkspacePairings(metadata) {
    const ownerCanPair = Boolean(
      metadata?.kind === "personal" &&
        actorIsVaultAdmin(metadata) &&
        state.sessionStatus === SESSION_STATUS.UNLOCKED &&
        state.signerStatus === "connected"
    );
    const visible = metadata?.kind === "personal" && actorIsVaultAdmin(metadata);
    safeSetHidden("agentWorkspacePairingSection", !visible);
    const rows = agentWorkspacePairingRows({ pairings: state.agentWorkspacePairings || [] });
    setPill("agentWorkspacePairingCount", `${rows.length}`, rows.length ? "ready" : "muted");
    setOptionalDisabled("agentWorkspaceNpubInput", !ownerCanPair || state.accessBusy);
    setOptionalDisabled("pairAgentWorkspaceButton", !ownerCanPair || state.accessBusy);
    setText(
      "agentWorkspacePairingHint",
      ownerCanPair
        ? "Pairing creates a dedicated restricted Folder. It does not make the agent a Vault admin."
        : "Unlock your Personal Vault as its owner to pair an agent."
    );
    setList("agentWorkspacePairingList", rows, "No agent is paired yet.", (item, row) => {
      linkRowInfo(item, identityDisplay(row.agentNpub), row.status, row.title);
      const detail = document.createElement("span");
      detail.className = "access-person-role";
      detail.textContent = `${row.folderId} · ${row.detail}`;
      item.appendChild(detail);
    });
  }

  function renderVaultPeopleList(metadata) {
    const rows = vaultPeopleRows(metadata);
    setPill("vaultPeopleCount", `${rows.length}`, rows.length ? "ready" : "muted");
    const emptyText = metadata?.kind === "personal"
      ? "Personal Vaults do not use a member list."
      : "Load an organization Vault to manage Member Identities.";
    const canManage = canManageVaultPeople(metadata);
    setList("vaultPeopleList", rows, emptyText, (item, person) => {
      const personInfo = document.createElement("div");
      personInfo.className = "access-person-info";

      const icon = document.createElement("svg");
      icon.className = "access-person-icon icon";
      icon.setAttribute("viewBox", "0 0 24 24");
      icon.innerHTML = person.type === "admin"
        ? '<path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2Z" />'
        : '<circle cx="12" cy="8" r="4"/><path d="M12 12c-4 0-7 2-7 6v2h14v-2c0-4-3-6-7-6z"/>';

      const nameSpan = document.createElement("span");
      nameSpan.className = "access-person-name";
      nameSpan.textContent = person.name;

      const roleSpan = document.createElement("span");
      roleSpan.className = "access-person-role";
      roleSpan.textContent = person.role;

      personInfo.appendChild(icon);
      personInfo.appendChild(nameSpan);
      personInfo.appendChild(roleSpan);
      item.appendChild(personInfo);

      const detailButton = document.createElement("button");
      detailButton.className = "access-person-info-button";
      detailButton.type = "button";
      detailButton.title = person.tooltip;
      detailButton.setAttribute("aria-expanded", "false");
      detailButton.setAttribute("aria-label", `Show identity details for ${person.name}`);
      detailButton.innerHTML =
        '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9" /><path d="M12 11v5" /><path d="M12 8h.01" /></svg>';

      const detailPanel = document.createElement("dl");
      detailPanel.className = "access-person-detail-panel";
      detailPanel.hidden = true;
      for (const detail of person.details || []) {
        const term = document.createElement("dt");
        term.textContent = detail.label;
        const value = document.createElement("dd");
        value.textContent = detail.value;
        detailPanel.appendChild(term);
        detailPanel.appendChild(value);
      }
      detailButton.addEventListener("click", () => {
        const isOpen = !detailPanel.hidden;
        detailPanel.hidden = isOpen;
        detailButton.setAttribute("aria-expanded", String(!isOpen));
      });
      item.appendChild(detailButton);

      if (person.removable && canManage) {
        const removeButton = document.createElement("button");
        removeButton.className = "access-remove-person vault-person-action";
        removeButton.type = "button";
        removeButton.textContent = person.type === "admin" ? "Remove admin" : "Remove";
        removeButton.addEventListener("click", () => {
          const action = person.type === "admin" ? removeVaultAdminFromPanel : removeVaultMemberFromPanel;
          action(person.id).catch((error) => {
            reportClientActionFailure(error);
            log("Failed to update Vault Member Identities.", { error: error.message });
          });
        });
        item.appendChild(removeButton);
      }
      item.appendChild(detailPanel);
    });
  }

  function renderVaultPeopleControls(metadata) {
    const canManage = canManageVaultPeople(metadata);
    setOptionalDisabled("addVaultMemberButton", !canManage);
    setOptionalDisabled("addVaultAdminButton", !canManage);
    const hint = !metadata
      ? "Load an organization Vault to manage Member Identities."
      : metadata.kind !== "organization"
        ? "Personal Vaults use Folder access and share links instead of member lists."
        : actorIsVaultAdmin(metadata)
          ? "Admins must already be Vault members."
          : "Only Vault admins can change organization members and admins.";
    setText("vaultPeopleHint", hint);
    setText("vaultPeopleActionHint", canManage ? "Add or promote existing identities" : "Admin-only");
  }

  function linkRowActionButton(label, onClick, options = {}) {
    const button = document.createElement("button");
    button.className = `access-remove-person vault-person-action${options.danger ? " danger-action" : ""}`;
    button.type = "button";
    button.textContent = label;
    button.disabled = state.accessBusy;
    button.addEventListener("click", () => {
      onClick().catch((error) => {
        reportClientActionFailure(error);
        log("Access list action failed.", { error: error.message });
      });
    });
    return button;
  }

  function linkRowInfo(item, title, status, detail) {
    const info = document.createElement("div");
    info.className = "access-person-info";
    const name = document.createElement("span");
    name.className = "access-person-name";
    name.textContent = title;
    info.appendChild(name);
    if (detail) {
      const detailSpan = document.createElement("span");
      detailSpan.className = "access-person-role";
      detailSpan.textContent = detail;
      info.appendChild(detailSpan);
    }
    const statusSpan = document.createElement("span");
    statusSpan.className = `access-link-status ${status}`;
    statusSpan.textContent = status;
    info.appendChild(statusSpan);
    item.appendChild(info);
  }

  function renderVaultInvitationList() {
    const rows = vaultInvitationRows(state.vaultInvitations);
    const pendingCount = rows.filter((row) => row.status === "pending").length;
    setPill("vaultInvitationCount", `${pendingCount}`, pendingCount ? "ready" : "muted");
    const emptyText = canLoadVaultAdminLists()
      ? "No invitations yet. Create one under Give Vault access."
      : "Vault admins see pending invitations here.";
    setList("vaultInvitationList", rows, emptyText, (item, row) => {
      linkRowInfo(item, identityDisplay(row.targetNpub), row.status, `expires ${row.expiresAt.slice(0, 10)}`);
      if (!row.revocable) return;
      item.appendChild(
        linkRowActionButton("Use code", async () => {
          rememberVaultInvitationSelection(row);
          setAccessResult("ready", "Invite code loaded", `${row.inviteCode} is in the invite field.`, {
            invitationId: row.id,
          });
        })
      );
      item.appendChild(
        linkRowActionButton("Revoke", () => revokeVaultInvitationById(row.id), { danger: true })
      );
    });
  }

  function renderSharedFolderList() {
    const rows = sharedFolderRelationshipRows(
      state.sharedFolderInvitations,
      state.sharedFolderConnections
    );
    const activeCount = rows.filter(
      (row) => row.status === "active" || row.status === "pending"
    ).length;
    setPill("sharedFolderCount", `${activeCount}`, activeCount ? "ready" : "muted");
    const emptyText = canLoadVaultAdminLists()
      ? "No shared Folders yet. Sharing across Vaults starts with a shared Folder invitation."
      : "Vault admins see cross-Vault shared Folders here.";
    setList("sharedFolderList", rows, emptyText, (item, row) => {
      const directionLabel = row.direction === "outgoing" ? "to" : "from";
      const title = `${row.folderId} ${directionLabel} ${row.counterpartVaultId}`;
      const detail =
        row.kind === "connection"
          ? countLabel(row.memberCount, "member")
          : `${row.direction} invite`;
      linkRowInfo(item, title, row.status, detail);
      if (row.acceptable) {
        item.appendChild(
          linkRowActionButton("Accept", () => acceptSharedFolderInvitationById(row.id))
        );
      }
      if (row.revocable) {
        item.appendChild(
          linkRowActionButton("Revoke", () => revokeSharedFolderInvitationById(row.id), {
            danger: true,
          })
        );
      }
    });
  }

  function renderFolderShareLinkList(row) {
    const listMatchesFolder = Boolean(row) && state.folderShareLinksFolderId === row.id;
    const rows = listMatchesFolder ? folderShareLinkRows(state.folderShareLinks) : [];
    const pendingCount = rows.filter((linkRow) => linkRow.status === "pending").length;
    setPill("folderShareLinkCount", `${pendingCount}`, pendingCount ? "ready" : "muted");
    const emptyText = canLoadVaultAdminLists()
      ? "No share links for this Folder yet."
      : "Vault admins see this Folder's share links here.";
    setList("folderShareLinkList", rows, emptyText, (item, linkRow) => {
      linkRowInfo(
        item,
        identityDisplay(linkRow.recipientNpub),
        linkRow.status,
        `expires ${linkRow.expiresAt.slice(0, 10)}`
      );
      if (!linkRow.revocable) return;
      item.appendChild(
        linkRowActionButton("Use link", async () => {
          $("accessShareLinkInput").value = linkRow.id;
          setAccessResult("ready", "Share link loaded", `${linkRow.id} is in the link field.`, {
            recipient: identityDisplay(linkRow.recipientNpub),
          });
        })
      );
      item.appendChild(
        linkRowActionButton("Revoke", () => revokeShareLinkById(linkRow.id), { danger: true })
      );
    });
  }

  function accessFolderOptionElements() {
    const list = $("accessFolderList");
    if (!list) return [];
    return Array.from(list.querySelectorAll?.('[role="option"]:not([disabled])') || []);
  }

  function setAccessFolderDropdownOpen(open) {
    const isOpen = Boolean(open);
    state.accessFolderDropdownOpen = isOpen;
    const dropdown = $("accessFolderDropdown");
    const button = $("accessFolderButton");
    if (dropdown) dropdown.hidden = !isOpen;
    button?.setAttribute("aria-expanded", String(isOpen));
  }

  function closeAccessFolderDropdown(options = {}) {
    setAccessFolderDropdownOpen(false);
    if (options.focusTrigger) $("accessFolderButton")?.focus?.();
  }

  function openAccessFolderDropdown(options = {}) {
    const optionElements = accessFolderOptionElements();
    if (!state.accessFolderDropdownOpen) {
      const selectedIndex = optionElements.findIndex(
        (option) => option.getAttribute("aria-selected") === "true"
      );
      state.accessFolderFocusedIndex = selectedIndex >= 0 ? selectedIndex : 0;
    }
    setAccessFolderDropdownOpen(true);
    if (options.focus) focusAccessFolderOption(state.accessFolderFocusedIndex);
  }

  function focusAccessFolderOption(index) {
    const optionElements = accessFolderOptionElements();
    if (!optionElements.length) return;
    const nextIndex = Math.min(Math.max(index, 0), optionElements.length - 1);
    state.accessFolderFocusedIndex = nextIndex;
    optionElements.forEach((option, optionIndex) => {
      option.tabIndex = optionIndex === nextIndex ? 0 : -1;
    });
    optionElements[nextIndex]?.focus?.();
  }

  function accessFolderOptionFromEvent(event) {
    return event.target?.closest?.('[role="option"][data-folder-id]') || null;
  }

  function selectAccessFolderOption(option) {
    const folderId = option?.dataset?.folderId;
    if (!folderId) return;
    const index = Number(option.dataset.folderIndex);
    if (Number.isInteger(index) && index >= 0) state.accessFolderFocusedIndex = index;
    closeAccessFolderDropdown();
    selectAccessFolder(folderId);
    $("accessFolderButton")?.focus?.();
  }

  function bindAccessFolderSelector() {
    const button = $("accessFolderButton");
    const list = $("accessFolderList");
    if (!button || !list) return;
    button.addEventListener("click", () => {
      if (state.accessFolderDropdownOpen) {
        closeAccessFolderDropdown();
      } else {
        openAccessFolderDropdown();
      }
    });
    button.addEventListener("keydown", (event) => {
      if (event.isComposing || event.keyCode === 229) return;
      if (event.key === "Escape" && state.accessFolderDropdownOpen) {
        event.preventDefault();
        event.stopPropagation();
        closeAccessFolderDropdown({ focusTrigger: true });
        return;
      }
      const optionElements = accessFolderOptionElements();
      const nextIndex = keyboardListNavigationIndex(
        event.key,
        state.accessFolderFocusedIndex,
        optionElements.length
      );
      if (nextIndex === null) return;
      event.preventDefault();
      event.stopPropagation();
      openAccessFolderDropdown();
      focusAccessFolderOption(nextIndex);
    });
    list.addEventListener("click", (event) => {
      const option = accessFolderOptionFromEvent(event);
      if (!option) return;
      selectAccessFolderOption(option);
    });
    list.addEventListener("focusin", (event) => {
      const option = accessFolderOptionFromEvent(event);
      const index = Number(option?.dataset?.folderIndex);
      if (Number.isInteger(index) && index >= 0) state.accessFolderFocusedIndex = index;
    });
    list.addEventListener("keydown", (event) => {
      if (event.isComposing || event.keyCode === 229) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        closeAccessFolderDropdown({ focusTrigger: true });
        return;
      }
      const optionElements = accessFolderOptionElements();
      const activeIndex = optionElements.indexOf(document.activeElement);
      const nextIndex = keyboardListNavigationIndex(
        event.key,
        activeIndex >= 0 ? activeIndex : state.accessFolderFocusedIndex,
        optionElements.length
      );
      if (nextIndex !== null) {
        event.preventDefault();
        event.stopPropagation();
        focusAccessFolderOption(nextIndex);
        return;
      }
      if (event.key !== "Enter" && event.key !== " " && event.key !== "Spacebar") return;
      const option = accessFolderOptionFromEvent(event) || optionElements[state.accessFolderFocusedIndex];
      if (!option) return;
      event.preventDefault();
      event.stopPropagation();
      selectAccessFolderOption(option);
    });
  }

  function renderFolderSelector(activeRow, rows, openedFolders) {
    if (activeRow) {
      setText("accessFolderTitle", activeRow.path);
      setPill("accessFolderStatus", activeRow.accessLabel, activeRow.status === "ready" ? "ready" : "warn");
    } else {
      setText("accessFolderTitle", "No Folder selected");
      setPill("accessFolderStatus", "empty", "muted");
    }

    const dropdown = $("accessFolderDropdown");
    const button = $("accessFolderButton");
    if (!dropdown || !button) return;
    button.setAttribute("aria-controls", "accessFolderList");
    dropdown.hidden = !state.accessFolderDropdownOpen;
    button.setAttribute("aria-expanded", String(state.accessFolderDropdownOpen));
    const selectedIndex = rows.findIndex((row) => row.id === activeRow?.id);
    if (
      !Number.isInteger(state.accessFolderFocusedIndex) ||
      state.accessFolderFocusedIndex < 0 ||
      state.accessFolderFocusedIndex >= rows.length ||
      !state.accessFolderDropdownOpen
    ) {
      state.accessFolderFocusedIndex = selectedIndex >= 0 ? selectedIndex : 0;
    }
    setList("accessFolderList", rows, "Load a Vault to inspect access", (item, row) => {
      const index = rows.indexOf(row);
      item.appendChild(
        accessFolderOptionButton(row, {
          index,
          isActive: row.id === activeRow?.id,
          isFocused: index === state.accessFolderFocusedIndex,
          openedFolders,
        })
      );
    });

    if (!state.accessFolderDropdownListenerBound) {
      state.accessFolderDropdownListenerBound = true;
      document.addEventListener("click", (event) => {
        const currentButton = $("accessFolderButton");
        const currentDropdown = $("accessFolderDropdown");
        if (!currentButton || !currentDropdown || !state.accessFolderDropdownOpen) return;
        if (!currentButton.contains(event.target) && !currentDropdown.contains(event.target)) {
          closeAccessFolderDropdown();
        }
      });
    }
  }

  function renderAccessInspector(activeRow, metadata, openedFolders) {
    if (!activeRow) {
      setText("accessCurrentFolder", "No folder selected");
      setText("accessSummaryLine", "Load a Vault and select a Folder to inspect access.");
      renderWhoHasAccessList(null, metadata, openedFolders);
      renderAccessShareControls();
      updateAdvancedOptions(null, metadata, openedFolders);
      return;
    }

    // Update basic info
    setText("accessCurrentFolder", activeRow.path);
    setText("accessSummaryLine", generateAccessSummaryLine(activeRow, metadata, openedFolders));

    // Render who has access
    renderWhoHasAccessList(activeRow, metadata, openedFolders);
    applyAccessIntentChrome(activeRow);

    // Share-link defaults stay in sync with the visible access controls.
    renderAccessShareControls();

    // Update advanced options
    updateAdvancedOptions(activeRow, metadata, openedFolders);
  }

  function generateAccessSummaryLine(row, metadata, openedFolders) {
    if (!row) return "No access information available.";

    const audienceText = accessAudienceSummary(row);
    const peopleText = accessPeopleSummary(row, metadata);
    const keyStatus = accessKeySummary(row, openedFolders);

    return `${audienceText} access • ${peopleText} • ${keyStatus}`;
  }

  function renderWhoHasAccessList(row, metadata, openedFolders) {
    const list = $("accessWhoHasList");
    const addPanel = $("accessAddPersonPanel");
    const addForm = $("accessAddPersonForm");

    if (!row) {
      list.innerHTML = '<li class="access-empty-state">No access information available</li>';
      if (addPanel) {
        addPanel.hidden = true;
        addPanel.open = false;
      }
      if (addForm) addForm.hidden = true;
      return;
    }

    const canManage =
      folderAllowsDirectGrant(row) &&
      hasOpenedAccessFolderKey(row) &&
      state.signerStatus === "connected";
    if (addPanel) {
      addPanel.hidden = false;
    }
    if (addForm) {
      addForm.hidden = false;
      addForm.classList.toggle("is-ready", canManage);
      addForm.classList.toggle("is-locked", !canManage);
    }
    const accessList = buildAccessList(row, metadata);

    if (accessList.length === 0) {
      list.innerHTML = '<li class="access-empty-state">No explicit access granted</li>';
    } else {
      list.innerHTML = "";
      accessList.forEach((person) => {
        const item = document.createElement("li");

        const personInfo = document.createElement("div");
        personInfo.className = "access-person-info";

        const icon = document.createElement("svg");
        icon.className = "access-person-icon icon";
        icon.setAttribute("viewBox", "0 0 24 24");
        icon.innerHTML = person.type === "admin"
          ? '<path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2Z" />'
          : '<circle cx="12" cy="8" r="4"/><path d="M12 12c-4 0-7 2-7 6v2h14v-2c0-4-3-6-7-6z"/>';

        const nameSpan = document.createElement("span");
        nameSpan.className = "access-person-name";
        nameSpan.textContent = person.name;

        const roleSpan = document.createElement("span");
        roleSpan.className = "access-person-role";
        roleSpan.textContent = person.role;

        personInfo.appendChild(icon);
        personInfo.appendChild(nameSpan);
        if (person.role) {
          personInfo.appendChild(roleSpan);
        }

        item.appendChild(personInfo);

        if (person.removable && canManage) {
          const removeBtn = document.createElement("button");
          removeBtn.className = "access-remove-person";
          removeBtn.textContent = "Remove";
          removeBtn.onclick = () => removePersonAccess(person.id, row.id);
          item.appendChild(removeBtn);
        }

        list.appendChild(item);
      });
    }

    setupAddPersonForm(row);
  }

  function accessPersonId(person) {
    if (!person) return "";
    if (typeof person === "string") return person;
    return person.id || person.pubkey || person.npub || person.userId || person.user_id || "";
  }

  function accessPersonName(person) {
    if (!person) return "-";
    if (typeof person === "string") return identityDisplay(person);
    return identityEmailDisplay(person) || identityDisplay(accessPersonId(person));
  }

  function addAccessListPerson(accessList, person, role, type, removable = false) {
    const id = accessPersonId(person);
    if (!id || accessList.some((entry) => entry.id === id)) return;
    accessList.push({
      id,
      name: accessPersonName(person),
      role,
      type,
      removable,
    });
  }

  function buildAccessList(row, metadata) {
    const accessList = [];

    // Add implicit access based on folder mode
    if (row.access === "owner") {
      accessList.push({
        id: "owner",
        name: "You (owner)",
        role: "owner",
        type: "owner",
        removable: false
      });
    } else if (row.access === "admin_only" && metadata?.admins) {
      metadata.admins.forEach((admin) => addAccessListPerson(accessList, admin, "admin", "admin"));
    } else if (row.access === "all_members") {
      // Add admins first
      if (metadata?.admins) {
        metadata.admins.forEach((admin) => addAccessListPerson(accessList, admin, "admin", "admin"));
      }
      // Add members
      (metadata?.members || []).forEach((member) => addAccessListPerson(accessList, member, "member", "member"));
    } else if (row.access === "restricted") {
      // Add admins (implicit access)
      if (metadata?.admins) {
        metadata.admins.forEach((admin) => addAccessListPerson(accessList, admin, "admin", "admin"));
      } else if (metadata?.kind === "personal") {
        accessList.push({
          id: "owner",
          name: "You (owner)",
          role: "owner",
          type: "owner",
          removable: false,
        });
      }

      // Add explicit grants
      if (row.accessUserIds) {
        row.accessUserIds.forEach(userId => {
          const member = metadata?.members?.find((candidate) => accessPersonId(candidate) === userId);
          const agentPairing = agentWorkspacePairingRows({
            pairings: state.agentWorkspacePairings || [],
          }).find((pairing) => pairing.agentNpub === userId && pairing.folderId === row.id);
          addAccessListPerson(
            accessList,
            member || userId,
            agentPairing ? "agent workspace" : "explicit access",
            agentPairing ? "agent" : "explicit",
            true
          );
        });
      }
    }

    return accessList;
  }

  function setupAddPersonForm(row) {
    const addInput = $("accessAddPersonInput");
    const addButton = $("accessAddPersonButton");
    const addHint = $("accessAddPersonHint");
    const keyOpen = hasOpenedAccessFolderKey(row);
    const canManage =
      folderAllowsDirectGrant(row) &&
      keyOpen &&
      state.signerStatus === "connected";

    addButton.disabled = state.accessBusy || !canManage;
    addInput.disabled = state.accessBusy || !canManage;

    addButton.onclick = () => {
      const identity = addInput.value.trim();
      if (identity && row) {
        state.activeAccessIntent = "people";
        grantFolderAccessFromPanel(identity)
          .then(() => {
            addInput.value = "";
          })
          .catch((error) => {
            reportClientActionFailure(error);
            log("Failed to grant Folder access.", { error: error.message });
          });
      }
    };

    addInput.onkeydown = (e) => {
      if (e.key === "Enter" && !addButton.disabled) {
        addButton.click();
      }
    };

    if (state.signerStatus !== "connected") {
      addHint.textContent = "Connect a signer to grant access to this Folder.";
    } else if (!folderAllowsDirectGrant(row) || !keyOpen) {
      addHint.textContent = accessFlowHint(row, "people", keyOpen);
    } else {
      addHint.textContent = row.access === "all_members"
        ? `Enter an existing Vault Member Identity to send the Folder Key for "${row.path}"`
        : `Enter a Member Identity to grant access to "${row.path}"`;
    }
  }

  function removePersonAccess(personId, folderId) {
    if (state.accessBusy || state.signerStatus !== "connected") return;

    state.activeAccessIntent = "people";
    if (folderId && folderId !== state.activeAccessFolderId) {
      state.activeAccessFolderId = folderId;
    }
    removeFolderAccessFromPanel(personId).catch((error) => {
      reportClientActionFailure(error);
      log("Failed to remove Folder access.", { error: error.message });
    });
  }

  function updateAdvancedOptions(row, metadata, openedFolders) {
    const section = $("accessAdvancedSection");
    const linkListSection = $("folderShareLinkListSection");
    const shareForm = $("accessShareForm");
    const shareHint = $("accessShareHint");
    const createShareButton = $("createShareLinkButton");
    const acceptShareButton = $("acceptShareLinkButton");
    const revokeShareButton = $("revokeShareLinkButton");
    const shareTargetInput = $("accessShareTargetInput");
    const shareExpiresInput = $("accessShareExpiresAtInput");
    const shareMountInput = $("accessShareMountInput");
    const shareMountHint = $("accessShareMountHint");

    if (!row) {
      section.hidden = true;
      section.open = false;
      if (linkListSection) linkListSection.hidden = true;
      return;
    }

    section.hidden = false;
    if (linkListSection) linkListSection.hidden = false;

    // Update share link controls
    const keyOpen = hasOpenedAccessFolderKey(row);
    const isRestricted = row.access === "restricted";
    const canCreateShare =
      isRestricted &&
      keyOpen &&
      !state.accessBusy &&
      state.signerStatus === "connected";

    createShareButton.disabled = !canCreateShare;
    acceptShareButton.disabled = state.accessBusy || state.signerStatus !== "connected";
    revokeShareButton.disabled = state.accessBusy || state.signerStatus !== "connected";
    shareTargetInput.disabled = !canCreateShare;
    shareExpiresInput.disabled = !canCreateShare;
    shareMountInput.disabled = !canCreateShare;

    if (shareForm) {
      shareForm.classList.toggle("is-ready", canCreateShare);
      shareForm.classList.toggle("is-locked", !canCreateShare);
    }
    if (shareHint) {
      if (state.signerStatus !== "connected") {
        shareHint.textContent = "Connect a signer to create or accept share links.";
      } else if (!keyOpen) {
        shareHint.textContent = accessFlowHint(row, "links", keyOpen);
      } else if (!isRestricted) {
        shareHint.textContent = "Share links are for restricted Folders. Choose a restricted Folder to create one.";
      } else {
        shareHint.textContent = "The selected Member Identity receives a single-use Folder Key Grant through the link.";
      }
    }
    if (shareMountHint) {
      shareMountHint.textContent = canCreateShare
        ? "When accepted, this adds a shortcut to the shared Folder in their Personal Vault. It does not copy data or change Folder access."
        : "Available when creating a restricted Folder share link.";
    }

    // Setup expiry defaults if not set
    if (!$("accessShareExpiresAtInput").value) {
      $("accessShareExpiresAtInput").value = defaultShareExpiryDateTimeLocal();
    }
    if (state.lastShareLinkId && !$("accessShareLinkInput").value) {
      $("accessShareLinkInput").value = state.lastShareLinkId;
    }

    renderFolderShareLinkList(row);
  }

  // Legacy compatibility - handle missing elements gracefully
  function safeSetElement(id, callback) {
    const element = document.getElementById(id);
    if (element && callback) {
      callback(element);
    }
  }

  function safeSetText(id, text) {
    safeSetElement(id, (el) => el.textContent = text);
  }

  function safeSetHidden(id, hidden) {
    safeSetElement(id, (el) => el.hidden = hidden);
  }

  function renderReader() {
    selectDefaultReaderTargets();
    const page = syncReaderInputsFromSelectedPage();
    const folderRows = readerFolderRows(state.metadata);

    setList("readerFolderList", folderRows, "Load a Vault to browse folders", (item, row) => {
      const expanded = state.expandedFolderIds.has(row.id);
      const button = obsidianTreeButton(
        row.path,
        "",
        `obsidian-folder-button ${row.status}${expanded ? " expanded" : ""}${
          row.id === state.selectedFolderId ? " active" : ""
        }`,
        () => toggleReaderFolder(row.id),
        {
          contextTarget: {
            type: "folder",
            folderId: row.id,
            path: row.path,
          },
        }
      );
      item.appendChild(button);
      const childPages = readerPageRows(row.id);
      if (expanded && childPages.length) {
        const childList = document.createElement("ol");
        childList.className = "obsidian-page-children";
        for (const pageRow of childPages) {
          const childItem = document.createElement("li");
          const pageButton = obsidianTreeButton(
            pageRow.label,
            pageRow.status === "ready" ? "" : "Locked",
            `obsidian-page-button ${pageRow.status}${pageRow.key === state.selectedPageKey ? " active" : ""}`,
            () => selectReaderPage(pageRow.key),
            {
              contextTarget: {
                type: "page",
                folderId: pageRow.folderId,
                localDraft: Boolean(pageRow.localDraft),
                objectId: pageRow.objectId,
                pageKey: pageRow.key,
                revision: pageRow.revision,
                title: pageRow.title,
              },
            }
          );
          childItem.appendChild(pageButton);
          childList.appendChild(childItem);
        }
        item.appendChild(childList);
      }
    });

    if (!page) {
      const sessionLocked = state.sessionStatus !== SESSION_STATUS.UNLOCKED;
      setText(
        "readerPageTitle",
        sessionLocked ? "Session locked" : state.selectedFolderId ? "No page selected" : "No folder selected"
      );
      setText(
        "readerPagePath",
        sessionLocked ? "Unlock the session to reopen encrypted Folder Key Grants" : state.selectedFolderId || "No page path loaded"
      );
      setPill("readerPageMeta", sessionLocked ? "locked" : "empty", sessionLocked ? "warn" : "muted");
      renderPageContent(null);
      renderLinkContext(null);
      renderPageStatus(null);
      renderWorkspaceChrome(null);
      return;
    }

    setText("readerPageTitle", page.title || page.objectId);
    setText("readerPagePath", pagePathLabel(page));
    setPill(
      "readerPageMeta",
      page.localDraft ? "draft" : `rev ${page.revision || 0}`,
      page.status === "ready" ? "ready" : "warn"
    );
    renderPageContent(page);
    renderLinkContext(page);
    renderPageStatus(page);
    renderWorkspaceChrome(page);
  }

  function renderSessionSecurity() {
    const view = sessionStatusView(state.sessionStatus);
    setText("sessionAccountVault", activeVaultLabel());
    setText("sessionAccountIdentity", sessionIdentityLabel());
    setText("sessionAccountStatus", view.title);
    const vaultTrigger = $("sessionAccountVaultButton");
    vaultTrigger?.setAttribute("aria-label", `Switch Vault (current: ${activeVaultLabel()})`);
    vaultTrigger?.setAttribute("title", "Switch Vault");
    vaultTrigger?.setAttribute("aria-expanded", String(state.vaultSwitcherOpen));
    setText("sessionSecurityTitle", view.title);
    setText("sessionSecurityDetail", state.sessionNotice || view.detail);
    safeSetHidden("resumeSessionButton", !view.locked);
    safeSetHidden("lockSessionButton", view.locked);
    setOptionalDisabled(
      "resumeSessionButton",
      state.sessionStatus === SESSION_STATUS.RESUMING || !canLoadVault()
    );
    const shell = document.querySelector?.(".obsidian-shell");
    if (shell) shell.dataset.sessionStatus = state.sessionStatus;
  }

  function render() {
    if (state.sessionStatus === SESSION_STATUS.LOCKED && sessionContainsSecretsOrPlaintext(state)) {
      clearSessionSecretsAndPlaintext(state);
      clearSessionOwnedDom();
    }
    renderClientActionFeedback();
    setOptionalDisabled("obsidianNewPageButton", state.sessionStatus !== SESSION_STATUS.UNLOCKED);
    setOptionalDisabled("obsidianNewFolderButton", state.sessionStatus !== SESSION_STATUS.UNLOCKED || !state.metadata);
    setOptionalDisabled(
      "refreshReaderButton",
      state.sessionStatus !== SESSION_STATUS.UNLOCKED || state.readerBusy || state.signerStatus !== "connected" || !state.metadata
    );
    renderSessionSecurity();
    renderSettingsModal();
    renderVaultSwitcher();
    renderManageVaultsModal();
    renderSidebarMode();
    renderReader();
    if (state.activeWorkspaceView === "graph") renderGraphView();
    updateEditorChrome();
    renderSearchPanel();
    renderAccessPanel();
    renderCommandPalette();
  }

  function utf8Base64(text) {
    const bytes = new TextEncoder().encode(text);
    let binary = "";
    for (const byte of bytes) binary += String.fromCharCode(byte);
    return btoa(binary);
  }

  async function sha256Hex(text) {
    const bytes = new TextEncoder().encode(text);
    return sha256HexBytes(bytes);
  }

  async function sha256HexBytes(bytes) {
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
  }

  function authNonce() {
    return bytesToHex(crypto.getRandomValues(new Uint8Array(16)));
  }

  async function buildAuthEventTemplate(method, url, bodyText) {
    const tags = [
      ["u", url],
      ["method", method.toUpperCase()],
      ["nonce", authNonce()],
    ];
    if (bodyText) tags.push(["payload", await sha256Hex(bodyText)]);
    return {
      kind: 27235,
      created_at: Math.floor(Date.now() / 1000),
      tags,
      content: "",
    };
  }

  async function buildBrainAuthorizationHeader(provider, config, path, options = {}) {
    const derived = deriveBrainIdentityProviderState(provider);
    if (!derived.canConnect) throw new Error(derived.detail);
    if (!config) throw new Error("Product Client config has not loaded");
    const method = options.method || "GET";
    const bodyText = options.body || "";
    const url = `${config.publicBaseUrl.replace(/\/$/, "")}${path}`;
    const eventTemplate = await buildAuthEventTemplate(method, url, bodyText);
    const signed = await provider.authorizeHttpRequest({
      method,
      url,
      bodyText,
      eventTemplate,
    });
    if (
      provider === state.identityProvider &&
      state.pubkeyHex &&
      !signedEventMatchesPinnedIdentity(state.pubkeyHex, signed)
    ) {
      expireBrainIdentitySession();
      throw new Error("Brain identity changed while authorizing this request. Reopen Brain from Chat.");
    }
    return `${config.authScheme} ${utf8Base64(JSON.stringify(signed))}`;
  }

  async function signAuthHeader(path, options = {}) {
    return buildBrainAuthorizationHeader(state.identityProvider, state.config, path, options);
  }

  function protectedRequestError(path, status, body) {
    const reason = typeof body?.error === "string" ? body.error : null;
    const error = new Error(reason || `Request failed with ${status}`);
    error.status = status;
    error.reason = reason;
    error.path = path;
    return error;
  }

  function isActiveVaultAuthorizationLoss(error, activeVaultId) {
    if (
      !error ||
      error.status !== 403 ||
      error.reason !== VAULT_ACCESS_REQUIRED_REASON
    ) {
      return false;
    }
    const vaultId = String(activeVaultId || "").trim();
    if (!vaultId) return false;
    const vaultPath = `/_admin/vaults/${encodeURIComponent(vaultId)}`;
    return [
      `${vaultPath}/metadata`,
      `${vaultPath}/export`,
      `${vaultPath}/sync/bootstrap`,
    ].includes(error.path);
  }

  async function protectedRequest(path, options = {}) {
    const sessionEpoch = state.sessionEpoch;
    requireCurrentSessionEpoch(sessionEpoch);
    const headers = {
      Authorization: await signAuthHeader(path, options),
    };
    requireCurrentSessionEpoch(sessionEpoch);
    if (options.body) headers["Content-Type"] = "application/json";
    const response = await fetch(path, {
      method: options.method || "GET",
      headers,
      body: options.body || undefined,
    });
    requireCurrentSessionEpoch(sessionEpoch);
    const text = await response.text();
    requireCurrentSessionEpoch(sessionEpoch);
    let body = text;
    try {
      body = JSON.parse(text);
    } catch (_) {
      body = text;
    }
    if (!response.ok) {
      const error = protectedRequestError(path, response.status, body);
      lockSessionForVaultAccessChange(error, sessionEpoch);
      throw error;
    }
    rememberIdentitiesFrom(body);
    return body;
  }

  async function loadVisibleVaults() {
    if (state.signerStatus !== "connected") {
      state.visibleVaults = [];
      render();
      return [];
    }
    const response = await protectedRequest("/_admin/vaults");
    state.visibleVaults = (response.vaults || []).map(normalizeVisibleVault).filter(Boolean);
    const fallbackVaultId = missingVisibleVaultFallback(
      state.sessionStatus,
      state.activeVaultId,
      state.visibleVaults,
      state.pubkeyHex,
      state.config?.defaultVaultId
    );
    if (fallbackVaultId) {
      setActiveVaultId(fallbackVaultId);
      state.sessionNotice = "The previously selected Vault is no longer visible. Unlock the session to open the fallback Vault.";
      render();
      return state.visibleVaults;
    }
    const personal = visibleVaultOptions().find((vault) => vault.kind === "personal");
    if (
      personal &&
      (state.activeVaultId === PERSONAL_VAULT_PLACEHOLDER_ID || state.activeVaultId === state.config?.defaultVaultId)
    ) {
      setActiveVaultId(personal.vaultId, { reset: false });
    }
    render();
    return state.visibleVaults;
  }

  function defaultVaultPages(kind) {
    if (kind === "personal") return PERSONAL_DEFAULT_VAULT_PAGES.map((page) => ({ ...page }));
    if (kind === "organization") return ORGANIZATION_DEFAULT_VAULT_PAGES.map((page) => ({ ...page }));
    throw new Error(`Unsupported Vault kind: ${kind}`);
  }

  function defaultVaultPagesFolderId(kind) {
    if (kind === "personal") return DEFAULT_CLIENT_FOLDER_ID;
    if (kind === "organization") return DEFAULT_CLIENT_FOLDER_ID;
    throw new Error(`Unsupported Vault kind: ${kind}`);
  }

  function defaultVaultBootstrapFolderIds(kind) {
    if (kind === "personal") return ["getting-started", "restricted"];
    if (kind === "organization") return ["getting-started", "restricted"];
    throw new Error(`Unsupported Vault kind: ${kind}`);
  }

  function configuredRawFolderKey(input, folderId) {
    const source = input.rawKeysByFolderId;
    let value = null;
    if (source instanceof Map && source.has(folderId)) value = source.get(folderId);
    if (!value && source && Object.prototype.hasOwnProperty.call(source, folderId)) {
      value = source[folderId];
    }
    if (!value) return randomFolderKeyBytes();
    if (value instanceof Uint8Array) return value;
    if (Array.isArray(value)) return new Uint8Array(value);
    if (typeof value === "string") return base64ToBytes(value);
    throw new Error(`Unsupported raw Folder Key for ${folderId}`);
  }

  async function buildVaultBootstrapPlan(input) {
    if (!input?.vaultId) throw new Error("Vault bootstrap needs a Vault id");
    if (!input?.kind) throw new Error("Vault bootstrap needs a Vault kind");
    const actorNpub = input.actorNpub || currentActorNpub();
    const keyring = input.keyring || createSessionKeyring();
    const bootstrapGrants = [];
    const folderKeys = new Map();
    for (const folderId of defaultVaultBootstrapFolderIds(input.kind)) {
      const rawKey = configuredRawFolderKey(input, folderId);
      folderKeys.set(folderId, rawKey);
      await importFolderKey(keyring, {
        vaultId: input.vaultId,
        folderId,
        keyVersion: 1,
        folderKey: bytesToBase64(rawKey),
      });
      const grant = await buildFolderKeyGrantRequest({
        createdAtUnix: input.createdAtUnix,
        issuerNpub: actorNpub,
        keyVersion: 1,
        brainIdentityProvider: input.brainIdentityProvider,
        provider: input.provider,
        rawKey,
        recipientNpub: actorNpub,
        signEvent: input.signEvent,
        vaultId: input.vaultId,
        folderId,
      });
      bootstrapGrants.push({ folderId, grant });
    }
    return {
      bootstrapGrants,
      defaultFolderId: defaultVaultPagesFolderId(input.kind),
      defaultPages: defaultVaultPages(input.kind),
      folderKeys,
      keyring,
    };
  }

  async function buildDefaultVaultPageWrites(input) {
    if (!input?.keyring) throw new Error("Default Vault Pages need an opened keyring");
    if (!input?.vaultId) throw new Error("Default Vault Pages need a Vault id");
    const actorNpub = input.actorNpub || currentActorNpub();
    const signEvent = requireBrainEventAuthorizer("folder-object-revision", input);
    const pages = input.pages || defaultVaultPages(input.kind);
    const writes = [];
    let pageIndex = 0;
    for (const page of pages) {
      const folderId = page.folderId || input.folderId || defaultVaultPagesFolderId(input.kind);
      if (!folderId) throw new Error("Default Vault Pages need a target Folder");
      const nonceBytes =
        typeof input.nonceFactory === "function" ? input.nonceFactory(pageIndex, page) : undefined;
      const body = await buildPageWriteRequest(input.keyring, {
        authorNpub: actorNpub,
        baseRevision: null,
        createdAtUnix: input.createdAtUnix,
        folderId,
        keyVersion: input.keyVersion || 1,
        nonceBytes,
        objectId: page.objectId,
        operation: "create",
        plaintext: encodeFolderObjectPagePlaintext(page.path, page.markdown),
        signEvent,
        vaultId: input.vaultId,
      });
      writes.push({
        body,
        folderId,
        objectId: page.objectId,
        path: `/_admin/vaults/${encodeURIComponent(input.vaultId)}/folders/${encodeURIComponent(
          folderId
        )}/objects/${encodeURIComponent(page.objectId)}`,
        targetPath: page.path,
      });
      pageIndex += 1;
    }
    return writes;
  }

  async function writeDefaultVaultPages(input) {
    const request = input.request || protectedRequest;
    const writes = await buildDefaultVaultPageWrites(input);
    if (input.sessionEpoch !== undefined) requireCurrentSessionEpoch(input.sessionEpoch);
    for (const write of writes) {
      if (input.sessionEpoch !== undefined) requireCurrentSessionEpoch(input.sessionEpoch);
      await request(write.path, {
        method: "PUT",
        body: JSON.stringify(write.body),
      });
      if (input.sessionEpoch !== undefined) requireCurrentSessionEpoch(input.sessionEpoch);
    }
    return writes;
  }

  async function createVault(vaultId, kind, name) {
    const sessionEpoch = state.sessionEpoch;
    const actorNpub = currentActorNpub();
    const plan = await buildVaultBootstrapPlan({ vaultId, kind, name, actorNpub });
    requireCurrentSessionEpoch(sessionEpoch);
    const metadata = await protectedRequest("/_admin/vaults", {
      method: "POST",
      body: JSON.stringify({ vaultId, kind, name, bootstrapGrants: plan.bootstrapGrants }),
    });
    requireCurrentSessionEpoch(sessionEpoch);
    await writeDefaultVaultPages({
      actorNpub,
      kind,
      keyring: plan.keyring,
      sessionEpoch,
      vaultId,
    });
    requireCurrentSessionEpoch(sessionEpoch);
    state.keyring = plan.keyring;
    return metadata;
  }

  async function ensurePersonalVaultForActiveSelection() {
    const active = activeVaultOption();
    if (active.kind !== "personal") return;
    if (state.activeVaultId === PERSONAL_VAULT_PLACEHOLDER_ID && state.pubkeyHex) {
      setActiveVaultId(personalVaultIdForPubkey(state.pubkeyHex), { reset: false });
    }
    const existing = state.visibleVaults
      .map(normalizeVisibleVault)
      .find((vault) => vault?.kind === "personal" && vault.vaultId === state.activeVaultId);
    if (existing && !existing.pending) return;
    try {
      const metadata = await createVault(state.activeVaultId, "personal", "Personal vault");
      state.metadata = metadata;
      rememberVisibleVault(metadata);
    } catch (error) {
      if (!/already has a personal vault|duplicate id/.test(error.message)) throw error;
      await loadVisibleVaults();
      const personal = visibleVaultOptions().find((vault) => vault.kind === "personal");
      if (!personal) throw error;
      setActiveVaultId(personal.vaultId, { reset: false });
    }
  }

  async function ensureInvitedVaultAcceptedForActiveSelection() {
    const active = activeVaultOption();
    if (active.role !== "invited" || !active.inviteCode) return;
    const invitation = await protectedRequest(vaultInvitationAcceptPath(active.inviteCode), {
      method: "POST",
    });
    rememberVaultInvitationSelection(invitation);
    setActiveVaultId(invitation.vaultId, { reset: false });
    await loadVisibleVaults();
  }

  async function createOrganizationVaultFromInput(inputId) {
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED) {
      throw new Error("Session is locked. Unlock the session before creating a Vault");
    }
    const sessionEpoch = state.sessionEpoch;
    const input = $(inputId);
    const name = input?.value.trim() || "New organization";
    if (state.signerStatus !== "connected") await connectSigner();
    requireCurrentSessionEpoch(sessionEpoch);
    if (state.signerStatus !== "connected") throw new Error("Connect your Brain identity first");
    const vaultId = vaultIdFromName("org", name);
    const metadata = await createVault(vaultId, "organization", name);
    requireCurrentSessionEpoch(sessionEpoch);
    const createdKeyring = cloneSessionKeyring(state.keyring);
    if (input) input.value = "";
    rememberVisibleVault(metadata);
    setActiveVaultId(metadata.vaultId);
    const createdVaultEpoch = state.sessionEpoch;
    state.keyring = createdKeyring;
    state.metadata = metadata;
    state.sessionStatus = SESSION_STATUS.UNLOCKED;
    await loadVisibleVaults();
    requireCurrentSessionEpoch(createdVaultEpoch);
    log("Created organization Vault.", { vaultId: metadata.vaultId });
    render();
  }

  async function loadConfig() {
    const response = await fetch("/client/config.json");
    state.config = await response.json();
    if (!state.activeVaultId || state.activeVaultId === "smoke") {
      state.activeVaultId = state.config.defaultVaultId || PERSONAL_VAULT_PLACEHOLDER_ID;
    }
    log("Loaded Product Client config.", state.config);
    render();
  }

  async function detectSigner() {
    const hostedState = hostedIdentityProviderStates.get(state.identityProvider);
    if (hostedState) {
      try {
        await state.identityProvider.identifyMember();
      } catch (error) {
        state.lastError = error.message;
      }
    }
    const derived = deriveBrainIdentityProviderState(state.identityProvider);
    state.signerStatus = derived.status;
    render();
  }

  async function connectBrainIdentityProvider(options = {}) {
    const operationEpoch = options.sessionEpoch ?? state.sessionEpoch;
    const provider = state.identityProvider;
    const derived = deriveBrainIdentityProviderState(provider);
    if (!derived.canConnect) {
      state.signerStatus = derived.status;
      render();
      return null;
    }
    const identity = await provider.identifyMember();
    const pubkey = requireHex64(identity?.publicKeyHex, "Brain Member public key");
    if (state.sessionEpoch !== operationEpoch) {
      throw new Error("Session changed while Brain identity connection was in progress; unlock again");
    }
    const identityChanged = signerIdentityChanged(state.pubkeyHex, pubkey);
    if (identityChanged) {
      resetVaultSessionState({ preserveManageVaultsReturnToSettings: false });
      setActiveVaultId(personalVaultIdForPubkey(pubkey), { reset: false });
    }
    state.pubkeyHex = pubkey;
    state.signerStatus = "connected";
    if (state.activeVaultId === PERSONAL_VAULT_PLACEHOLDER_ID || state.activeVaultId === state.config?.defaultVaultId) {
      setActiveVaultId(personalVaultIdForPubkey(pubkey), { reset: false });
    }
    log(identityChanged ? "Connected a different signer identity." : "Connected signer.", {
      status: "connected",
    });
    if (options.loadVisibleVaults !== false && state.sessionStatus !== SESSION_STATUS.LOCKED) {
      await loadVisibleVaults().catch((error) => {
        state.lastError = error.message;
        log("Failed to load visible Vaults.", { error: error.message });
      });
    }
    render();
    return { publicKeyHex: pubkey, npub: npubFromHex(pubkey) };
  }

  async function connectSigner(options = {}) {
    return connectBrainIdentityProvider(options);
  }

  function expireBrainIdentitySession() {
    state.identityProvider = null;
    lockSession();
    const providerState = deriveBrainIdentityProviderState(null);
    state.signerStatus = providerState.status;
    render();
    return {
      sessionStatus: state.sessionStatus,
      providerStatus: providerState.status,
    };
  }

  async function loadVaultMetadata(options = {}) {
    if (!options.preserveActive) {
      await ensureInvitedVaultAcceptedForActiveSelection();
      await ensurePersonalVaultForActiveSelection();
    }
    const path = `/_admin/vaults/${encodeURIComponent(state.activeVaultId)}/metadata`;
    const metadata = await protectedRequest(path);
    state.metadata = metadata;
    if (metadata.kind === "personal" && actorIsVaultAdmin(metadata)) {
      const pairingList = await protectedRequest(agentWorkspacePairingsPath(metadata.vaultId));
      state.agentWorkspacePairings = pairingList.pairings || [];
    } else {
      state.agentWorkspacePairings = null;
    }
    rememberVisibleVault(metadata);
    log("Loaded Vault metadata.", metadata);
    render();
    if (state.settingsModalOpen && state.settingsSection === "access") {
      refreshAccessManagementListsInBackground();
    }
  }

  function canLoadVaultAdminLists() {
    return Boolean(
      state.metadata &&
        state.metadata.kind === "organization" &&
        state.signerStatus === "connected" &&
        actorIsVaultAdmin(state.metadata)
    );
  }

  async function refreshVaultAdminLists() {
    if (!canLoadVaultAdminLists()) {
      state.vaultInvitations = null;
      state.sharedFolderInvitations = null;
      state.sharedFolderConnections = null;
      return;
    }
    const vaultPath = `/_admin/vaults/${encodeURIComponent(state.activeVaultId)}`;
    const invitationList = await protectedRequest(`${vaultPath}/invitations`);
    state.vaultInvitations = invitationList.invitations || [];
    state.sharedFolderInvitations = await protectedRequest(
      `${vaultPath}/shared-folder-invitations`
    );
    state.sharedFolderConnections = await protectedRequest(
      `${vaultPath}/shared-folder-connections`
    );
  }

  async function refreshFolderShareLinks(folderId) {
    if (!folderId || !canLoadVaultAdminLists()) {
      state.folderShareLinks = null;
      state.folderShareLinksFolderId = null;
      return;
    }
    const path = `/_admin/vaults/${encodeURIComponent(
      state.activeVaultId
    )}/folders/${encodeURIComponent(folderId)}/share-links`;
    const list = await protectedRequest(path);
    state.folderShareLinks = list.shareLinks || [];
    state.folderShareLinksFolderId = folderId;
  }

  function refreshAccessManagementListsInBackground() {
    const work = async () => {
      await refreshVaultAdminLists();
      await refreshFolderShareLinks(state.activeAccessFolderId);
      render();
    };
    work().catch((error) => {
      log("Failed to refresh access management lists.", { error: error.message });
    });
  }

  async function revokeVaultInvitationById(invitationId) {
    requireUnlockedVaultInvitationAction("revoking an invitation");
    const sessionEpoch = captureSessionOperationEpoch();
    const vaultId = state.activeVaultId;
    beginAccessOperation(sessionEpoch);
    try {
      const invitation = await protectedRequest(
        vaultInvitationRevokePath(vaultId, invitationId),
        { method: "DELETE" }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      setAccessResult("warn", "Invitation revoked", `${invitation.id} is ${invitation.status}.`, {
        updatedAt: invitation.updatedAt,
      });
      log("Revoked Vault invitation from pending list.", { invitationId });
      await refreshVaultAdminLists();
      requireCurrentSessionEpoch(sessionEpoch);
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function revokeShareLinkById(shareLinkId) {
    const sessionEpoch = captureSessionOperationEpoch();
    beginAccessOperation(sessionEpoch);
    try {
      const shareLink = await protectedRequest(
        `/_admin/share-links/${encodeURIComponent(shareLinkId)}`,
        { method: "DELETE" }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      setAccessResult("warn", "Share link revoked", `${shareLink.id} is ${shareLink.status}.`, {
        updatedAt: shareLink.updatedAt,
      });
      log("Revoked Folder share link from list.", { shareLinkId });
      await refreshFolderShareLinks(state.activeAccessFolderId);
      requireCurrentSessionEpoch(sessionEpoch);
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function acceptSharedFolderInvitationById(invitationId) {
    const sessionEpoch = captureSessionOperationEpoch();
    beginAccessOperation(sessionEpoch);
    try {
      const invitation = await protectedRequest(
        `/_admin/shared-folder-invitations/${encodeURIComponent(invitationId)}/accept`,
        { method: "POST" }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      setAccessResult(
        "ready",
        "Shared Folder mounted",
        `${invitation.sourceFolderId} from ${invitation.sourceVaultId} is now mounted.`,
        { invitationId: invitation.id, status: invitation.status }
      );
      log("Accepted shared Folder invitation.", { invitationId });
      await loadVaultMetadata();
      requireCurrentSessionEpoch(sessionEpoch);
      await refreshVaultAdminLists();
      requireCurrentSessionEpoch(sessionEpoch);
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function revokeSharedFolderInvitationById(invitationId) {
    const sessionEpoch = captureSessionOperationEpoch();
    beginAccessOperation(sessionEpoch);
    try {
      const invitation = await protectedRequest(
        `/_admin/shared-folder-invitations/${encodeURIComponent(invitationId)}`,
        { method: "DELETE" }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      setAccessResult(
        "warn",
        "Shared Folder invitation revoked",
        `${invitation.id} is ${invitation.status}.`,
        { updatedAt: invitation.updatedAt }
      );
      log("Revoked shared Folder invitation.", { invitationId });
      await refreshVaultAdminLists();
      requireCurrentSessionEpoch(sessionEpoch);
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function openAvailableFolderKeyGrants(options = {}) {
    if (!sessionGrantOpeningAllowed(state.sessionStatus)) {
      throw new Error("Session is locked. Unlock the session before opening encrypted Folder Key Grants");
    }
    const sessionEpoch = state.sessionEpoch;
    const assertCurrent = () => requireCurrentSessionEpoch(sessionEpoch);
    assertCurrent();
    const keyring = options.keyring || state.keyring || createSessionKeyring();
    const vaultId = options.vaultId || state.activeVaultId;
    if (!options.keyring && !state.keyring) state.keyring = keyring;
    const exported = await protectedRequest(`/_admin/vaults/${encodeURIComponent(vaultId)}/export`);
    assertCurrent();
    const expectedRecipient = state.pubkeyHex ? npubFromHex(state.pubkeyHex) : null;
    return openFolderKeyGrants(keyring, exported, expectedRecipient, { assertCurrent });
  }

  function canLoadVault() {
    const provider = deriveBrainIdentityProviderState(state.identityProvider);
    return Boolean(
      state.config &&
        !state.readerBusy &&
        (state.signerStatus === "connected" || provider.canConnect)
    );
  }

  async function loadVaultReader(options = {}) {
    const allowResume = options.allowResume === true;
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED && !allowResume) {
      throw new Error("Session is locked. Use Unlock session before loading protected Vault state");
    }
    let relockOnFailure = state.sessionStatus !== SESSION_STATUS.UNLOCKED;
    let sessionEpoch = state.sessionEpoch;
    state.readerBusy = true;
    render();
    try {
      await connectSigner({ loadVisibleVaults: false, sessionEpoch });
      if (state.signerStatus !== "connected") throw new Error("Connect a Brain Identity Provider first");
      if (state.sessionStatus !== SESSION_STATUS.UNLOCKED && !allowResume) {
        throw new Error("Signer identity changed. Use Unlock session to open the new session");
      }
      relockOnFailure = relockOnFailure || state.sessionStatus !== SESSION_STATUS.UNLOCKED;
      if (state.sessionStatus !== SESSION_STATUS.UNLOCKED) state.sessionStatus = SESSION_STATUS.RESUMING;
      sessionEpoch = state.sessionEpoch;
      render();
      await loadVisibleVaults().catch((error) => {
        log("Failed to refresh visible Vaults before opening reader.", { error: error.message });
      });
      requireCurrentSessionEpoch(sessionEpoch);
      await loadVaultMetadata();
      requireCurrentSessionEpoch(sessionEpoch);
      const grants = await openAvailableFolderKeyGrants();
      requireCurrentSessionEpoch(sessionEpoch);
      await pullSyncBootstrap();
      requireCurrentSessionEpoch(sessionEpoch);
      selectDefaultReaderTargets();
      renderGraphView();
      state.sessionStatus = SESSION_STATUS.UNLOCKED;
      if (applyPendingInviteNavigation()) {
        state.sessionNotice = "Invitation details loaded into this unlocked session.";
      }
      log("Loaded Vault reader.", {
        openedFolderKeys: grants.opened.length,
        skippedFolderKeyGrants: grants.skipped.length,
        readablePages: readablePages().length,
      });
    } catch (error) {
      if (relockOnFailure && state.sessionEpoch === sessionEpoch) resetVaultSessionState();
      throw error;
    } finally {
      if (state.sessionEpoch === sessionEpoch) state.readerBusy = false;
      render();
    }
  }

  async function refreshReader() {
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED) {
      throw new Error("Session is locked. Unlock the session before refreshing readable content");
    }
    const sessionEpoch = state.sessionEpoch;
    state.readerBusy = true;
    render();
    try {
      await loadVaultMetadata();
      requireCurrentSessionEpoch(sessionEpoch);
      if (state.keyring?.openedGrants.length) await pullSyncBootstrap();
      requireCurrentSessionEpoch(sessionEpoch);
      selectDefaultReaderTargets();
      log("Refreshed Vault reader.", {
        readablePages: readablePages().length,
      });
    } finally {
      if (state.sessionEpoch === sessionEpoch) state.readerBusy = false;
      render();
    }
  }

  function activePageInput() {
    if (state.editorMode === "markdown") {
      rememberActiveDraft($("pageDraftInput").value);
    } else if (visualEditorElement()?.getAttribute?.("contenteditable") === "true") {
      syncDraftFromVisualEditor();
    }
    const folderId = $("pageFolderIdInput").value.trim() || DEFAULT_CLIENT_FOLDER_ID;
    const objectId = $("pageObjectIdInput").value.trim() || "obj_000000000001";
    const key = pageKey(folderId, objectId);
    const page = state.projection.pages.get(key);
    const draft = state.projection.localDrafts.get(key);
    return {
      baseRevision: $("pageBaseRevisionInput").value.trim(),
      folderId,
      objectId,
      path: draft?.path || page?.path || `${objectId}.md`,
      text: $("pageDraftInput").value,
    };
  }

  function currentFolderKeyVersion(folderId) {
    const folder = (state.metadata?.folders || []).find((candidate) => candidate.id === folderId);
    return folder?.currentKeyVersion || 1;
  }

  function currentActorNpub() {
    if (!state.pubkeyHex) throw new Error("Connect a signer first");
    return npubFromHex(state.pubkeyHex);
  }

  function activeAccessRow() {
    const rows = readerFolderRows(state.metadata);
    const activeFolderId = state.activeAccessFolderId || state.selectedFolderId;
    return rows.find((row) => row.id === activeFolderId) || rows[0] || null;
  }

  function requireRestrictedAccessRow() {
    const row = activeAccessRow();
    if (!row) throw new Error("Select a Folder first");
    if (row.access !== "restricted") {
      throw new Error("Folder sharing is available for restricted Folders");
    }
    return row;
  }

  function requireGrantableAccessRow() {
    const row = activeAccessRow();
    if (!row) throw new Error("Select a Folder first");
    if (!folderAllowsDirectGrant(row)) {
      throw new Error("Folder key grants are available for restricted or all-members Folders");
    }
    return row;
  }

  function openedAccessFolderKey(row) {
    const keyVersion = row.currentKeyVersion || currentFolderKeyVersion(row.id);
    const key = state.keyring?.keys.get(folderKeyId(state.activeVaultId, row.id, keyVersion));
    if (!key) throw new Error(`Open the Folder Key for ${row.path} before sharing`);
    return key;
  }

  function hasOpenedAccessFolderKey(row) {
    if (!row) return false;
    const keyVersion = row.currentKeyVersion || currentFolderKeyVersion(row.id);
    return Boolean(state.keyring?.keys.has(folderKeyId(state.activeVaultId, row.id, keyVersion)));
  }

  async function normalizedNpubInput(inputId, message) {
    return normalizedNpubValue($(inputId).value, message);
  }

  async function normalizedNpubValue(value, message) {
    const identity = await resolveIdentityInputValue(String(value || "").trim(), message);
    return identity.npub;
  }

  function defaultShareExpiryDateTimeLocal() {
    const date = new Date(Date.now() + 7 * 24 * 60 * 60 * 1000);
    date.setSeconds(0, 0);
    return dateTimeLocalValue(date);
  }

  function dateTimeLocalFromIso(value) {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) throw new Error("Timestamp is invalid");
    date.setSeconds(0, 0);
    return dateTimeLocalValue(date);
  }

  function dateTimeLocalValue(date) {
    const pad = (value) => String(value).padStart(2, "0");
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(
      date.getHours()
    )}:${pad(date.getMinutes())}`;
  }

  function slugFromFolderName(name) {
    return String(name || "")
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9_-]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 96);
  }

  function uniqueFolderId(baseId) {
    const existing = new Set((state.metadata?.folders || []).map((folder) => folder.id));
    let candidate = baseId || "folder";
    let suffix = 2;
    while (existing.has(candidate)) {
      candidate = `${baseId || "folder"}-${suffix}`;
      suffix += 1;
    }
    return candidate;
  }

  function folderCreationParent(parentFolderId, folders = []) {
    if (parentFolderId === null || parentFolderId === undefined) return null;
    const normalizedParentFolderId = String(parentFolderId).trim();
    if (!normalizedParentFolderId) throw new Error("Select a valid parent Folder");
    const parentFolder = (folders || []).find(
      (folder) => folder?.id === normalizedParentFolderId
    );
    if (!parentFolder || !String(parentFolder.path || "").trim()) {
      throw new Error("The selected parent Folder is no longer available");
    }
    return parentFolder;
  }

  function folderCreationHierarchy(parentFolder, name, folderId) {
    const normalizedName = String(name || "").trim();
    const normalizedFolderId = String(folderId || "").trim();
    if (!normalizedName || !normalizedFolderId) {
      throw new Error("Folder name and identifier are required");
    }
    if (!parentFolder) {
      return { parentFolderId: null, path: normalizedFolderId };
    }
    const parentFolderId = String(parentFolder.id || "").trim();
    const parentPath = String(parentFolder.path || "").trim();
    if (!parentFolderId || !parentPath) {
      throw new Error("The selected parent Folder is no longer available");
    }
    return { parentFolderId, path: `${parentPath}/${normalizedName}` };
  }

  function folderRecipientsForAccess(access, accessUserIds = []) {
    const recipients = new Set();
    if (access === "owner") {
      if (state.metadata?.ownerUserId) recipients.add(state.metadata.ownerUserId);
      else recipients.add(currentActorNpub());
      return [...recipients];
    }
    if (access === "admin_only" || access === "all_members" || access === "restricted") {
      for (const admin of state.metadata?.admins || []) recipients.add(admin);
    }
    if (access === "all_members") {
      for (const member of state.metadata?.members || []) recipients.add(member);
    }
    if (access === "restricted") {
      for (const user of accessUserIds) recipients.add(user);
    }
    if (!recipients.size) recipients.add(currentActorNpub());
    return [...recipients];
  }

  async function createFolderFromToolbar(parentFolderId = null) {
    if (!state.metadata) throw new Error("Open a Vault before creating a Folder");
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED) {
      throw new Error("Session is locked. Unlock the session before creating a Folder");
    }
    const sessionEpoch = state.sessionEpoch;
    const vaultId = state.activeVaultId;
    const sessionKeyring = state.keyring || createSessionKeyring();
    if (state.signerStatus !== "connected") await connectSigner({ sessionEpoch });
    requireCurrentSessionEpoch(sessionEpoch);
    if (state.signerStatus !== "connected") throw new Error("Connect your Brain identity first");

    const name = window.prompt("Folder name", "Notes")?.trim();
    if (!name) return;
    const folderId = uniqueFolderId(slugFromFolderName(name));
    const parentFolder = folderCreationParent(parentFolderId, state.metadata?.folders || []);
    const hierarchy = folderCreationHierarchy(parentFolder, name, folderId);
    const access = state.metadata.kind === "personal" ? "owner" : "all_members";
    const accessUserIds = [];
    const rawKey = randomFolderKeyBytes();
    const recipients = folderRecipientsForAccess(access, accessUserIds);
    const createdAtUnix = Math.floor(Date.now() / 1000);
    const grants = [];
    for (const recipientNpub of recipients) {
      grants.push(
        await buildFolderKeyGrantRequest({
          createdAtUnix,
          folderId,
          keyVersion: 1,
          rawKey,
          recipientNpub,
          vaultId,
        })
      );
      requireCurrentSessionEpoch(sessionEpoch);
    }
    await importFolderKey(
      sessionKeyring,
      {
        vaultId,
        folderId,
        keyVersion: 1,
        folderKey: bytesToBase64(rawKey),
      },
      { assertCurrent: () => requireCurrentSessionEpoch(sessionEpoch) }
    );
    requireCurrentSessionEpoch(sessionEpoch);
    const accessChangeEvent = await buildAdminAccessChangeEvent({
      action: "set-folder-access-mode",
      createdAtUnix,
      folderId,
      keyVersion: 1,
    });
    requireCurrentSessionEpoch(sessionEpoch);
    const metadata = await protectedRequest(
      `/_admin/vaults/${encodeURIComponent(vaultId)}/folders`,
      {
        method: "POST",
        body: JSON.stringify({
          access,
          accessChangeEvent,
          accessUserIds,
          folderId,
          grants,
          name,
          parentFolderId: hierarchy.parentFolderId,
          path: hierarchy.path,
          role: "folder",
          sharedFolderSource: false,
        }),
      }
    );
    requireCurrentSessionEpoch(sessionEpoch);
    state.keyring = sessionKeyring;
    state.metadata = metadata;
    state.selectedFolderId = folderId;
    state.expandedFolderIds.add(folderId);
    $("pageFolderIdInput").value = folderId;
    log("Created Folder from toolbar.", { folderId, recipients: recipients.length });
    render();
  }

  function shareExpiryIso() {
    const value = $("accessShareExpiresAtInput").value.trim();
    const date = value ? new Date(value) : new Date(Date.now() + 7 * 24 * 60 * 60 * 1000);
    if (Number.isNaN(date.getTime())) throw new Error("Share link expiry is invalid");
    return date.toISOString();
  }

  function vaultInvitationExpiryIso() {
    const value = $("vaultInviteExpiresAtInput").value.trim();
    const date = value ? new Date(value) : new Date(Date.now() + 7 * 24 * 60 * 60 * 1000);
    if (Number.isNaN(date.getTime())) throw new Error("Vault invitation expiry is invalid");
    return date.toISOString();
  }

  function emailProofCreatedAtIso() {
    const value = $("vaultInviteEmailProofCreatedAtInput")?.value.trim();
    if (!value) return new Date().toISOString();
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) throw new Error("Email proof timestamp is invalid");
    const now = new Date();
    if (now.getTime() >= date.getTime() && now.getTime() - date.getTime() < 60 * 1000) {
      return now.toISOString();
    }
    return date.toISOString();
  }

  function initialVaultInvitationFolders(value = $("vaultInviteFoldersInput").value) {
    return uniqueValues(
      String(value || "")
        .split(/[,\s]+/)
        .map((part) => part.trim())
        .filter(Boolean)
    );
  }

  function buildVaultInvitationRequest(input) {
    const targetNpub = input.targetNpub;
    npubToHex(targetNpub);
    return {
      targetNpub,
      initialFolderAccess: initialVaultInvitationFolders(input.initialFolderAccess || ""),
      expiresAt: input.expiresAt,
    };
  }

  function vaultInvitationIdentifierHint(input) {
    const value = String(input || "").trim();
    if (!value) return null;
    if (value.startsWith("invitation-")) {
      return "That is an invitation id. Inspect and Join Vault use an Invite Code like invite-...; use Revoke invite or fbrain invites accept --vault <vault-id> --id <invitation-id> for id-based actions.";
    }
    if (!value.startsWith("invite-")) {
      return "Invite Codes start with invite-. Check the copied code and the active signer.";
    }
    return null;
  }

  function vaultInvitationPanelState(input = {}) {
    const code = String(input.code || "").trim();
    const connected = input.signerStatus === "connected";
    const unlocked = input.sessionStatus === SESSION_STATUS.UNLOCKED;
    const busy = Boolean(input.busy);
    const organizationVault = Boolean(input.organizationVault);
    const codeHint = vaultInvitationIdentifierHint(code);
    const inviteCodeUsable = Boolean(code) && !codeHint;
    const emailClaimReady = Boolean(
      inviteCodeUsable && String(input.email || "").trim() && String(input.inviteSecret || "").trim()
    );
    const protectedActionDisabled = !connected || !unlocked || busy;
    let hint;
    if (!unlocked) {
      hint = "Unlock the session to inspect, accept, or manage invitations.";
    } else if (!connected) {
      hint = "Connect signer";
    } else if (codeHint) {
      hint = codeHint;
    } else if (inviteCodeUsable) {
      hint = "Ready to join Vault";
    } else {
      hint = "Enter an Invite Code";
    }
    return {
      acceptDisabled: protectedActionDisabled || !inviteCodeUsable,
      codeHint,
      connectDisabled: busy || !input.signerCanConnect,
      connected,
      createDisabled:
        protectedActionDisabled || !organizationVault || input.activeVaultAvailable === false,
      emailScopeDisabled: protectedActionDisabled || !emailClaimReady,
      hint,
      inspectDisabled: protectedActionDisabled || !inviteCodeUsable,
      inviteCodeUsable,
      revokeDisabled: protectedActionDisabled || !organizationVault || !code,
    };
  }

  function requireUnlockedVaultInvitationAction(action) {
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED) {
      throw new Error(`Session is locked. Unlock the session before ${action}`);
    }
  }

  function clearRememberedEmailInvitationMaterial() {
    const rememberedSecret = state.lastEmailInviteSecret;
    const secretInput = $("vaultInviteSecretInput");
    if (rememberedSecret && secretInput?.value === rememberedSecret) {
      secretInput.value = "";
    }
    state.lastEmailInviteSecret = null;
    state.lastEmailInviteUrl = null;
    const inviteUrlInput = $("vaultInviteUrlInput");
    if (inviteUrlInput) inviteUrlInput.value = "";
    safeSetHidden("vaultInviteUrlOutput", true);
    setOptionalDisabled("copyVaultInviteUrlButton", true);
  }

  function rememberVaultInvitationSelection(invitation) {
    const inviteCode = String(invitation?.inviteCode || "").trim();
    const invitationId = String(invitation?.id || "").trim() || null;
    const changed = inviteCode !== state.lastVaultInvitationCode;
    state.lastVaultInvitationCode = inviteCode || null;
    state.lastVaultInvitationId = invitationId;
    if (changed) {
      state.lastEmailInvitePostProof = null;
      clearRememberedEmailInvitationMaterial();
    }
    const codeInput = $("vaultInviteCodeInput");
    if (codeInput) codeInput.value = inviteCode;
  }

  function handleVaultInvitationInput(inputId) {
    if (inputId === "vaultInviteCodeInput") {
      const inviteCode = $("vaultInviteCodeInput")?.value.trim() || "";
      if (inviteCode !== state.lastVaultInvitationCode) {
        state.lastVaultInvitationCode = inviteCode || null;
        state.lastVaultInvitationId = null;
        state.lastEmailInvitePostProof = null;
        clearRememberedEmailInvitationMaterial();
      }
    } else if (
      inputId === "vaultInviteEmailInput" ||
      inputId === "vaultInviteEmailProofCreatedAtInput" ||
      inputId === "vaultInviteSecretInput"
    ) {
      state.lastEmailInvitePostProof = null;
      if (
        inputId === "vaultInviteSecretInput" &&
        $("vaultInviteSecretInput")?.value.trim() !== state.lastEmailInviteSecret
      ) {
        clearRememberedEmailInvitationMaterial();
      }
    }
    renderVaultInvitationPanel();
  }

  function vaultInvitationRevokeTarget(input = {}) {
    const value = String(input.input || "").trim();
    if (!value) throw new Error("Paste an Invite Code or invitation id first");
    const vaultId = String(input.activeVaultId || "").trim();
    if (!vaultId) throw new Error("Select a Vault before revoking an invitation");
    const invitations = input.invitations || [];
    const knownInvitation = invitations.find(
      (invitation) => invitation?.id === value || invitation?.inviteCode === value
    );
    if (knownInvitation?.id) {
      return { invitationId: knownInvitation.id, vaultId: knownInvitation.vaultId || vaultId };
    }
    if (
      value === String(input.lastVaultInvitationCode || "").trim() &&
      input.lastVaultInvitationId
    ) {
      return { invitationId: input.lastVaultInvitationId, vaultId };
    }
    if (value.startsWith("invitation-")) {
      return { invitationId: value, vaultId };
    }
    throw new Error(
      "Revoke an invitation created by this Vault admin from the pending invitation list, or paste its invitation id."
    );
  }

  function currentVaultInvitationInput() {
    const value = $("vaultInviteCodeInput").value.trim() || state.lastVaultInvitationCode;
    if (!value) throw new Error("Paste an Invite Code or invitation id first");
    return value;
  }

  function currentVaultInvitationCode() {
    const value = currentVaultInvitationInput();
    const hint = vaultInvitationIdentifierHint(value);
    if (hint) throw new Error(hint);
    return value;
  }

  function vaultInvitationUnavailableDetail(error) {
    const message = error?.message || String(error || "");
    if (message === "vault invitation unavailable") {
      return "Vault invitation unavailable. Check the Invite Code, active signer, expiry, or whether the invite was already handled.";
    }
    return message;
  }

  function activeSignerInviteDetail() {
    if (state.signerStatus !== "connected" || !state.pubkeyHex) return "Connect signer";
    return "Active signer connected. Invites are bound to the target email.";
  }

  function vaultInvitationCreatePath(vaultId) {
    return `/_admin/vaults/${encodeURIComponent(vaultId)}/invitations`;
  }

  function agentWorkspacePairingsPath(vaultId) {
    return `/_admin/vaults/${encodeURIComponent(vaultId)}/agent-workspace-pairings`;
  }

  function agentWorkspacePairingRows(response) {
    return (response?.pairings || []).map((pairing) => ({
      id: pairing.delegationId,
      agentNpub: pairing.agentNpub,
      folderId: pairing.workspaceFolderId,
      status: pairing.status,
      title: "Agent Workspace",
      detail: `${pairing.status === "active" ? "Active" : "Revoked"} · ${
        pairing.scope?.permission === "read_write" ? "read/write" : "scoped"
      } · explicitly paired by the Personal Vault owner`,
    }));
  }

  async function buildAgentWorkspacePairingRequest(input) {
    const vaultId = String(input.vaultId || state.activeVaultId || "").trim();
    const ownerNpub = input.ownerNpub || currentActorNpub();
    const agentNpub = publicKeyIdentityFromInput(input.agentNpub)?.npub;
    if (!vaultId) throw new Error("Agent Workspace pairing requires a Personal Vault");
    if (!agentNpub) throw new Error("Agent Workspace pairing requires an Agent Principal npub");
    if (agentNpub === ownerNpub) {
      throw new Error("Agent Workspace pairing requires a distinct Agent Principal");
    }
    const folderId = String(input.folderId || "agent-workspace").trim();
    const name = String(input.name || "Agent Workspace").trim();
    const path = String(input.path || name).trim();
    const rawKey = input.rawKey || randomFolderKeyBytes();
    const createdAtUnix = input.createdAtUnix || Math.floor(Date.now() / 1000);
    const grants = [];
    for (const recipientNpub of [ownerNpub, agentNpub]) {
      grants.push(
        await buildFolderKeyGrantRequest({
          brainIdentityProvider: input.brainIdentityProvider,
          createdAtUnix,
          encrypt: input.encrypt,
          folderId,
          issuerNpub: ownerNpub,
          keyVersion: 1,
          provider: input.provider,
          rawKey,
          recipientNpub,
          signEvent: input.signEvent,
          vaultId,
        })
      );
    }
    const accessChangeEvent = await buildAdminAccessChangeEvent({
      action: "set-folder-access-mode",
      adminNpub: ownerNpub,
      brainIdentityProvider: input.brainIdentityProvider,
      createdAtUnix,
      folderId,
      keyVersion: 1,
      provider: input.provider,
      signEvent: input.signEvent,
      vaultId,
    });
    return {
      path: agentWorkspacePairingsPath(vaultId),
      rawKey,
      body: {
        agentNpub,
        folderId,
        name,
        path,
        grants,
        accessChangeEvent,
      },
    };
  }

  async function ensureAgentWorkspacePairing(input) {
    const plan = await buildAgentWorkspacePairingRequest(input);
    const pairing = await protectedRequest(plan.path, {
      method: "POST",
      body: JSON.stringify(plan.body),
    });
    state.agentWorkspacePairings = [
      pairing,
      ...(state.agentWorkspacePairings || []).filter(
        (candidate) => candidate.delegationId !== pairing.delegationId
      ),
    ];
    return { pairing, rawKey: plan.rawKey };
  }

  async function pairAgentWorkspaceFromPanel() {
    const sessionEpoch = captureSessionOperationEpoch();
    const metadata = state.metadata;
    if (metadata?.kind !== "personal" || !actorIsVaultAdmin(metadata)) {
      throw new Error("Only the Personal Vault owner can pair an Agent Principal");
    }
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED) {
      throw new Error("Unlock your Personal Vault before pairing an Agent Principal");
    }
    beginAccessOperation(sessionEpoch);
    try {
      const identity = await resolveIdentityInputValue(
        $("agentWorkspaceNpubInput")?.value,
        "Enter an Agent Principal first"
      );
      requireCurrentSessionEpoch(sessionEpoch);
      if (
        (state.agentWorkspacePairings || []).some(
          (pairing) => pairing.agentNpub === identity.npub
        )
      ) {
        throw new Error("This Agent Principal is already paired with the Personal Vault");
      }
      const existingPairingCount = (state.agentWorkspacePairings || []).length;
      const folderId = existingPairingCount
        ? `agent-workspace-${npubToHex(identity.npub).slice(0, 12)}`
        : "agent-workspace";
      const result = await ensureAgentWorkspacePairing({
        agentNpub: identity.npub,
        folderId,
        name: "Agent Workspace",
        ownerNpub: currentActorNpub(),
        path: folderId,
        vaultId: metadata.vaultId,
      });
      requireCurrentSessionEpoch(sessionEpoch);
      await openFolderKeyGrantPlaintext(
        state.keyring,
        {
          version: "finite-folder-key-grant-v1",
          vaultId: metadata.vaultId,
          folderId: result.pairing.workspaceFolderId,
          keyVersion: 1,
          folderKey: bytesToBase64(result.rawKey),
          issuerNpub: currentActorNpub(),
          recipientNpub: currentActorNpub(),
        },
        { assertCurrent: () => requireCurrentSessionEpoch(sessionEpoch) }
      );
      if ($("agentWorkspaceNpubInput")) $("agentWorkspaceNpubInput").value = "";
      await loadVaultMetadata({ preserveActive: true });
      requireCurrentSessionEpoch(sessionEpoch);
      setAccessResult(
        "ready",
        "Agent paired",
        `${identityDisplay(identity.npub)} can read and write only ${result.pairing.workspaceFolderId}.`,
        { delegationId: result.pairing.delegationId }
      );
      log("Paired Agent Principal with a restricted Personal Vault Folder.", {
        agentNpub: identityDisplay(identity.npub),
        folderId: result.pairing.workspaceFolderId,
      });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Agent pairing failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  function vaultInvitationLinkPath(code) {
    return `/_admin/vault-invitation-links/${encodeURIComponent(code)}`;
  }

  function vaultInvitationAcceptPath(code) {
    return `${vaultInvitationLinkPath(code)}/accept`;
  }

  function vaultInvitationRevokePath(vaultId, invitationId) {
    return `/_admin/vaults/${encodeURIComponent(vaultId)}/invitations/${encodeURIComponent(invitationId)}`;
  }

  function uniqueValues(values) {
    return [...new Set((values || []).map((value) => String(value || "").trim()).filter(Boolean))];
  }

  function uniqueNpubs(values) {
    return uniqueValues(values);
  }

  function folderAccessRemovalRecipients(metadata, row, targetNpub) {
    if (!row || row.access !== "restricted") {
      throw new Error("Folder access removal is available for restricted Folders");
    }
    const accessUsers = uniqueNpubs(row.accessUserIds);
    if (!accessUsers.includes(targetNpub)) {
      throw new Error(`${identityDisplay(targetNpub)} does not have explicit access to ${row.path}`);
    }
    const admins = uniqueNpubs(metadata?.admins || []);
    if (admins.includes(targetNpub)) {
      throw new Error("Admins can still open restricted Folders; remove admin role first");
    }
    const remainingAccessUsers = accessUsers.filter((npub) => npub !== targetNpub);
    const recipients = uniqueNpubs([...admins, ...remainingAccessUsers]);
    if (!recipients.length) throw new Error("Folder Key rotation needs at least one remaining recipient");
    return { remainingAccessUsers, recipients };
  }

  function liveReadableFolderObjects(objects, folderId) {
    const rows = (objects || [])
      .filter((object) => object.folderId === folderId && !object.deleted)
      .sort((left, right) => String(left.objectId).localeCompare(String(right.objectId)));
    const unreadable = rows.filter((object) => object.status !== "ready" || typeof object.text !== "string");
    if (unreadable.length) {
      throw new Error("Every live Page in this Folder must be readable before rotating access");
    }
    return rows;
  }

  function randomFolderKeyBytes() {
    return crypto.getRandomValues(new Uint8Array(32));
  }

  function deterministicClientId(prefix, parts) {
    return sha256Hex(parts.join("\n")).then((digest) => `${prefix}-${digest.slice(0, 16)}`);
  }

  function canonicalAdminAccessChangePayload(input) {
    const fields = [
      `"version":${JSON.stringify("finite-vault-admin-access-change-v1")}`,
      `"vaultId":${JSON.stringify(input.vaultId)}`,
      `"changeId":${JSON.stringify(input.changeId)}`,
      `"action":${JSON.stringify(input.action)}`,
      `"adminNpub":${JSON.stringify(input.adminNpub)}`,
    ];
    if (input.folderId) fields.push(`"folderId":${JSON.stringify(input.folderId)}`);
    if (input.targetNpub) fields.push(`"targetNpub":${JSON.stringify(input.targetNpub)}`);
    if (input.keyVersion !== undefined && input.keyVersion !== null) {
      fields.push(`"keyVersion":${Number(input.keyVersion)}`);
    }
    if (input.note) fields.push(`"note":${JSON.stringify(input.note)}`);
    fields.push(`"createdAt":${JSON.stringify(input.createdAt)}`);
    return `{${fields.join(",")}}`;
  }

  function adminAccessChangeTags(input) {
    const tags = [
      ["d", `finite-vault-admin-access-change:${input.vaultId}:${input.changeId}`],
      ["vault", input.vaultId],
      ["action", input.action],
    ];
    if (input.folderId) tags.push(["folder", input.folderId]);
    if (input.targetNpub) tags.push(["p", npubToHex(input.targetNpub)]);
    if (input.keyVersion !== undefined && input.keyVersion !== null) {
      tags.push(["keyVersion", String(input.keyVersion)]);
    }
    return tags;
  }

  async function buildAdminAccessChangeEvent(input) {
    const signEvent = requireBrainEventAuthorizer("vault-access-change", input);
    const createdAtUnix = input.createdAtUnix || Math.floor(Date.now() / 1000);
    const createdAt = accessChangeCreatedAt(createdAtUnix);
    const adminNpub = input.adminNpub || currentActorNpub();
    const vaultId = input.vaultId || state.activeVaultId;
    const changeId =
      input.changeId ||
      (await deterministicClientId("access-change", [
        vaultId,
        input.action,
        input.folderId || "-",
        input.targetNpub || "-",
        createdAt,
      ]));
    const payload = {
      version: "finite-vault-admin-access-change-v1",
      vaultId,
      changeId,
      action: input.action,
      adminNpub,
      folderId: input.folderId,
      targetNpub: input.targetNpub,
      keyVersion: input.keyVersion,
      note: input.note,
      createdAt,
    };
    return signEvent({
      kind: APP_EVENT_KIND,
      created_at: createdAtUnix,
      tags: adminAccessChangeTags(payload),
      content: canonicalAdminAccessChangePayload(payload),
    });
  }

  async function buildFolderKeyGrantRequest(input) {
    const issuerNpub = input.issuerNpub || currentActorNpub();
    const createdAtUnix = input.createdAtUnix || Math.floor(Date.now() / 1000);
    const createdAt = revisionCreatedAt(createdAtUnix);
    const folderKey = input.folderKey || bytesToBase64(input.rawKey);
    const grantId =
      input.id ||
      (await deterministicClientId("grant", [
        input.vaultId,
        input.folderId,
        String(input.keyVersion),
        input.recipientNpub,
        createdAt,
      ]));
    const identityProvider = input.brainIdentityProvider || state.identityProvider;
    if (
      identityProvider?.grantOperationMode === "scoped" &&
      !input.encrypt &&
      !input.provider
    ) {
      const grant = await identityProvider.wrapGrantPayload({
        purpose: "folder-key-grant",
        vaultId: input.vaultId,
        folderId: input.folderId,
        keyVersion: Number(input.keyVersion),
        recipientNpub: input.recipientNpub,
        id: grantId,
        folderKey,
        createdAt,
        createdAtUnixSeconds: createdAtUnix,
      });
      if (
        grant?.id !== grantId ||
        Number(grant?.keyVersion) !== Number(input.keyVersion) ||
        grant?.recipientNpub !== input.recipientNpub ||
        typeof grant?.wrappedEventJson !== "string" ||
        !grant.wrappedEventJson ||
        grant?.createdAt !== createdAt
      ) {
        throw new Error("Hosted Folder Key Grant does not match the requested resource");
      }
      return grant;
    }
    const signSeal = requireBrainEventAuthorizer("folder-key-grant-seal", input);
    const signWrap = requireBrainEventAuthorizer("folder-key-grant-wrap", input);
    const encrypt = nip44EncryptAdapter(input);
    if (!encrypt && !input.allowPlaintextDevelopmentGrant) {
      throw new Error("NIP-44 encryption is unavailable");
    }
    const recipientHex = npubToHex(input.recipientNpub);
    const issuerHex = npubToHex(issuerNpub);
    const plaintextGrant = {
      version: "finite-folder-key-grant-v1",
      vaultId: input.vaultId,
      folderId: input.folderId,
      keyVersion: input.keyVersion,
      folderKey,
      issuerNpub,
      recipientNpub: input.recipientNpub,
      createdAt,
    };
    const rumorTags = [
      ["d", `finite-folder-key-grant:${input.vaultId}:${input.folderId}:${input.keyVersion}`],
      ["vault", input.vaultId],
      ["folder", input.folderId],
      ["keyVersion", String(input.keyVersion)],
    ];
    let wrappedEvent;
    if (encrypt) {
      const rumorContent = JSON.stringify(plaintextGrant);
      const rumor = {
        pubkey: issuerHex,
        created_at: createdAtUnix,
        kind: APP_EVENT_KIND,
        tags: rumorTags,
        content: rumorContent,
      };
      rumor.id = await sha256Hex(canonicalNostrEventIdInput(rumor));
      const sealContent = await encrypt(recipientHex, JSON.stringify(rumor));
      const seal = await signSeal({
        kind: 13,
        created_at: createdAtUnix,
        tags: [],
        content: sealContent,
      });
      const wrappedContent = await encrypt(recipientHex, JSON.stringify(seal));
      wrappedEvent = await signWrap({
        kind: 1059,
        created_at: createdAtUnix,
        tags: [["p", recipientHex]],
        content: wrappedContent,
      });
    } else {
      wrappedEvent = await signWrap({
        kind: 1059,
        created_at: createdAtUnix,
        tags: [["p", recipientHex]],
        content: JSON.stringify(plaintextGrant),
      });
    }
    return {
      id: grantId,
      keyVersion: input.keyVersion,
      recipientNpub: input.recipientNpub,
      wrappedEventJson: JSON.stringify(wrappedEvent),
      createdAt,
    };
  }

  function inviteEmailLike(value) {
    return looksLikeEmailIdentity(value);
  }

  function finiteVipEmail(value) {
    return /@finite\.vip$/i.test(String(value || "").trim());
  }

  function canonicalInviteEmail(value) {
    const email = String(value || "").trim().toLowerCase();
    if (!inviteEmailLike(email)) throw new Error("Email invite target must be an email address");
    if (/[\u0000-\u001f\u007f]/.test(email)) {
      throw new Error("Email invite target must be printable");
    }
    return email;
  }

  function emailInviteScope(metadata, selectedFolders) {
    const selectedValues = Array.isArray(selectedFolders)
      ? selectedFolders
      : initialVaultInvitationFolders(selectedFolders || "");
    const selected = new Set(uniqueValues(selectedValues));
    const seenSelected = new Set();
    const scope = [];
    for (const folder of metadataFolderRows(metadata)) {
      const selectedFolder = selected.has(folder.id);
      if (selectedFolder) seenSelected.add(folder.id);
      if (folder.access === "all_members" || (folder.access === "restricted" && selectedFolder)) {
        scope.push({
          folderId: folder.id,
          access: folder.access,
          keyVersion: folder.currentKeyVersion || 1,
        });
        continue;
      }
      if (selectedFolder) {
        throw new Error("Email invite bootstrap can include all-members and selected restricted Folders only");
      }
    }
    if (seenSelected.size !== selected.size) throw new Error("Folder not found");
    return scope;
  }

  function emailInviteScopeJson(scope) {
    return (scope || []).map((folder) => ({
      folderId: folder.folderId,
      access: folder.access,
      keyVersion: Number(folder.keyVersion),
    }));
  }

  function canonicalEmailInviteAuthorizationPayload(input) {
    return JSON.stringify({
      version: "finite-email-invite-bootstrap-authorization-v1",
      vaultId: input.vaultId,
      invitedEmail: input.invitedEmail,
      inviteUnwrapNpub: input.inviteUnwrapNpub,
      bootstrapPayloadHash: input.bootstrapPayloadHash,
      expiresAt: input.expiresAt,
      folders: emailInviteScopeJson(input.scope),
    });
  }

  function emailInviteAuthorizationTags(input) {
    return [
      ["d", `finite-email-invite-bootstrap-authorization:${input.vaultId}:${input.invitedEmail}`],
      ["vault", input.vaultId],
      ["email", input.invitedEmail],
    ];
  }

  async function buildEmailInviteAuthorizationEvent(input) {
    const signEvent = requireBrainEventAuthorizer("vault-invite-authorization", input);
    const createdAtUnix = input.createdAtUnix || Math.floor(Date.now() / 1000);
    return signEvent({
      kind: APP_EVENT_KIND,
      created_at: createdAtUnix,
      tags: emailInviteAuthorizationTags(input),
      content: canonicalEmailInviteAuthorizationPayload(input),
    });
  }

  function emailInviteBootstrapPayload(input) {
    return {
      version: "finite-email-invite-bootstrap-payload-v1",
      vaultId: input.vaultId,
      invitedEmail: input.invitedEmail,
      inviteUnwrapNpub: input.inviteUnwrapNpub,
      folders: emailInviteScopeJson(input.scope),
      grants: input.grants,
    };
  }

  async function buildEmailInviteBootstrapWrappedEvent(input) {
    const createdAtUnix = input.createdAtUnix || Math.floor(Date.now() / 1000);
    const identityProvider = input.brainIdentityProvider || state.identityProvider;
    if (
      identityProvider?.grantOperationMode === "scoped" &&
      !input.encrypt &&
      !input.provider
    ) {
      return identityProvider.wrapGrantPayload({
        purpose: "vault-invite-bootstrap",
        vaultId: input.vaultId,
        recipientNpub: input.inviteUnwrapNpub,
        plaintext: input.bootstrapPayloadJson,
        createdAtUnixSeconds: createdAtUnix,
      });
    }
    const signSeal = requireBrainEventAuthorizer("vault-invite-bootstrap-seal", input);
    const signWrap = requireBrainEventAuthorizer("vault-invite-bootstrap-wrap", input);
    const encrypt = nip44EncryptAdapter(input);
    if (!encrypt) throw new Error("NIP-44 encryption is unavailable");
    const issuerNpub = input.issuerNpub || currentActorNpub();
    const issuerHex = npubToHex(issuerNpub);
    const recipientHex = npubToHex(input.inviteUnwrapNpub);
    const rumor = {
      pubkey: issuerHex,
      created_at: createdAtUnix,
      kind: APP_EVENT_KIND,
      tags: [
        ["d", `finite-email-invite-bootstrap:${input.vaultId}`],
        ["vault", input.vaultId],
      ],
      content: input.bootstrapPayloadJson,
    };
    rumor.id = await sha256Hex(canonicalNostrEventIdInput(rumor));
    const sealContent = await encrypt(recipientHex, JSON.stringify(rumor));
    const seal = await signSeal({
      kind: 13,
      created_at: createdAtUnix,
      tags: [],
      content: sealContent,
    });
    const wrappedContent = await encrypt(recipientHex, JSON.stringify(seal));
    const wrapped = await signWrap({
      kind: 1059,
      created_at: createdAtUnix,
      tags: [["p", recipientHex]],
      content: wrappedContent,
    });
    return JSON.stringify(wrapped);
  }

  function openedKeyForScopeItem(keyring, vaultId, item) {
    const key = keyring?.keys?.get(folderKeyId(vaultId, item.folderId, item.keyVersion));
    if (!key) throw new Error(`Open Folder Key for ${item.folderId} v${item.keyVersion} before creating the invite`);
    return key;
  }

  async function buildEmailVaultInvitationRequest(keyring, input) {
    const invitedEmail = canonicalInviteEmail(input.target || input.invitedEmail);
    const vaultId = input.vaultId || state.activeVaultId;
    const issuerNpub = input.issuerNpub || currentActorNpub();
    const inviteKeypair = input.inviteKeypair || createInviteUnwrapKeypair();
    const inviteUnwrapNpub = inviteKeypair.npub || inviteKeypair.inviteUnwrapNpub;
    const inviteSecret = inviteKeypair.secretHex || inviteKeypair.inviteSecret;
    const scope = input.scope || emailInviteScope(input.metadata || state.metadata, input.initialFolderAccess || []);
    const initialFolderAccess =
      input.initialFolderAccess === undefined || input.initialFolderAccess === null
        ? scope.filter((folder) => folder.access === "restricted").map((folder) => folder.folderId)
        : initialVaultInvitationFolders(input.initialFolderAccess || "");
    const bootstrapGrants = [];
    for (const item of scope) {
      const key = openedKeyForScopeItem(keyring, vaultId, item);
      bootstrapGrants.push({
        folderId: item.folderId,
        grant: await buildFolderKeyGrantRequest({
          id: input.grantIdFactory ? input.grantIdFactory(item) : undefined,
          vaultId,
          folderId: item.folderId,
          keyVersion: item.keyVersion,
          folderKey: bytesToBase64(key.rawKey),
          issuerNpub,
          brainIdentityProvider: input.brainIdentityProvider,
          provider: input.provider,
          recipientNpub: inviteUnwrapNpub,
          signEvent: input.signEvent,
          createdAtUnix: input.createdAtUnix,
        }),
      });
    }
    const bootstrapPayload = emailInviteBootstrapPayload({
      vaultId,
      invitedEmail,
      inviteUnwrapNpub,
      scope,
      grants: bootstrapGrants,
    });
    const bootstrapPayloadJson = JSON.stringify(bootstrapPayload);
    const bootstrapPayloadHash = `sha256:${await sha256Hex(bootstrapPayloadJson)}`;
    const bootstrapWrappedEventJson = await buildEmailInviteBootstrapWrappedEvent({
      ...input,
      vaultId,
      issuerNpub,
      inviteUnwrapNpub,
      bootstrapPayloadJson,
      brainIdentityProvider: input.brainIdentityProvider,
      signEvent: input.signEvent,
    });
    const bootstrapAuthorizationEventJson = JSON.stringify(
      await buildEmailInviteAuthorizationEvent({
        ...input,
        vaultId,
        invitedEmail,
        inviteUnwrapNpub,
        bootstrapPayloadHash,
        expiresAt: input.expiresAt,
        scope,
        brainIdentityProvider: input.brainIdentityProvider,
        signEvent: input.signEvent,
      })
    );
    return {
      body: {
        target: invitedEmail,
        initialFolderAccess,
        expiresAt: input.expiresAt,
        inviteUnwrapNpub,
        bootstrapPayloadHash,
        bootstrapWrappedEventJson,
        bootstrapAuthorizationEventJson,
      },
      bootstrapPayloadJson,
      inviteSecret,
      inviteUnwrapNpub,
      scope,
    };
  }

  function emailInviteBootstrapPath(code) {
    return `${vaultInvitationLinkPath(code)}/bootstrap`;
  }

  function emailInviteInstructionsPath(code) {
    return `${vaultInvitationLinkPath(code)}/instructions`;
  }

  function emailInviteClaimPath(code) {
    return `${vaultInvitationLinkPath(code)}/claim`;
  }

  function emailInviteClientUrl(input) {
    const inviteCode = String(input.inviteCode || "").trim();
    const inviteSecret = String(input.inviteSecret || "").trim();
    if (!inviteCode) throw new Error("Email invite URL needs an invite code");
    if (!inviteSecret) throw new Error("Email invite URL needs an Invite Secret");
    const base = String(input.publicBaseUrl || state.config?.publicBaseUrl || window.location.origin).replace(/\/$/, "");
    const fragment = [`inviteCode=${encodeURIComponent(inviteCode)}`];
    if (input.invitedEmail) fragment.push(`inviteEmail=${encodeURIComponent(canonicalInviteEmail(input.invitedEmail))}`);
    fragment.push(`inviteSecret=${encodeURIComponent(inviteSecret)}`);
    return `${base}/client#${fragment.join("&")}`;
  }

  function emailInviteClaimProofPayload(input) {
    return JSON.stringify({
      version: "finite-email-invite-bootstrap-claim-proof-v1",
      vaultId: input.vaultId,
      inviteCode: input.inviteCode,
      invitedEmail: input.invitedEmail,
      claimantNpub: input.claimantNpub,
      bootstrapPayloadHash: input.bootstrapPayloadHash,
      emailProofCreatedAt: input.emailProofCreatedAt,
    });
  }

  async function buildEmailInviteClaimProofEvent(input) {
    const createdAtUnix = input.createdAtUnix || Math.floor(Date.now() / 1000);
    return signEventWithInviteSecret(
      {
        kind: APP_EVENT_KIND,
        created_at: createdAtUnix,
        tags: [],
        content: emailInviteClaimProofPayload(input),
      },
      input.inviteSecret,
      input
    );
  }

  function validateEmailBootstrapPayload(payload, payloadJson, invitation, input) {
    if (payload.version !== "finite-email-invite-bootstrap-payload-v1") {
      throw new Error("Unsupported Email Invite Bootstrap payload version");
    }
    const invitedEmail = canonicalInviteEmail(input.invitedEmail || input.email);
    if (payload.vaultId !== invitation.vaultId) throw new Error("Email Invite Bootstrap Vault mismatch");
    if (canonicalInviteEmail(payload.invitedEmail) !== invitedEmail) {
      throw new Error("Email Invite Bootstrap email mismatch");
    }
    if (payload.inviteUnwrapNpub !== invitation.inviteUnwrapNpub) {
      throw new Error("Email Invite Bootstrap unwrap npub mismatch");
    }
    const expectedScope = JSON.stringify(emailInviteScopeJson(invitation.bootstrapScope || []));
    if (JSON.stringify(emailInviteScopeJson(payload.folders || [])) !== expectedScope) {
      throw new Error("Email Invite Bootstrap scope mismatch");
    }
    return sha256Hex(payloadJson).then((hash) => {
      if (`sha256:${hash}` !== invitation.bootstrapPayloadHash) {
        throw new Error("Email Invite Bootstrap payload hash mismatch");
      }
      return payload;
    });
  }

  async function openEmailInviteBootstrap(invitation, input) {
    const inviteSecret = String(input.inviteSecret || "").trim();
    const inviteKeypair = inviteUnwrapKeypairFromSecret(inviteSecret);
    if (inviteKeypair.npub !== invitation.inviteUnwrapNpub) {
      throw new Error("Invite Secret does not match this email invitation");
    }
    const inviteDecrypt = input.inviteDecrypt || inviteSecretDecryptAdapter(inviteSecret);
    const { rumor } = await openGiftWrappedRumorContent(
      invitation.bootstrapWrappedEventJson,
      invitation.inviteUnwrapNpub,
      { decrypt: inviteDecrypt },
      "Email Invite Bootstrap"
    );
    const payloadJson = rumor.content;
    const payload = parseJsonObject(payloadJson, "Email Invite Bootstrap payload");
    await validateEmailBootstrapPayload(payload, payloadJson, invitation, input);
    return { payload, payloadJson };
  }

  async function buildEmailInviteClaimRequest(input) {
    const inviteSecret = String(input.inviteSecret || "").trim();
    const invitation = input.invitation;
    const claimantNpub = input.claimantNpub || currentActorNpub();
    const { payload } = await openEmailInviteBootstrap(invitation, input);
    const inviteDecrypt = input.inviteDecrypt || inviteSecretDecryptAdapter(inviteSecret);
    const keyring = input.keyring || createSessionKeyring();
    const grants = [];
    for (const entry of payload.grants || []) {
      const plaintext = await plaintextGrantFromGiftWrappedExportGrant(
        entry.grant,
        invitation.inviteUnwrapNpub,
        { decrypt: inviteDecrypt }
      );
      await openFolderKeyGrantPlaintext(keyring, plaintext);
      grants.push({
        folderId: entry.folderId,
        grant: await buildFolderKeyGrantRequest({
          id: input.claimGrantIdFactory ? input.claimGrantIdFactory(entry, plaintext) : undefined,
          vaultId: plaintext.vaultId,
          folderId: plaintext.folderId,
          keyVersion: plaintext.keyVersion,
          folderKey: plaintext.folderKey,
          issuerNpub: claimantNpub,
          brainIdentityProvider: input.brainIdentityProvider,
          provider: input.provider,
          recipientNpub: claimantNpub,
          signEvent: input.signEvent,
          createdAtUnix: input.createdAtUnix,
        }),
      });
    }
    const inviteUnwrapProofEventJson = JSON.stringify(
      await buildEmailInviteClaimProofEvent({
        inviteSecret,
        vaultId: invitation.vaultId,
        inviteCode: invitation.inviteCode,
        invitedEmail: canonicalInviteEmail(input.invitedEmail || input.email),
        claimantNpub,
        bootstrapPayloadHash: invitation.bootstrapPayloadHash,
        emailProofCreatedAt: input.emailProofCreatedAt,
        createdAtUnix: input.createdAtUnix,
        auxBytes: input.auxBytes,
      })
    );
    return {
      body: {
        email: canonicalInviteEmail(input.invitedEmail || input.email),
        emailProofCreatedAt: input.emailProofCreatedAt,
        inviteUnwrapProofEventJson,
        grants,
      },
      keyring,
      openedGrantCount: grants.length,
    };
  }

  function setAccessResult(tone, title, detail, meta = null) {
    state.accessResult = { tone, title, detail, meta };
    render();
  }

  function captureSessionOperationEpoch() {
    const sessionEpoch = state.sessionEpoch;
    requireCurrentSessionEpoch(sessionEpoch);
    return sessionEpoch;
  }

  function beginAccessOperation(sessionEpoch) {
    requireCurrentSessionEpoch(sessionEpoch);
    state.accessBusy = true;
    state.accessResult = null;
    render();
  }

  function failAccessOperation(sessionEpoch, title, error, detail = (value) => value.message) {
    markAccessFailureHandled(error);
    if (!sessionOperationIsCurrent(state.sessionEpoch, sessionEpoch, state.sessionStatus)) return;
    setAccessResult("error", title, detail(error));
  }

  function finishAccessOperation(sessionEpoch) {
    if (!sessionOperationIsCurrent(state.sessionEpoch, sessionEpoch, state.sessionStatus)) return;
    state.accessBusy = false;
    render();
  }

  async function buildAccessGrantForRow(row, recipientNpub) {
    const key = openedAccessFolderKey(row);
    return buildFolderKeyGrantRequest({
      vaultId: state.activeVaultId,
      folderId: row.id,
      keyVersion: key.keyVersion,
      rawKey: key.rawKey,
      recipientNpub,
    });
  }

  async function buildVaultPeopleMutationRequest(action, targetNpub) {
    npubToHex(targetNpub);
    return {
      targetNpub,
      accessChangeEvent: await buildAdminAccessChangeEvent({
        action,
        targetNpub,
      }),
    };
  }

  async function mutateVaultPeople(path, options, sessionEpoch) {
    requireCurrentSessionEpoch(sessionEpoch);
    const metadata = await protectedRequest(path, options);
    requireCurrentSessionEpoch(sessionEpoch);
    state.metadata = metadata;
    rememberVisibleVault(metadata);
    try {
      await loadVisibleVaults();
    } catch (error) {
      requireCurrentSessionEpoch(sessionEpoch);
      log("Failed to refresh visible Vaults after Member Identity mutation.", { error: error.message });
    }
    requireCurrentSessionEpoch(sessionEpoch);
    return metadata;
  }

  async function addVaultMemberFromPanel() {
    const sessionEpoch = captureSessionOperationEpoch();
    const vaultId = state.activeVaultId;
    const targetNpub = await normalizedNpubInput("vaultMemberNpubInput", "Enter a Member Identity first");
    requireCurrentSessionEpoch(sessionEpoch);
    beginAccessOperation(sessionEpoch);
    try {
      const body = JSON.stringify(await buildVaultPeopleMutationRequest("add-member", targetNpub));
      requireCurrentSessionEpoch(sessionEpoch);
      await mutateVaultPeople(`/_admin/vaults/${encodeURIComponent(vaultId)}/members`, {
        method: "POST",
        body,
      }, sessionEpoch);
      requireCurrentSessionEpoch(sessionEpoch);
      $("vaultMemberNpubInput").value = "";
      setAccessResult("ready", "Member added", `${identityDisplay(targetNpub)} can now belong to this Vault.`);
      log("Added Vault member.", { targetNpub: identityDisplay(targetNpub), vaultId });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Add member failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function addVaultAdminFromPanel() {
    const sessionEpoch = captureSessionOperationEpoch();
    const vaultId = state.activeVaultId;
    const targetNpub = await normalizedNpubInput("vaultAdminNpubInput", "Enter a Member Identity first");
    requireCurrentSessionEpoch(sessionEpoch);
    beginAccessOperation(sessionEpoch);
    try {
      const body = JSON.stringify(await buildVaultPeopleMutationRequest("add-admin", targetNpub));
      requireCurrentSessionEpoch(sessionEpoch);
      await mutateVaultPeople(`/_admin/vaults/${encodeURIComponent(vaultId)}/admins`, {
        method: "POST",
        body,
      }, sessionEpoch);
      requireCurrentSessionEpoch(sessionEpoch);
      $("vaultAdminNpubInput").value = "";
      setAccessResult("ready", "Admin added", `${identityDisplay(targetNpub)} can manage this Vault.`);
      log("Added Vault admin.", { targetNpub: identityDisplay(targetNpub), vaultId });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Add admin failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function removeVaultMemberFromPanel(targetNpub) {
    const sessionEpoch = captureSessionOperationEpoch();
    const vaultId = state.activeVaultId;
    beginAccessOperation(sessionEpoch);
    try {
      const accessChangeEvent = await buildAdminAccessChangeEvent({
        action: "remove-member",
        targetNpub,
      });
      requireCurrentSessionEpoch(sessionEpoch);
      await mutateVaultPeople(
        `/_admin/vaults/${encodeURIComponent(vaultId)}/members/${encodeURIComponent(targetNpub)}`,
        {
          method: "DELETE",
          body: JSON.stringify({ accessChangeEvent }),
        },
        sessionEpoch
      );
      requireCurrentSessionEpoch(sessionEpoch);
      setAccessResult("warn", "Member removed", `${identityDisplay(targetNpub)} was removed from this Vault.`);
      log("Removed Vault member.", { targetNpub: identityDisplay(targetNpub), vaultId });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Remove member failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function removeVaultAdminFromPanel(targetNpub) {
    const sessionEpoch = captureSessionOperationEpoch();
    const vaultId = state.activeVaultId;
    beginAccessOperation(sessionEpoch);
    try {
      const accessChangeEvent = await buildAdminAccessChangeEvent({
        action: "remove-admin",
        targetNpub,
      });
      requireCurrentSessionEpoch(sessionEpoch);
      await mutateVaultPeople(
        `/_admin/vaults/${encodeURIComponent(vaultId)}/admins/${encodeURIComponent(targetNpub)}`,
        {
          method: "DELETE",
          body: JSON.stringify({ accessChangeEvent }),
        },
        sessionEpoch
      );
      requireCurrentSessionEpoch(sessionEpoch);
      setAccessResult("warn", "Admin removed", `${identityDisplay(targetNpub)} is still a member.`);
      log("Removed Vault admin.", { targetNpub: identityDisplay(targetNpub), vaultId });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Remove admin failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function buildFolderAccessRemovalRequest(keyring, input) {
    if (!keyring) throw new Error("Open this Folder key before removing access");
    const row = input.row;
    const vaultId = input.vaultId || state.activeVaultId;
    const metadata = input.metadata || state.metadata;
    const targetNpub = input.targetNpub;
    npubToHex(targetNpub);
    const currentKeyVersion = row.currentKeyVersion || 1;
    const currentKey = keyring.keys.get(folderKeyId(vaultId, row.id, currentKeyVersion));
    if (!currentKey) throw new Error(`Open the Folder Key for ${row.path} before removing access`);

    const { recipients } = folderAccessRemovalRecipients(metadata, row, targetNpub);
    const newKeyVersion = input.newKeyVersion || currentKeyVersion + 1;
    if (newKeyVersion !== currentKeyVersion + 1) {
      throw new Error("Folder access removal must rotate to the next key version");
    }
    const newRawKey = input.newRawKey || randomFolderKeyBytes();
    if (newRawKey.length !== 32) throw new Error("New Folder Key must be 32 bytes");
    const folderKey = bytesToBase64(newRawKey);
    const createdAtUnix = input.createdAtUnix || Math.floor(Date.now() / 1000);
    const actorNpub = input.actorNpub || currentActorNpub();
    await importFolderKey(keyring, {
      vaultId,
      folderId: row.id,
      keyVersion: newKeyVersion,
      folderKey,
    });

    const grants = [];
    for (const recipientNpub of recipients) {
      grants.push(
        await buildFolderKeyGrantRequest({
          vaultId,
          folderId: row.id,
          keyVersion: newKeyVersion,
          rawKey: newRawKey,
          issuerNpub: actorNpub,
          recipientNpub,
          createdAtUnix,
          brainIdentityProvider: input.brainIdentityProvider,
          encrypt: input.encrypt,
          provider: input.provider,
          signEvent: input.signEvent,
        })
      );
    }

    const reencryptedRecords = [];
    for (const object of liveReadableFolderObjects(input.objects, row.id)) {
      const write = await buildPageWriteRequest(keyring, {
        authorNpub: actorNpub,
        baseRevision: object.revision,
        createdAtUnix,
        folderId: row.id,
        keyVersion: newKeyVersion,
        objectId: object.objectId,
        operation: "update",
        plaintext: encodeFolderObjectPagePlaintext(object.path || `${object.objectId}.md`, object.text),
        signEvent: requireBrainEventAuthorizer("folder-object-revision", input),
        vaultId,
      });
      reencryptedRecords.push({
        objectId: object.objectId,
        ...write,
      });
    }

    const accessChangeEvent = await buildAdminAccessChangeEvent({
      action: "remove-folder-access",
      adminNpub: actorNpub,
      createdAtUnix,
      folderId: row.id,
      keyVersion: newKeyVersion,
      brainIdentityProvider: input.brainIdentityProvider,
      provider: input.provider,
      signEvent: input.signEvent,
      targetNpub,
      vaultId,
    });

    return {
      newKeyVersion,
      grants,
      reencryptedRecords,
      accessChangeEvent,
      folderKey,
      recipientNpubs: recipients,
    };
  }

  async function grantFolderAccessFromPanel(targetValue) {
    const sessionEpoch = captureSessionOperationEpoch();
    const vaultId = state.activeVaultId;
    const row = requireGrantableAccessRow();
    const targetNpub = await normalizedNpubValue(targetValue, "Enter an email first");
    requireCurrentSessionEpoch(sessionEpoch);
    beginAccessOperation(sessionEpoch);
    try {
      const grant = await buildAccessGrantForRow(row, targetNpub);
      requireCurrentSessionEpoch(sessionEpoch);
      const accessChangeEvent = await buildAdminAccessChangeEvent({
        action: "grant-folder-access",
        folderId: row.id,
        keyVersion: row.currentKeyVersion,
        targetNpub,
      });
      requireCurrentSessionEpoch(sessionEpoch);
      const body = JSON.stringify({
        targetNpub,
        grant,
        accessChangeEvent,
      });
      const metadata = await protectedRequest(
        `/_admin/vaults/${encodeURIComponent(vaultId)}/folders/${encodeURIComponent(row.id)}/access`,
        { method: "POST", body }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      state.metadata = metadata;
      const title = row.access === "all_members" ? "Folder key granted" : "Access granted";
      setAccessResult("ready", title, `${identityDisplay(targetNpub)} can open ${row.path}.`, {
        grantId: grant.id,
      });
      log("Granted Folder key/access.", { folderId: row.id, targetNpub: identityDisplay(targetNpub) });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Grant failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function removeFolderAccessFromPanel(targetValue) {
    const sessionEpoch = captureSessionOperationEpoch();
    const vaultId = state.activeVaultId;
    const row = requireRestrictedAccessRow();
    const targetNpub = await normalizedNpubValue(targetValue, "Enter an email first");
    requireCurrentSessionEpoch(sessionEpoch);
    const operationKeyring = cloneSessionKeyring(state.keyring);
    const metadataSnapshot = state.metadata;
    const objectSnapshot = [...state.projection.pages.values()];
    beginAccessOperation(sessionEpoch);
    try {
      const removal = await buildFolderAccessRemovalRequest(operationKeyring, {
        vaultId,
        metadata: metadataSnapshot,
        row,
        targetNpub,
        objects: objectSnapshot,
      });
      requireCurrentSessionEpoch(sessionEpoch);
      const body = JSON.stringify({
        newKeyVersion: removal.newKeyVersion,
        grants: removal.grants,
        reencryptedRecords: removal.reencryptedRecords,
        accessChangeEvent: removal.accessChangeEvent,
      });
      const metadata = await protectedRequest(
        `/_admin/vaults/${encodeURIComponent(vaultId)}/folders/${encodeURIComponent(
          row.id
        )}/access/${encodeURIComponent(targetNpub)}`,
        { method: "DELETE", body }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      state.metadata = metadata;
      state.keyring = operationKeyring;
      await openAvailableFolderKeyGrants();
      requireCurrentSessionEpoch(sessionEpoch);
      await pullSyncBootstrap();
      requireCurrentSessionEpoch(sessionEpoch);
      selectDefaultReaderTargets();
      renderGraphView();
      setAccessResult("warn", "Access removed", `${identityDisplay(targetNpub)} was removed from ${row.path}.`, {
        keyVersion: `v${removal.newKeyVersion}`,
        reencryptedPages: String(removal.reencryptedRecords.length),
      });
      log("Removed restricted Folder access with key rotation.", {
        folderId: row.id,
        keyVersion: removal.newKeyVersion,
        reencryptedPages: removal.reencryptedRecords.length,
        targetNpub: identityDisplay(targetNpub),
      });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Remove failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function createShareLinkFromPanel() {
    const sessionEpoch = captureSessionOperationEpoch();
    const vaultId = state.activeVaultId;
    const row = requireRestrictedAccessRow();
    const recipientNpub = await normalizedNpubInput("accessShareTargetInput", "Enter a Member Identity first");
    requireCurrentSessionEpoch(sessionEpoch);
    beginAccessOperation(sessionEpoch);
    try {
      const expiresAt = shareExpiryIso();
      const grant = await buildAccessGrantForRow(row, recipientNpub);
      requireCurrentSessionEpoch(sessionEpoch);
      const accessChangeEvent = await buildAdminAccessChangeEvent({
        action: "grant-folder-access",
        folderId: row.id,
        keyVersion: row.currentKeyVersion,
        targetNpub: recipientNpub,
      });
      requireCurrentSessionEpoch(sessionEpoch);
      const body = JSON.stringify({
        recipientNpub,
        grant,
        accessChangeEvent,
        expiresAt,
        createPersonalMount: $("accessShareMountInput").checked,
      });
      const shareLink = await protectedRequest(
        `/_admin/vaults/${encodeURIComponent(vaultId)}/folders/${encodeURIComponent(row.id)}/share-links`,
        { method: "POST", body }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      state.lastShareLinkId = shareLink.id;
      $("accessShareLinkInput").value = shareLink.id;
      setAccessResult("ready", "Share link created", `${shareLink.id} is pending for ${identityDisplay(recipientNpub)}.`, {
        acceptPath: shareLink.acceptPath,
        expiresAt: shareLink.expiresAt,
      });
      log("Created Folder share link.", { folderId: row.id, shareLinkId: shareLink.id });
      await refreshFolderShareLinks(row.id);
      requireCurrentSessionEpoch(sessionEpoch);
    } catch (error) {
      failAccessOperation(sessionEpoch, "Share failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function acceptShareLinkFromPanel() {
    const sessionEpoch = captureSessionOperationEpoch();
    const shareLinkId = $("accessShareLinkInput").value.trim() || state.lastShareLinkId;
    if (!shareLinkId) throw new Error("Paste a share link id first");
    beginAccessOperation(sessionEpoch);
    try {
      const shareLink = await protectedRequest(`/_admin/share-links/${encodeURIComponent(shareLinkId)}/accept`, {
        method: "POST",
      });
      requireCurrentSessionEpoch(sessionEpoch);
      state.lastShareLinkId = shareLink.id;
      await loadVaultMetadata();
      requireCurrentSessionEpoch(sessionEpoch);
      const grants = await openAvailableFolderKeyGrants();
      requireCurrentSessionEpoch(sessionEpoch);
      await pullSyncBootstrap();
      requireCurrentSessionEpoch(sessionEpoch);
      selectDefaultReaderTargets();
      setAccessResult(
        "ready",
        shareLink.duplicateAccept ? "Share link already accepted" : "Share link accepted",
        `${shareLink.folderId} is now available to this signer.`,
        {
          mounted: shareLink.personalMountId || "none",
          openedKeys: String(grants.opened.length),
        }
      );
      log("Accepted Folder share link.", { shareLinkId: shareLink.id });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Accept failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function revokeShareLinkFromPanel() {
    const sessionEpoch = captureSessionOperationEpoch();
    const shareLinkId = $("accessShareLinkInput").value.trim() || state.lastShareLinkId;
    if (!shareLinkId) throw new Error("Paste a share link id first");
    beginAccessOperation(sessionEpoch);
    try {
      const shareLink = await protectedRequest(`/_admin/share-links/${encodeURIComponent(shareLinkId)}`, {
        method: "DELETE",
      });
      requireCurrentSessionEpoch(sessionEpoch);
      state.lastShareLinkId = shareLink.id;
      setAccessResult("warn", "Share link revoked", `${shareLink.id} is ${shareLink.status}.`, {
        updatedAt: shareLink.updatedAt,
      });
      log("Revoked Folder share link.", { shareLinkId: shareLink.id });
      await refreshFolderShareLinks(state.folderShareLinksFolderId);
      requireCurrentSessionEpoch(sessionEpoch);
    } catch (error) {
      failAccessOperation(sessionEpoch, "Revoke failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function createVaultInvitationFromPanel() {
    requireUnlockedVaultInvitationAction("creating an invitation");
    const sessionEpoch = state.sessionEpoch;
    const vaultId = state.activeVaultId;
    const metadata = state.metadata;
    const publicBaseUrl = state.config?.publicBaseUrl;
    const targetInput = $("vaultInviteTargetNpubInput").value.trim();
    if (!targetInput) throw new Error("Enter an email address or Member Identity first");
    state.accessBusy = true;
    state.accessResult = null;
    render();
    try {
      let body;
      let localInviteSecret = null;
      let targetLabel = targetInput;
      if (inviteEmailLike(targetInput)) {
        let resolvedNpub = null;
        if (finiteVipEmail(targetInput)) {
          try {
            resolvedNpub = (await resolveIdentityInputValue(targetInput, "Enter an email address or Member Identity first")).npub;
            requireCurrentSessionEpoch(sessionEpoch);
          } catch (error) {
            if (state.sessionEpoch !== sessionEpoch) throw error;
            resolvedNpub = null;
          }
        }
        if (resolvedNpub) {
          body = JSON.stringify(
            buildVaultInvitationRequest({
              targetNpub: resolvedNpub,
              initialFolderAccess: $("vaultInviteFoldersInput").value,
              expiresAt: vaultInvitationExpiryIso(),
            })
          );
          targetLabel = identityDisplay(resolvedNpub);
        } else {
          const sessionKeyring = state.keyring || createSessionKeyring();
          await openAvailableFolderKeyGrants({ keyring: sessionKeyring, vaultId });
          requireCurrentSessionEpoch(sessionEpoch);
          const request = await buildEmailVaultInvitationRequest(sessionKeyring, {
            target: targetInput,
            metadata,
            initialFolderAccess: $("vaultInviteFoldersInput").value,
            expiresAt: vaultInvitationExpiryIso(),
            vaultId,
          });
          requireCurrentSessionEpoch(sessionEpoch);
          body = JSON.stringify(request.body);
          localInviteSecret = request.inviteSecret;
          targetLabel = canonicalInviteEmail(targetInput);
          state.keyring = sessionKeyring;
        }
      } else {
        const targetNpub = await normalizedNpubInput("vaultInviteTargetNpubInput", "Enter an email address or Member Identity first");
        requireCurrentSessionEpoch(sessionEpoch);
        body = JSON.stringify(
          buildVaultInvitationRequest({
            targetNpub,
            initialFolderAccess: $("vaultInviteFoldersInput").value,
            expiresAt: vaultInvitationExpiryIso(),
          })
        );
        targetLabel = identityDisplay(targetNpub);
      }
      requireCurrentSessionEpoch(sessionEpoch);
      const invitation = await protectedRequest(
        vaultInvitationCreatePath(vaultId),
        { method: "POST", body }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      rememberVaultInvitationSelection(invitation);
      if (localInviteSecret && invitation.targetKind === "email_bootstrap") {
        const invitedEmail = invitation.invitedEmail || canonicalInviteEmail(targetInput);
        state.lastEmailInviteSecret = localInviteSecret;
        state.lastEmailInviteUrl = emailInviteClientUrl({
          publicBaseUrl,
          inviteCode: invitation.inviteCode,
          invitedEmail,
          inviteSecret: localInviteSecret,
        });
        $("vaultInviteSecretInput").value = localInviteSecret;
        $("vaultInviteEmailInput").value = invitedEmail;
        $("vaultInviteUrlInput").value = state.lastEmailInviteUrl;
      } else {
        clearRememberedEmailInvitationMaterial();
      }
      const invitationAccessDetail = invitation.targetKind === "email_bootstrap"
        ? "They can claim the encrypted Folder Key Grants in the invitation scope after proving the invited email."
        : "They can join with this one-time invite; grant any required Folder Keys after they join.";
      setAccessResult("ready", "Invitation created", `${targetLabel} can join ${invitation.vaultId}. ${invitationAccessDetail}`, {
        inviteCode: invitation.inviteCode,
        invitationId: invitation.id,
        acceptPath: invitation.acceptPath,
        publicInstructions: invitation.publicInstructionsUrl || invitation.publicInstructionsPath || "none",
        expiresAt: invitation.expiresAt,
        target: invitation.invitedEmail || targetLabel,
        delivery: invitation.deliveryStatus || "manual",
      });
      log("Created Vault invitation.", {
        invitationId: invitation.id,
        targetKind: invitation.targetKind,
        vaultId: invitation.vaultId,
      });
      await refreshVaultAdminLists();
    } catch (error) {
      markAccessFailureHandled(error);
      if (state.sessionEpoch === sessionEpoch) {
        setAccessResult("error", "Invite failed", vaultInvitationUnavailableDetail(error));
      }
      throw error;
    } finally {
      if (state.sessionEpoch === sessionEpoch) {
        state.accessBusy = false;
        render();
      }
    }
  }

  async function inspectVaultInvitationFromPanel() {
    requireUnlockedVaultInvitationAction("inspecting an invitation");
    const sessionEpoch = captureSessionOperationEpoch();
    const code = currentVaultInvitationCode();
    beginAccessOperation(sessionEpoch);
    try {
      const invitation = await protectedRequest(vaultInvitationLinkPath(code));
      requireCurrentSessionEpoch(sessionEpoch);
      rememberVaultInvitationSelection(invitation);
      setAccessResult("ready", "Invitation loaded", `${identityDisplay(invitation.userId)} is ${invitation.status}.`, {
        vaultId: invitation.vaultId,
        invitationId: invitation.id,
        acceptPath: invitation.acceptPath,
        "target email": identityDisplay(invitation.userId),
        signer: state.pubkeyHex ? "connected" : "none",
      });
      log("Loaded Vault invitation.", { invitationId: invitation.id, vaultId: invitation.vaultId });
      return invitation;
    } catch (error) {
      failAccessOperation(sessionEpoch, "Inspect failed", error, vaultInvitationUnavailableDetail);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function loadEmailInviteInstructionsFromPanel() {
    requireUnlockedVaultInvitationAction("verifying email and loading invitation access");
    const sessionEpoch = captureSessionOperationEpoch();
    const code = currentVaultInvitationCode();
    const email = canonicalInviteEmail($("vaultInviteEmailInput").value);
    const inviteSecret = $("vaultInviteSecretInput").value.trim();
    if (!inviteSecret) throw new Error("Paste the client-only Invite Secret first");
    beginAccessOperation(sessionEpoch);
    try {
      const body = JSON.stringify({
        email,
        emailProofCreatedAt: emailProofCreatedAtIso(),
      });
      const invitation = await protectedRequest(emailInviteBootstrapPath(code), {
        method: "POST",
        body,
      });
      requireCurrentSessionEpoch(sessionEpoch);
      await openEmailInviteBootstrap(invitation, {
        inviteSecret,
        invitedEmail: email,
      });
      requireCurrentSessionEpoch(sessionEpoch);
      rememberVaultInvitationSelection(invitation);
      state.lastEmailInvitePostProof = invitation;
      const folderScope = (invitation.bootstrapScope || [])
        .map((folder) => `${folder.folderId} v${folder.keyVersion}`)
        .join(", ");
      setAccessResult("ready", "Email verified", `${email} can claim encrypted Folder Key Grants for ${invitation.vaultId}.`, {
        inviteCode: invitation.inviteCode,
        scope: folderScope || "none",
        status: invitation.status,
      });
      log("Verified email invitation scope.", {
        invitationId: invitation.id,
        vaultId: invitation.vaultId,
      });
      return invitation;
    } catch (error) {
      failAccessOperation(sessionEpoch, "Email verification failed", error, vaultInvitationUnavailableDetail);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function acceptVaultInvitationFromPanel() {
    requireUnlockedVaultInvitationAction("accepting an invitation");
    const code = currentVaultInvitationCode();
    const email = $("vaultInviteEmailInput")?.value.trim();
    const inviteSecret = $("vaultInviteSecretInput")?.value.trim();
    if (email || inviteSecret) {
      return claimEmailVaultInvitationFromPanel(code);
    }
    const sessionEpoch = captureSessionOperationEpoch();
    beginAccessOperation(sessionEpoch);
    try {
      const invitation = await protectedRequest(vaultInvitationAcceptPath(code), {
        method: "POST",
      });
      requireCurrentSessionEpoch(sessionEpoch);
      setActiveVaultId(invitation.vaultId);
      state.sessionNotice = invitation.duplicateAccept
        ? "This Member Identity already joined the selected Vault. An admin must grant any required Folder Keys before encrypted content can open."
        : "Joined the selected Vault. An admin must grant any required Folder Keys before encrypted content can open.";
      render();
      log("Accepted Vault invitation.", { invitationId: invitation.id, vaultId: invitation.vaultId });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Accept failed", error, vaultInvitationUnavailableDetail);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function claimEmailVaultInvitationFromPanel(code) {
    requireUnlockedVaultInvitationAction("claiming encrypted Folder Key Grants");
    const sessionEpoch = captureSessionOperationEpoch();
    const email = canonicalInviteEmail($("vaultInviteEmailInput").value);
    const inviteSecret = $("vaultInviteSecretInput").value.trim();
    if (!inviteSecret) throw new Error("Paste the client-only Invite Secret first");
    const invitationSnapshot = state.lastEmailInvitePostProof;
    const operationKeyring = cloneSessionKeyring(state.keyring);
    beginAccessOperation(sessionEpoch);
    try {
      const proofCreatedAt = emailProofCreatedAtIso();
      const proofBody = JSON.stringify({
        email,
        emailProofCreatedAt: proofCreatedAt,
      });
      const invitation =
        invitationSnapshot?.inviteCode === code
          ? invitationSnapshot
          : await protectedRequest(emailInviteBootstrapPath(code), {
              method: "POST",
              body: proofBody,
            });
      requireCurrentSessionEpoch(sessionEpoch);
      const claimantNpub = currentActorNpub();
      const claimRequest = await buildEmailInviteClaimRequest({
        claimantNpub,
        email,
        emailProofCreatedAt: proofCreatedAt,
        invitation,
        inviteSecret,
        keyring: operationKeyring,
      });
      requireCurrentSessionEpoch(sessionEpoch);
      const claimed = await protectedRequest(emailInviteClaimPath(code), {
        method: "POST",
        body: JSON.stringify(claimRequest.body),
      });
      requireCurrentSessionEpoch(sessionEpoch);
      setActiveVaultId(claimed.vaultId);
      state.sessionNotice = claimed.duplicateAccept
        ? "Email invitation was already claimed. Unlock the session to open the selected Vault."
        : "Email invitation claimed. Unlock the session to open the selected Vault.";
      render();
      log("Claimed email Vault invitation.", {
        invitationId: claimed.id,
        vaultId: claimed.vaultId,
      });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Claim failed", error, vaultInvitationUnavailableDetail);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function revokeVaultInvitationFromPanel() {
    requireUnlockedVaultInvitationAction("revoking an invitation");
    const sessionEpoch = captureSessionOperationEpoch();
    const value = currentVaultInvitationInput();
    const target = vaultInvitationRevokeTarget({
      activeVaultId: state.activeVaultId,
      input: value,
      invitations: state.vaultInvitations,
      lastVaultInvitationCode: state.lastVaultInvitationCode,
      lastVaultInvitationId: state.lastVaultInvitationId,
    });
    beginAccessOperation(sessionEpoch);
    try {
      const invitation = await protectedRequest(
        vaultInvitationRevokePath(target.vaultId, target.invitationId),
        { method: "DELETE" }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      rememberVaultInvitationSelection(invitation);
      setAccessResult("warn", "Invitation revoked", `${invitation.id} is ${invitation.status}.`, {
        updatedAt: invitation.updatedAt,
      });
      log("Revoked Vault invitation.", { invitationId: invitation.id, vaultId: invitation.vaultId });
      await refreshVaultAdminLists();
      requireCurrentSessionEpoch(sessionEpoch);
    } catch (error) {
      failAccessOperation(sessionEpoch, "Revoke failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function prepareDraftWrite(options = {}) {
    if (!state.keyring) throw new Error("Open a Folder Key before encrypting a Page draft");
    if (!state.pubkeyHex) throw new Error("Connect a signer before preparing a signed Page write");
    const sessionEpoch = state.sessionEpoch;
    const keyring = state.keyring;
    const vaultId = state.activeVaultId;
    const input = activePageInput();
    const authorNpub = npubFromHex(state.pubkeyHex);
    const keyVersion = currentFolderKeyVersion(input.folderId);
    const preparedWrite = await buildPageWriteRequest(keyring, {
      authorNpub,
      baseRevision: input.baseRevision,
      folderId: input.folderId,
      keyVersion,
      objectId: input.objectId,
      plaintext: encodeFolderObjectPagePlaintext(input.path, input.text),
      signEvent: requireBrainEventAuthorizer("folder-object-revision"),
      vaultId,
    });
    requireCurrentSessionEpoch(sessionEpoch);
    state.preparedWrite = preparedWrite;
    state.preparedWriteTarget = {
      folderId: input.folderId,
      objectId: input.objectId,
      path: input.path,
    };
    state.projection.localDrafts.set(pageKey(input.folderId, input.objectId), {
      baseRevision: state.preparedWrite.baseRevision || 0,
      path: input.path,
      text: input.text,
    });
    log("Encrypted Page draft and prepared signed revision request.", {
      folderId: input.folderId,
      objectId: input.objectId,
      baseRevision: state.preparedWrite.baseRevision,
      keyVersion,
    });
    if (options.renderAfter !== false) render();
    return preparedWrite;
  }

  async function savePreparedPage() {
    if (!state.preparedWrite) throw new Error("Prepare a Page write before saving");
    const sessionEpoch = state.sessionEpoch;
    const vaultId = state.activeVaultId;
    const preparedWrite = state.preparedWrite;
    const savedInput = activePageInput();
    const target = state.preparedWriteTarget || savedInput;
    const savedText = savedInput.text;
    const savedPath = target.path || savedInput.path || `${target.objectId}.md`;
    const path = `/_admin/vaults/${encodeURIComponent(vaultId)}/folders/${encodeURIComponent(
      target.folderId
    )}/objects/${encodeURIComponent(target.objectId)}`;
    const result = await protectedRequest(path, {
      method: "PUT",
      body: JSON.stringify(preparedWrite),
    });
    requireCurrentSessionEpoch(sessionEpoch);
    state.projection.pages.set(pageKey(target.folderId, target.objectId), {
      folderId: target.folderId,
      objectId: target.objectId,
      revision: result.revision,
      path: savedPath,
      status: "ready",
      text: savedText,
      title: pageTitleFromText(savedText, pageTitleFromPath(savedPath, target.objectId)),
    });
    state.projection.localDrafts.delete(pageKey(target.folderId, target.objectId));
    state.preparedWrite = null;
    state.preparedWriteTarget = null;
    $("pageBaseRevisionInput").value = String(result.revision);
    setEditorDraftText(savedText);
    log("Saved encrypted Page revision.", result);
    render();
  }

  async function saveActivePage() {
    const { key } = activePageKeyFromInputs();
    if (state.pageSaveInFlight) throw new Error("A Page save is already in progress");
    const operation = { key, sessionEpoch: state.sessionEpoch };
    state.pageSaveInFlight = operation;
    updateSaveControls();
    try {
      await prepareDraftWrite({ renderAfter: false });
      await savePreparedPage();
    } finally {
      if (state.pageSaveInFlight === operation) state.pageSaveInFlight = null;
      updateSaveControls();
    }
  }

  async function pullSyncBootstrap() {
    const sessionEpoch = state.sessionEpoch;
    const keyring = state.keyring;
    const projection = state.projection;
    const vaultId = state.activeVaultId;
    const path = `/_admin/vaults/${encodeURIComponent(vaultId)}/sync/bootstrap`;
    const sync = await protectedRequest(path);
    requireCurrentSessionEpoch(sessionEpoch);
    const openedSync = await openSyncObjects(keyring, sync);
    requireCurrentSessionEpoch(sessionEpoch);
    state.projection = mergeSyncProjection(projection, openedSync);
    log("Pulled sync bootstrap into local projection.", {
      conflicts: state.projection.conflicts,
      decryptedPages: openedSync.objects.filter((object) => object.status === "ready").length,
      pages: state.projection.pages.size,
      seenEvents: state.projection.seenEventIds.size,
    });
    render();
  }

  function renderGraphView() {
    const pages = decryptedPagesForGraph();
    const graph = buildGraphProjection(pages);
    drawGraph(graph, { readablePageCount: pages.length });
    setGraphStats(graph);
    updateGraphFullscreenControl();
    log("Rendered graph from decrypted client index.", {
      edges: graph.edges.length,
      nodes: graph.nodes.length,
    });
  }

  function fitGraphView() {
    setGraphZoom(1);
    log("Reset graph zoom.");
  }

  function bind() {
    window.addEventListener?.("pagehide", handlePageHide);
    window.addEventListener?.("pageshow", handlePageShow);
    window.addEventListener?.("finite:account-session-ended", expireBrainIdentitySession);
    onOptionalClick("lockSessionButton", () => {
      lockSession();
    });
    onOptionalClick("resumeSessionButton", () => {
      resumeSession().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to unlock Product Client session.", { error: error.message });
        render();
      });
    });
    $("sessionSettingsButton")?.addEventListener("click", () => {
      openSettingsModal("session");
    });
    $("sessionAccountVaultButton")?.addEventListener("click", () => {
      openVaultSwitcher();
    });
    $("manageVaultsButton")?.addEventListener("click", () => {
      openManageVaultsModal();
    });
    $("closeManageVaultsButton")?.addEventListener("click", () => {
      closeManageVaultsModal();
    });
    $("manageVaultsModal")?.addEventListener("click", (event) => {
      if (event.target === $("manageVaultsModal")) closeManageVaultsModal();
    });
    $("manageVaultsConnectSignerButton")?.addEventListener("click", () => {
      connectSigner().catch((error) => {
        state.lastError = error.message;
        log("Failed to connect signer from Manage Vaults.", { error: error.message });
        render();
      });
    });
    $("manageVaultsLoadButton")?.addEventListener("click", () => {
      manageVaultsLoadAction();
    });
    $("manageCreateOrganizationVaultButton")?.addEventListener("click", () => {
      createOrganizationVaultFromInput("manageOrganizationVaultNameInput").catch((error) => {
        state.lastError = error.message;
        log("Failed to create organization Vault from Manage Vaults.", { error: error.message });
        render();
      });
    });
    $("manageOrganizationVaultNameInput")?.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") return;
      event.preventDefault();
      $("manageCreateOrganizationVaultButton")?.click?.();
    });
    $("settingsConnectSignerButton")?.addEventListener("click", () => {
      connectSigner().catch((error) => {
        state.lastError = error.message;
        log("Failed to connect signer from Settings.", { error: error.message });
        render();
      });
    });
    $("settingsManageVaultsButton")?.addEventListener("click", () => {
      openManageVaultsModal({ returnToSettings: true });
    });
    $("closeSettingsButton")?.addEventListener("click", () => {
      closeSettingsModal();
    });
    $("settingsNavSession")?.addEventListener("click", () => {
      setSettingsSection("session");
    });
    $("settingsNavVault")?.addEventListener("click", () => {
      setSettingsSection("vault");
    });
    $("settingsNavAccess")?.addEventListener("click", () => {
      setSettingsSection("access");
    });
    $("settingsNavInvitations")?.addEventListener("click", () => {
      setSettingsSection("invitations");
    });
    $("settingsModal")?.addEventListener("click", (event) => {
      if (event.target === $("settingsModal")) closeSettingsModal();
    });
    $("settingsNav")?.addEventListener("keydown", (event) => {
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "Home" && event.key !== "End") return;
      const buttons = [
        $("settingsNavSession"),
        $("settingsNavVault"),
        $("settingsNavAccess"),
        $("settingsNavInvitations"),
      ].filter(Boolean);
      const activeIndex = buttons.findIndex((button) => button.getAttribute("aria-selected") === "true");
      if (activeIndex < 0) return;
      event.preventDefault();
      if (event.key === "Home") {
        setSettingsSection("session");
        return;
      }
      if (event.key === "End") {
        setSettingsSection("invitations");
        return;
      }
      const direction = event.key === "ArrowDown" ? 1 : -1;
      const nextIndex = (activeIndex + direction + buttons.length) % buttons.length;
      setSettingsSection(["session", "vault", "access", "invitations"][nextIndex] || "session");
    });
    bindAccessFolderSelector();
    $("refreshReaderButton").addEventListener("click", () => {
      refreshReader().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to refresh Vault reader.", { error: error.message });
        state.readerBusy = false;
        render();
      });
    });
    onOptionalClick("savePageButton", () => {
      if (!canSaveActiveDraft()) return;
      saveActivePage().catch((error) => {
        state.lastError = error.message;
        log("Failed to save Page.", { error: error.message });
        render();
      });
    });
    $("ribbonGraphButton").addEventListener("click", () => {
      setWorkspaceView("graph");
    });
    $("ribbonFilesButton").addEventListener("click", () => {
      setWorkspaceView("page");
      setSidebarMode("files");
    });
    $("ribbonSearchButton").addEventListener("click", () => {
      setSidebarMode("search");
    });
    $("ribbonCommandButton").addEventListener("click", () => {
      if (state.commandPaletteOpen) {
        closeCommandPalette();
      } else {
        openCommandPalette();
      }
    });
    $("ribbonAccessButton").addEventListener("click", () => {
      openSettingsModal("access");
    });
    onOptionalClick("addVaultMemberButton", () => {
      addVaultMemberFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to add Vault member.", { error: error.message });
      });
    });
    onOptionalClick("addVaultAdminButton", () => {
      addVaultAdminFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to add Vault admin.", { error: error.message });
      });
    });
    onOptionalClick("pairAgentWorkspaceButton", () => {
      pairAgentWorkspaceFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to pair Agent Principal.", { error: error.message });
      });
    });
    for (const [inputId, buttonId] of [
      ["vaultMemberNpubInput", "addVaultMemberButton"],
      ["vaultAdminNpubInput", "addVaultAdminButton"],
      ["agentWorkspaceNpubInput", "pairAgentWorkspaceButton"],
    ]) {
      const input = $(inputId);
      if (!input) continue;
      input.addEventListener("keydown", (event) => {
        if (event.key !== "Enter") return;
        event.preventDefault();
        const button = $(buttonId);
        if (!button?.disabled) button.click();
      });
    }
    onOptionalClick("createShareLinkButton", () => {
      createShareLinkFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to create Folder share link.", { error: error.message });
      });
    });
    onOptionalClick("acceptShareLinkButton", () => {
      acceptShareLinkFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to accept Folder share link.", { error: error.message });
      });
    });
    onOptionalClick("revokeShareLinkButton", () => {
      revokeShareLinkFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to revoke Folder share link.", { error: error.message });
      });
    });
    onOptionalClick("createVaultInvitationButton", () => {
      createVaultInvitationFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to create Vault invitation.", { error: error.message });
      });
    });
    onOptionalClick("copyVaultInviteUrlButton", () => {
      void copyVaultInviteUrl();
    });
    onOptionalClick("getVaultInvitationButton", () => {
      inspectVaultInvitationFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to inspect Vault invitation.", { error: error.message });
      });
    });
    onOptionalClick("getEmailInviteInstructionsButton", () => {
      loadEmailInviteInstructionsFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to load email Vault invitation scope.", { error: error.message });
      });
    });
    onOptionalClick("acceptVaultInvitationButton", () => {
      acceptVaultInvitationFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to accept Vault invitation.", { error: error.message });
      });
    });
    onOptionalClick("vaultInviteConnectSignerButton", () => {
      connectSigner().catch((error) => {
        state.lastError = error.message;
        log("Failed to connect signer for Vault invitation.", { error: error.message });
        render();
      });
    });
    onOptionalClick("revokeVaultInvitationButton", () => {
      revokeVaultInvitationFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to revoke Vault invitation.", { error: error.message });
      });
    });
    for (const inputId of [
      "accessShareTargetInput",
      "accessShareExpiresAtInput",
      "accessShareLinkInput",
      "vaultInviteTargetNpubInput",
      "vaultInviteFoldersInput",
      "vaultInviteExpiresAtInput",
      "vaultInviteCodeInput",
      "vaultInviteEmailInput",
      "vaultInviteEmailProofCreatedAtInput",
      "vaultInviteSecretInput",
    ]) {
      bindPrimaryFormAction(inputId);
    }
    for (const inputId of [
      "vaultInviteCodeInput",
      "vaultInviteEmailInput",
      "vaultInviteEmailProofCreatedAtInput",
      "vaultInviteSecretInput",
    ]) {
      const input = $(inputId);
      if (input) input.addEventListener("input", () => handleVaultInvitationInput(inputId));
    }
    $("sidebarSearchInput").addEventListener("input", () => {
      if (state.searchHighlight) {
        const query = $("sidebarSearchInput").value.trim();
        state.searchHighlight = query ? { ...state.searchHighlight, query } : null;
        state.searchHighlightShouldScroll = Boolean(query);
        render();
        return;
      }
      renderSearchPanel();
    });
    $("commandPaletteInput").addEventListener("input", () => {
      state.commandPaletteSelectedIndex = 0;
      renderCommandPalette();
    });
    $("commandPaletteInput").addEventListener("keydown", (event) => {
      if (event.isComposing || event.keyCode === 229) return;
      const rows = commandPaletteRows($("commandPaletteInput").value);
      const currentIndex = commandPaletteSelectionIndex(rows);
      const nextIndex = keyboardListNavigationIndex(event.key, currentIndex, rows.length);
      if (nextIndex !== null) {
        event.preventDefault();
        state.commandPaletteSelectedIndex = nextIndex;
        renderCommandPalette();
        $("commandPaletteOption-" + nextIndex)?.scrollIntoView?.({ block: "nearest" });
        return;
      }
      if (event.key !== "Enter") return;
      event.preventDefault();
      runCommandPaletteRow(rows[currentIndex]);
    });
    $("closeCommandPaletteButton").addEventListener("click", () => {
      closeCommandPalette();
    });
    $("commandPalette").addEventListener("click", (event) => {
      if (event.target === $("commandPalette")) closeCommandPalette();
    });
    $("obsidianNewPageButton").addEventListener("click", () => {
      startNewPageDraft();
    });
    $("obsidianNewFolderButton").addEventListener("click", () => {
      createFolderFromToolbar().catch((error) => {
        state.lastError = error.message;
        log("Failed to create Folder from toolbar.", { error: error.message });
        render();
      });
    });
    $("readerPageContent").addEventListener("input", (event) => {
      if (event.target?.matches?.("input[data-task-index]")) return;
      if (visualEditorElement()?.getAttribute?.("contenteditable") === "true") {
        syncDraftFromVisualEditor({ remember: true });
        refreshEditorSlashMenu();
      }
    });
    $("readerPageContent").addEventListener("change", (event) => {
      updateActiveTaskDraft(event.target);
    });
    $("readerPageContent").addEventListener("click", (event) => {
      activatePageContentLink(event);
    });
    $("readerPageContent").addEventListener("keydown", (event) => {
      if (handlePageContentLinkKeydown(event)) return;
      handleEditorSlashKeydown(event);
    });
    $("pageDraftInput").addEventListener("input", () => {
      rememberActiveDraft($("pageDraftInput").value);
    });
    $("editorDrawer").addEventListener("toggle", () => {
      setEditorMode($("editorDrawer").open ? "markdown" : "visual");
    });
    $("zoomInGraphButton").addEventListener("click", () => {
      zoomGraphView(1);
    });
    $("zoomOutGraphButton").addEventListener("click", () => {
      zoomGraphView(-1);
    });
    $("fitGraphButton").addEventListener("click", () => {
      fitGraphView();
    });
    $("fullscreenGraphButton").addEventListener("click", () => {
      toggleGraphFullscreen();
    });
    document.addEventListener("fullscreenchange", updateGraphFullscreenControl);
    document.addEventListener("click", (event) => {
      const menu = $("contextMenu");
      if (!menu.hidden && !menu.contains(event.target)) closeContextMenu();
      const vaultSwitcher = $("vaultSwitcherMenu");
      const vaultSwitcherTrigger = $("sessionAccountVaultButton");
      if (
        state.vaultSwitcherOpen &&
        vaultSwitcher &&
        !vaultSwitcher.contains(event.target) &&
        !vaultSwitcherTrigger?.contains?.(event.target)
      ) {
        closeVaultSwitcher();
      }
      const slashMenu = $("editorSlashMenu");
      if (state.editorSlashOpen && slashMenu && !slashMenu.contains(event.target) && !visualEditorElement()?.contains(event.target)) {
        closeEditorSlashMenu();
      }
    });
    document.addEventListener("selectionchange", () => {
      if (state.editorSlashOpen) refreshEditorSlashMenu();
    });
    document.addEventListener("keydown", (event) => {
      if (state.manageVaultsModalOpen) {
        if (event.key === "Escape") {
          event.preventDefault();
          closeManageVaultsModal();
          return;
        }
        if (event.key === "Tab") {
          const focusable = manageVaultsModalFocusableElements();
          if (!focusable.length) {
            event.preventDefault();
            return;
          }
          const first = focusable[0];
          const last = focusable[focusable.length - 1];
          if (event.shiftKey && document.activeElement === first) {
            event.preventDefault();
            last.focus();
          } else if (!event.shiftKey && document.activeElement === last) {
            event.preventDefault();
            first.focus();
          }
          return;
        }
        return;
      }
      if (state.vaultSwitcherOpen) {
        if (event.key === "Escape") {
          event.preventDefault();
          closeVaultSwitcher();
          return;
        }
        if (event.key === "Tab") {
          event.preventDefault();
          moveVaultSwitcherFocusOut({ backwards: event.shiftKey });
          return;
        }
        const direction =
          event.key === "ArrowDown" ? 1 :
          event.key === "ArrowUp" ? -1 :
          event.key === "Home" ? 0 :
          event.key === "End" ? Number.POSITIVE_INFINITY :
          null;
        if (direction !== null) {
          const items = vaultSwitcherFocusableElements();
          if (!items.length) return;
          event.preventDefault();
          const currentIndex = Math.max(0, items.indexOf(document.activeElement));
          const nextIndex = direction === Number.POSITIVE_INFINITY
            ? items.length - 1
            : direction === 0
              ? 0
              : (currentIndex + direction + items.length) % items.length;
          items[nextIndex]?.focus?.();
        }
        return;
      }
      if (handleCommandPaletteKeydown(event)) return;
      if (handleContextMenuKeydown(event)) return;
      if (state.settingsModalOpen) {
        if (event.key === "Escape") {
          event.preventDefault();
          closeSettingsModal();
          return;
        }
        if (event.key === "Tab") {
          const focusable = settingsModalFocusableElements();
          if (!focusable.length) {
            event.preventDefault();
            return;
          }
          const first = focusable[0];
          const last = focusable[focusable.length - 1];
          if (event.shiftKey && document.activeElement === first) {
            event.preventDefault();
            last.focus();
          } else if (!event.shiftKey && document.activeElement === last) {
            event.preventDefault();
            first.focus();
          }
          return;
        }
        return;
      }
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "p") {
        event.preventDefault();
        openCommandPalette();
        return;
      }
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        if (!canSaveActiveDraft()) return;
        saveActivePage().catch((error) => {
          state.lastError = error.message;
          log("Failed to save Page.", { error: error.message });
          render();
        });
        return;
      }
      if (event.key === "Escape") {
        closeContextMenu();
        closeCommandPalette();
        closeEditorSlashMenu();
      }
    });
  }

  function inviteNavigationFromHash(hash) {
    const params = new URLSearchParams(String(hash || "").replace(/^#/, ""));
    const hasInviteNavigation =
      params.has("invite") ||
      params.has("inviteCode") ||
      params.has("inviteEmail") ||
      params.has("inviteSecret");
    if (!hasInviteNavigation) return null;
    const inviteCode = params.get("inviteCode") || params.get("code") || null;
    const rawEmail = params.get("inviteEmail") || params.get("email");
    let inviteEmail = null;
    if (rawEmail) {
      try {
        inviteEmail = canonicalInviteEmail(rawEmail);
      } catch (_) {
        return null;
      }
    }
    const inviteSecret = params.get("inviteSecret") || null;
    if (!inviteCode && !inviteEmail && !inviteSecret) return null;
    return { inviteCode, inviteEmail, inviteSecret };
  }

  function populateInviteFromHash() {
    const hash = String(window.location?.hash || "");
    const params = new URLSearchParams(hash.replace(/^#/, ""));
    const hasInviteNavigation =
      params.has("invite") ||
      params.has("inviteCode") ||
      params.has("inviteEmail") ||
      params.has("inviteSecret");
    if (!hasInviteNavigation) return false;
    pendingInviteNavigation = null;
    const parsedInviteNavigation = inviteNavigationFromHash(hash);
    const cleanUrl = `${window.location.pathname || ""}${window.location.search || ""}`;
    const fallbackUrl = cleanUrl || window.location.href.split("#")[0];
    let fragmentRemoved = false;
    try {
      if (typeof window.history?.replaceState === "function") {
        window.history.replaceState(null, "", fallbackUrl);
        fragmentRemoved = true;
      }
    } catch (_) {
      fragmentRemoved = false;
    }
    if (!fragmentRemoved) {
      try {
        window.location?.replace?.(fallbackUrl);
      } catch (_) {
        // The secret is still discarded even when the browser refuses URL cleanup.
      }
      state.lastError =
        "Invitation link could not be cleared safely. Its client-only secret was discarded.";
      return false;
    }
    if (!parsedInviteNavigation) {
      state.lastError = "Invitation link is incomplete or invalid";
      return false;
    }
    pendingInviteNavigation = parsedInviteNavigation;
    return true;
  }

  function applyPendingInviteNavigation() {
    const pending = pendingInviteNavigation;
    pendingInviteNavigation = null;
    if (!pending) return false;
    let populated = false;
    if (pending.inviteCode) {
      rememberVaultInvitationSelection({ inviteCode: pending.inviteCode });
      populated = true;
    }
    if (pending.inviteEmail) {
      if ($("vaultInviteEmailInput")) $("vaultInviteEmailInput").value = pending.inviteEmail;
      populated = true;
    }
    if (pending.inviteSecret) {
      state.lastEmailInviteSecret = pending.inviteSecret;
      if ($("vaultInviteSecretInput")) $("vaultInviteSecretInput").value = pending.inviteSecret;
      populated = true;
    }
    if (!populated) return false;
    state.settingsSection = "invitations";
    state.settingsModalOpen = true;
    return true;
  }

  async function start(options = {}) {
    if (Object.prototype.hasOwnProperty.call(options, "identityProvider")) {
      configureBrainIdentityProvider(options.identityProvider);
    } else if (!state.identityProvider) {
      configureBrainIdentityProvider(createHostedBrainIdentityProvider());
    }
    mountAccessPanelInSettings();
    mountInvitationPanelInSettings();
    bind();
    setEditorDraftText($("pageDraftInput").value);
    populateInviteFromHash();
    await loadConfig();
    await detectSigner();
  }

  return {
    accessActionRoute,
    accessBadgesForFolder,
    accessIntentValue,
    accessPanelState,
    accessPeopleSummary,
    agentWorkspacePairingRows,
    agentWorkspacePairingsPath,
    buildAgentWorkspacePairingRequest,
    adminAccessChangeTags,
    buildAdminAccessChangeEvent,
    buildFolderKeyGrantRequest,
    buildPageDeleteRequest,
    buildPageWriteRequest,
    buildAuthEventTemplate,
    buildBrainAuthorizationHeader,
    buildDefaultVaultPageWrites,
    buildFolderAccessRemovalRequest,
    buildEmailInviteAuthorizationEvent,
    buildEmailInviteClaimProofEvent,
    buildEmailInviteClaimRequest,
    buildEmailVaultInvitationRequest,
    buildVaultInvitationRequest,
    buildVaultBootstrapPlan,
    buildGraphProjection,
    canonicalAdminAccessChangePayload,
    canonicalEmailInviteAuthorizationPayload,
    canonicalInviteEmail,
    clearSessionSecretsAndPlaintext,
    commandPaletteCommands,
    commandPaletteRows,
    contextMenuItemsForTarget,
    cloneSessionKeyring,
    createClientProjection,
    createLocalNip07ProviderFromSecret,
    createHostedBrainIdentityProvider,
    createNip07BrainIdentityProvider,
    configureBrainIdentityProvider,
    connectBrainIdentityProvider,
    createSessionKeyring,
    deriveSignerState,
    deriveBrainIdentityProviderState,
    expireBrainIdentitySession,
    discardLocalPageDraft,
    defaultVaultBootstrapFolderIds,
    defaultVaultPages,
    defaultVaultPagesFolderId,
    emailInviteAuthorizationTags,
    emailInviteBootstrapPath,
    emailInviteClaimPath,
    emailInviteClientUrl,
    emailInviteInstructionsPath,
    emailInviteScope,
    emailInviteScopeJson,
    ensureAgentWorkspacePairing,
    decodeFolderObjectPlaintext,
    encryptFolderObject,
    encodeFolderObjectAssetPlaintext,
    encodeFolderObjectPagePlaintext,
    editorSlashCommandRows,
    extractPageLinks,
    folderAllowsDirectGrant,
    folderCreationHierarchy,
    folderCreationParent,
    folderShareLinkRows,
    graphEmptyStateCopy,
    graphLayout,
    graphNeighborIds,
    graphStats,
    graphViewBoxForZoom,
    handlePageHide,
    handlePageShow,
    inlineLinkSegments,
    initialVaultInvitationFolders,
    isActiveVaultAuthorizationLoss,
    applyPendingInviteNavigation,
    inviteNavigationFromHash,
    inviteUnwrapKeypairFromSecret,
    nip44DecryptWithSecret,
    nip44EncryptWithSecret,
    markdownFromEditorElement,
    markdownPreviewBlocks,
    mergeSyncProjection,
    metadataVaultRole,
    metadataFolderRows,
    metadataMountRows,
    nextDraftObjectId,
    normalizeSidebarMode,
    normalizeSettingsSection,
    normalizeVisibleVault,
    npubFromHex,
    npubToHex,
    openFolderKeyGrants,
    openEmailInviteBootstrap,
    openDevelopmentFolderKeyGrants,
    openFolderKeyGrantPlaintext,
    openFolderObject,
    openSyncObjects,
    parseOkfBundle,
    pageDeletionDisposition,
    pageKeyForReference,
    pageLinkContext,
    pageReferencesForPage,
    pagePathLabel,
    pageStatsForText,
    personalVaultIdForPubkey,
    plaintextDevelopmentGrantFromExportGrant,
    plaintextGrantFromGiftWrappedExportGrant,
    planOkfImport,
    populateInviteFromHash,
    prepareOkfImportWrites,
    projectionPagesFromProjection,
    protectedRequestError,
    publicKeyIdentityFromInput,
    readerFolderDetail,
    readerFolderRows,
    readerSearchHighlightForPage,
    readerPageDetail,
    readerPageRows,
    resumeSession,
    searchHighlightSegments,
    searchPageRows,
    searchResultSnippet,
    settingsSectionsForSession,
    sharedFolderRelationshipRows,
    sessionGrantOpeningAllowed,
    sessionOperationIsCurrent,
    sessionStatusView,
    signedEventMatchesPinnedIdentity,
    signerIdentityChanged,
    hasOrganizationVaultControls,
    showsCreateOrganizationControl,
    sidebarAccessBadgesForFolder,
    sidebarModeLabel,
    shortKey,
    start,
    lockSession,
    rememberIdentity,
    identityMetadataForNpub,
    identityDisplay,
    lockedVaultSelection,
    missingVisibleVaultFallback,
    visibleVaultOptions,
    vaultHealthBadges,
    workspaceChromeState,
    workspaceTabTitle,
    vaultInvitationAcceptPath,
    vaultInvitationCreatePath,
    vaultInvitationIdentifierHint,
    vaultInvitationLinkPath,
    vaultInvitationPanelState,
    vaultInvitationRevokePath,
    vaultInvitationRevokeTarget,
    vaultInvitationRows,
    vaultInvitationUnavailableDetail,
    vaultPeopleRows,
    toggleMarkdownTask,
    taskCheckboxAriaLabel,
  };
})();

window.FiniteBrainProductClient = FiniteBrainProductClient;
if (!window.__FINITE_BRAIN_DISABLE_AUTOSTART__) {
  FiniteBrainProductClient.start();
}
