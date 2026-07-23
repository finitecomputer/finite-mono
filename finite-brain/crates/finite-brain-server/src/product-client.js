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
    activeBrainId: null,
    requestedBrainId: null,
    pendingOrganizationCreation: null,
    visibleBrains: [],
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
    brainSwitcherOpen: false,
    brainSwitcherPreviousFocus: null,
    manageBrainsModalOpen: false,
    manageBrainsModalPreviousFocus: null,
    manageBrainsReturnToSettings: null,
    activeAccessFolderId: null,
    activeAccessIntent: "overview",
    accessBusy: false,
    accessResult: null,
    identityByNpub: new Map(),
    lastShareLinkId: null,
    lastBrainInvitationCode: null,
    lastBrainInvitationId: null,
    lastEmailInviteSecret: null,
    lastEmailInviteUrl: null,
    lastEmailInvitePostProof: null,
    brainInvitationFolderIds: new Set(),
    brainInvitations: null,
    folderShareLinks: null,
    folderShareLinksFolderId: null,
    sharedFolderInvitations: null,
    sharedFolderConnections: null,
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
  const BRAIN_PERSONAL_AGENT_CONFIRMATION_REQUEST =
    "finite-brain-personal-agent-confirmation-request-v1";
  const BRAIN_PERSONAL_AGENT_CONFIRMATION_RESPONSE =
    "finite-brain-personal-agent-confirmation-response-v1";
  const BRAIN_EVENT_KIND_BY_INTENT = Object.freeze({
    "folder-object-revision": APP_EVENT_KIND,
    "folder-object-tombstone": APP_EVENT_KIND,
    "brain-access-change": APP_EVENT_KIND,
    "brain-invite-authorization": APP_EVENT_KIND,
  });
  const BRAIN_EVENT_D_PREFIX_BY_INTENT = Object.freeze({
    "folder-object-revision": "finite-folder-object-revision:",
    "folder-object-tombstone": "finite-folder-object-tombstone:",
    "brain-access-change": "finite-brain-admin-access-change:",
    "brain-invite-authorization": "finite-email-invite-bootstrap-authorization:",
  });
  const MAX_OBJECT_ID_ATTEMPTS = 1000;
  const MAX_BRAIN_INVITE_BOOTSTRAP_FOLDERS = 100;
  // Keep these public-client preflight bounds aligned with finite-brain-core.
  const MAX_PERSONAL_AGENT_ROTATION_FOLDERS = 100;
  const MAX_FOLDER_ROTATION_GRANTS = 1000;
  const MAX_FOLDER_ROTATION_RECORDS = 1000;
  const MAX_PERSONAL_AGENT_ROTATION_GRANTS = 10000;
  const MAX_PERSONAL_AGENT_ROTATION_RECORDS = 10000;
  const BRAIN_ACCESS_CHANGED_NOTICE =
    "Brain access changed. Your Brain was locked. Select a Brain you can open, then open it again.";
  const BRAIN_ACCESS_REQUIRED_REASON = "brain access required";
  const CLIENT_ACTION_FEEDBACK = Object.freeze({
    inviteLinkCopyFailure: "Could not copy private invite link. Try again.",
    inviteLinkCopySuccess: "Private invite link copied.",
    folderIdCopyFailure: "Could not copy Folder ID. Try again.",
    folderIdCopySuccess: "Folder ID copied.",
    pageIdCopyFailure: "Could not copy Page ID. Try again.",
    pageIdCopySuccess: "Page ID copied.",
    failure:
      "Action could not be completed. Try again. If it continues, check your connection and Brain status.",
  });
  const CLIENT_ACTION_FEEDBACK_DURATION_MS = 5000;
  const SESSION_PLAINTEXT_INPUT_IDS = [
    "accessAddPersonInput",
    "accessShareExpiresAtInput",
    "accessShareLinkInput",
    "accessShareMountInput",
    "accessShareTargetInput",
    "commandPaletteInput",
    "manageOrganizationBrainNameInput",
    "managePersonalAgentEmailInput",
    "pageBaseRevisionInput",
    "pageDraftInput",
    "pageFolderIdInput",
    "pageObjectIdInput",
    "personalAgentEmailInput",
    "sidebarSearchInput",
    "brainAdminEmailInput",
    "brainInviteCodeInput",
    "brainInviteEmailInput",
    "brainInviteExpiresAtInput",
    "brainInviteSecretInput",
    "brainInviteRecipientEmailInput",
    "brainInviteUrlInput",
    "brainMemberEmailInput",
  ];
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
    lastErrorValue = error instanceof Error ? error.message : String(error || "Action failed");
    setClientActionFeedback("error", clientFailureMessage(error), { expires: false });
  }

  function clientFailureMessage(error) {
    const message = error instanceof Error ? error.message : String(error || "");
    if (error?.code === "brain_target_unavailable") {
      return "You do not have access to that Brain yet. Refresh or check your invitation.";
    }
    if (error?.code === "brain_setup_cancelled" || /setup was cancelled/i.test(message)) {
      return "Brain setup was cancelled. Nothing was created.";
    }
    if (
      error?.code === "brain_identity_resolution_failed" ||
      /managed agent|agent identity|identity.*resolv|does not belong to the signed owner's account/i.test(message)
    ) {
      return "Brain could not verify that agent. Check the Managed Agent Email and try again.";
    }
    if (/session is locked|unlock(?:ed)? session|session changed|signer identity changed/i.test(message)) {
      return "Your Brain is locked. Open it and try again.";
    }
    if (error instanceof TypeError || error?.code === "network_error" || Number(error?.status) >= 500) {
      return "FiniteBrain could not connect to the server. Check your connection and try again.";
    }
    return CLIENT_ACTION_FEEDBACK.failure;
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
    const display = email || "Private member";
    const status = email
      ? "Email confirmed"
      : "Email unavailable";
    const details = [
      {
        label: "Email",
        value: email || "Not available",
      },
    ];
    if (identity?.verifiedAt) {
      details.push({ label: "Verified", value: identity.verifiedAt });
    }
    return {
      details,
      display,
      email,
      npub: identity?.npub || value,
      status,
      tooltip: email || "This member's email is not available in the current Brain.",
    };
  }

  function identityDisplay(npub) {
    return identityMetadataForNpub(npub).display;
  }

  function personalAgentEmail(metadata) {
    const agentNpub = metadata?.personalAgent?.agentNpub;
    return agentNpub ? identityMetadataForNpub(agentNpub).email : null;
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

  function personalBrainIdForPubkey(pubkeyHex) {
    return pubkeyHex ? `personal-${pubkeyHex.slice(0, 16)}` : null;
  }

  function signerIdentityChanged(previousPubkeyHex, nextPubkeyHex) {
    return Boolean(
      previousPubkeyHex &&
        nextPubkeyHex &&
        previousPubkeyHex !== nextPubkeyHex
    );
  }

  function brainTargetFromSearch(search) {
    const candidate = new URLSearchParams(String(search || "")).get("brainId")?.trim() || "";
    return /^[a-z0-9][a-z0-9_-]{0,127}$/u.test(candidate) ? candidate : null;
  }

  function signedEventMatchesPinnedIdentity(expectedPubkeyHex, signedEvent) {
    return Boolean(
      expectedPubkeyHex &&
        typeof signedEvent?.pubkey === "string" &&
        signedEvent.pubkey.toLowerCase() === expectedPubkeyHex.toLowerCase()
    );
  }

  function normalizeVisibleBrain(brain) {
    const brainId = brain?.brainId || brain?.brain_id || brain?.id || "";
    if (!brainId) return null;
    const kind = String(brain.kind || "organization").toLowerCase();
    return {
      brainId,
      kind: kind === "personal" ? "personal" : "organization",
      name: brain.name || (kind === "personal" ? "Personal Brain" : brainId),
      role: brain.role || (kind === "personal" ? "owner" : "member"),
      inviteCode: brain.inviteCode || brain.invite_code || null,
    };
  }

  function visibleBrainOptions(brains = state.visibleBrains) {
    const normalized = brains.map(normalizeVisibleBrain).filter(Boolean);
    const personal = normalized.find((brain) => brain.kind === "personal");
    const organizations = normalized
      .filter((brain) => brain.kind === "organization")
      .sort((left, right) => left.name.localeCompare(right.name) || left.brainId.localeCompare(right.brainId));
    return personal ? [personal, ...organizations] : organizations;
  }

  function activeBrainOption() {
    return visibleBrainOptions().find((brain) => brain.brainId === state.activeBrainId) || null;
  }

  function selectAccessibleBrain({ brains, currentBrainId, explicitTargetBrainId }) {
    const visible = visibleBrainOptions(brains || []);
    if (explicitTargetBrainId) {
      const target = visible.find((brain) => brain.brainId === explicitTargetBrainId);
      return target
        ? { brainId: target.brainId, reason: "explicit_target" }
        : { brainId: null, reason: "target_unavailable", targetBrainId: explicitTargetBrainId };
    }
    const current = visible.find((brain) => brain.brainId === currentBrainId);
    if (current) return { brainId: current.brainId, reason: "current_session" };
    const personal = visible.find((brain) => brain.kind === "personal");
    if (personal) return { brainId: personal.brainId, reason: "personal_default" };
    const organizations = visible.filter((brain) => brain.kind === "organization");
    if (organizations.length === 1) {
      return { brainId: organizations[0].brainId, reason: "sole_organization" };
    }
    return { brainId: null, reason: organizations.length ? "choose" : "empty" };
  }

  function activeBrainLabel() {
    const lockedSelection = lockedBrainSelection(
      state.sessionStatus,
      state.activeBrainId,
      state.visibleBrains
    );
    if (lockedSelection) return lockedSelection.label;
    return state.metadata?.name || activeBrainOption()?.name || state.activeBrainId || "No Brain selected";
  }

  function nestedManageBrainsReturnToken() {
    if (!state.manageBrainsModalOpen || !state.manageBrainsReturnToSettings) return null;
    // This only carries a Settings section and a DOM focus target. It is not
    // session content, so a nested Manage Brains reset can return safely.
    return state.manageBrainsReturnToSettings;
  }

  function resetBrainSessionState(options = {}) {
    const returnToSettings =
      options.preserveManageBrainsReturnToSettings === false ? null : nestedManageBrainsReturnToken();
    state.sessionEpoch += 1;
    pendingInviteNavigation = null;
    clearSessionSecretsAndPlaintext(state);
    if (returnToSettings) state.manageBrainsReturnToSettings = returnToSettings;
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
      "brainInvitationList",
      "brainPeopleList",
      "brainSwitcherList",
      "manageBrainsList",
    ]) {
      $(id)?.replaceChildren?.();
    }
    setText("readerPageTitle", "Brain locked");
    setText("readerPagePath", "Open the Brain to view your private content");
    const readerContent = $("readerPageContent");
    if (readerContent) {
      readerContent.replaceChildren?.();
      readerContent.textContent = "Brain locked. Open it to view your private content.";
    }
    if (typeof document.title === "string") document.title = "FiniteBrain";
    setPill("graphStats", "0 nodes / 0 links", "muted");
    setText("graphEmptyTitle", "No graph yet");
    setText("graphEmptyCopy", "Open the Brain to rebuild the local graph.");
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
    resetBrainSessionState({ preserveManageBrainsReturnToSettings: false });
    render();
    log("Locked Product Client session.", { status: state.sessionStatus });
  }

  function lockSessionForBrainAccessChange(error, requestEpoch) {
    if (
      !sessionOperationIsCurrent(state.sessionEpoch, requestEpoch, state.sessionStatus) ||
      !isActiveBrainAuthorizationLoss(error, state.activeBrainId)
    ) {
      return false;
    }
    markSessionLockFailureHandled(error);
    resetBrainSessionState({ preserveManageBrainsReturnToSettings: false });
    state.sessionNotice = BRAIN_ACCESS_CHANGED_NOTICE;
    render();
    log("Locked Product Client session after Brain access changed.", {
      status: state.sessionStatus,
    });
    return true;
  }

  async function refreshVisibleBrainsAfterAccessChange() {
    const lockedBrainId = state.activeBrainId;
    state.sessionStatus = SESSION_STATUS.RESUMING;
    try {
      const response = await protectedRequest("/_admin/brains");
      state.visibleBrains = (response.brains || []).map(normalizeVisibleBrain).filter(Boolean);
      if (!state.visibleBrains.some((brain) => brain.brainId === lockedBrainId)) {
        state.activeBrainId = null;
      }
    } finally {
      state.sessionStatus = SESSION_STATUS.LOCKED;
      state.sessionNotice = BRAIN_ACCESS_CHANGED_NOTICE;
      render();
    }
  }

  function handlePageHide() {
    lockSession();
  }

  function handlePageShow(event) {
    if (event?.persisted) lockSession();
  }

  async function resumeSession() {
    return loadBrainReader({ allowResume: true });
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
        target.brainInvitations?.length ||
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

  function setActiveBrainId(brainId, options = {}) {
    const nextBrainId = brainId || null;
    const changed = nextBrainId !== state.activeBrainId;
    state.activeBrainId = nextBrainId;
    if (changed && options.reset !== false) resetBrainSessionState();
  }

  function lockedBrainSelection(status, activeBrainId, visibleBrains) {
    if (status === SESSION_STATUS.UNLOCKED || visibleBrains.length) return null;
    return {
      label: "Selected Brain (locked)",
      value: activeBrainId,
    };
  }

  function brainIdFromName(prefix, name, options = {}) {
    const slug =
      String(name || prefix)
        .trim()
        .toLowerCase()
        .replace(/[^a-z0-9_-]+/g, "-")
        .replace(/^-+|-+$/g, "")
        .slice(0, 48) || prefix;
    const createdAt = Number.isFinite(options.createdAt) ? options.createdAt : Date.now();
    const entropy = options.entropy || crypto.getRandomValues(new Uint8Array(8));
    return `${prefix}-${slug}-${createdAt.toString(36)}-${bytesToHex(entropy)}`.slice(0, 128);
  }

  function rememberVisibleBrain(metadata) {
    if (!metadata?.brainId) return;
    const actorNpub = state.pubkeyHex ? npubFromHex(state.pubkeyHex) : null;
    const brain = normalizeVisibleBrain({
      brainId: metadata.brainId,
      kind: metadata.kind,
      name: metadata.name,
      role: metadataBrainRole(metadata, actorNpub),
    });
    if (!brain) return;
    state.visibleBrains = [
      brain,
      ...state.visibleBrains.filter((candidate) => normalizeVisibleBrain(candidate)?.brainId !== brain.brainId),
    ];
  }

  function metadataBrainRole(metadata, actorNpub) {
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
        detail: "A secure Brain connection is required before opening Brain.",
        canConnect: false,
      };
    }
    if (typeof provider.getPublicKey !== "function" || typeof provider.signEvent !== "function") {
      return {
        status: "unsupported",
        label: "unsupported",
        detail: "The secure Brain connection needs an update before Brain can open.",
        canConnect: false,
      };
    }
    return {
      status: "ready",
      label: "ready",
      detail: "A secure Brain connection is ready.",
      canConnect: true,
    };
  }

  function deriveBrainIdentityProviderState(provider) {
    if (!provider) {
      return {
        status: "setup_required",
        label: "setup required",
        detail: "Finish setting up your Finite account in Chat before opening Brain.",
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
        detail: "The secure Brain connection needs an update before Brain can open.",
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
            ? "Finish setting up your Finite account in Chat before opening Brain."
            : "Checking the secure Brain connection."),
        canConnect: false,
      };
    }
    return {
      status: "ready",
      label: "ready",
      detail: "A secure Brain connection is ready.",
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
  // or the add-person form. Brain selection lives in the footer and Manage Brains.
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
        detail: "Load a Brain and select a Folder to inspect access.",
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
        ? `${countLabel(adminCount, "admin")} + ${countLabel(explicitCount, "person", "people")}`
        : `${countLabel(adminCount, "admin")}`;
    }
    if (row.access === "restricted") {
      return explicitCount ? countLabel(explicitCount, "person", "people") : "Owner only";
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
    if (!row) return "Load a Brain to inspect Folder access.";
    const keyOpen = openedFolderKeys.has(folderKeyVersionKey(row.id, row.currentKeyVersion || 1));
    if (row.setupIncomplete) return "This Folder still needs setup before its access is ready.";
    if (row.access === "owner") return "Only the Personal Brain owner should be able to open this Folder.";
    if (row.access === "admin_only") return "Brain admins can open this Folder. Ordinary members cannot.";
    if (row.access === "all_members") return "Every member of this Brain can open this Folder after access is ready.";
    if (row.access === "restricted" && metadata?.kind === "organization") {
      return keyOpen
        ? "Admins and explicitly added people can open this restricted Folder."
        : "Open this restricted Folder before changing its People or Links.";
    }
    if (row.access === "restricted") {
      return keyOpen
        ? "This personal restricted Folder is open on this device and stays inside its tighter boundary."
        : "This personal restricted Folder is owner-scoped until you grant or share access.";
    }
    return "Access is Folder-scoped. Keep summaries and logs inside a Folder with the right audience.";
  }

  function accessPeopleHint(row, metadata) {
    if (!row) return "Choose a Folder first.";
    if (row.access === "all_members") {
      return "All Brain members have access; use Add if a newer member cannot open this Folder yet.";
    }
    if (row.access !== "restricted") return "Direct access is only needed for restricted Folders.";
    if (metadata?.kind === "organization") return "Admins can open it; add people by email when needed.";
    return "Personal restricted Folders start owner-only; grant one email when sharing is intentional.";
  }

  function folderAllowsDirectGrant(row) {
    return row?.access === "restricted" || row?.access === "all_members";
  }

  function accessFlowHint(row, mode, keyOpen) {
    if (!row) return "Choose a Folder to manage access.";
    if (mode === "people" && !folderAllowsDirectGrant(row)) {
      return "This Folder uses Brain-level access, so there is no direct People list to edit.";
    }
    if (mode === "links" && row.access !== "restricted") {
      return "Create links from restricted Folders so each link grants only the intended access.";
    }
    if (!keyOpen) return "Open this Folder before creating access or links.";
    if (mode === "people" && row.access === "all_members") {
      return "Grant makes this Folder available to an existing Brain member.";
    }
    if (mode === "people") return "Grant adds one email. Removing someone securely revokes their access.";
    if (mode === "links") return "Create a single-use link for a target email, or accept an existing link.";
    return "Choose People or Links when this Folder needs an access change.";
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
      label: `${mount.displayName} -> ${mount.sourceBrainId}/${mount.sourceFolderId}`,
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
    if (input?.purpose !== expectedPurpose || !String(input?.brainId || "").trim()) {
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
      brainId: payload.brainId,
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
      ["d", `finite-folder-key-grant:${input.brainId}:${input.folderId}:${input.keyVersion}`],
      ["brain", input.brainId],
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
      plaintext.brainId !== input.brainId ||
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
        brainId: input.brainId,
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
    if (input?.purpose === "brain-invite-bootstrap") {
      const { recipientHex } = scopedBrainGrantRecipient(input, "brain-invite-bootstrap");
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
        payload.brainId !== input.brainId ||
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
          ["d", `finite-email-invite-bootstrap:${input.brainId}`],
          ["brain", input.brainId],
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
        const configuredRetryIntervalMs = Number(options.sessionProofRetryIntervalMs);
        const retryIntervalMs =
          Number.isFinite(configuredRetryIntervalMs) && configuredRetryIntervalMs > 0
            ? configuredRetryIntervalMs
            : 250;
        const proofRequest = {
          type: BRAIN_SESSION_PROOF_REQUEST,
          requestId,
          requestHash,
        };
        const sendProofRequest = () => {
          window.parent.postMessage(proofRequest, trustedParentOrigin);
        };
        let retry = null;
        const timeout = setTimeout(() => {
          if (retry) clearInterval(retry);
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
          if (retry) clearInterval(retry);
          window.removeEventListener("message", handleProof);
          if (typeof event.data.proof === "string" && event.data.proof) {
            resolve(event.data.proof);
          } else {
            reject(new Error("Your dashboard session expired. Sign in and open Brain again."));
          }
        }
        window.addEventListener("message", handleProof);
        sendProofRequest();
        retry = setInterval(sendProofRequest, retryIntervalMs);
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
        if (input?.purpose === "brain-invite-bootstrap") return result.wrappedEventJson;
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
      if (intent === "brain-access-change") {
        if (
          payload.version !== "finite-brain-admin-access-change-v1" ||
          (signerNpub && payload.adminNpub !== signerNpub) ||
          canonicalAdminAccessChangePayload(payload) !== eventTemplate.content
        ) {
          throw new Error("Brain access-change payload is invalid");
        }
        requireExactBrainEventTags(eventTemplate, adminAccessChangeTags(payload), intent);
        return;
      }
      if (intent === "brain-invite-authorization") {
        const canonical = JSON.stringify({
          version: payload.version,
          brainId: payload.brainId,
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
          brainId: payload.brainId,
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
    target.visibleBrains = [];
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
    target.lastBrainInvitationCode = null;
    target.lastBrainInvitationId = null;
    target.lastEmailInviteSecret = null;
    target.lastEmailInviteUrl = null;
    target.lastEmailInvitePostProof = null;
    target.brainInvitationFolderIds?.clear?.();
    target.brainInvitations = null;
    target.folderShareLinks = null;
    target.folderShareLinksFolderId = null;
    target.sharedFolderInvitations = null;
    target.sharedFolderConnections = null;
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
    target.manageBrainsReturnToSettings = null;
    return target;
  }

  function sessionStatusView(status) {
    if (status === SESSION_STATUS.UNLOCKED) {
      return {
        action: "Lock Brain",
        detail: "Your Brain is open and ready on this device.",
        locked: false,
        title: "Brain ready",
      };
    }
    if (status === SESSION_STATUS.RESUMING) {
      return {
        action: "Lock Brain",
        detail: "Opening your private content on this device.",
        locked: false,
        title: "Opening Brain",
      };
    }
    return {
      action: "Open Brain",
      detail: "Your private content is closed on this device.",
      locked: true,
      title: "Brain locked",
    };
  }

  function sessionIdentityLabel() {
    if (state.signerStatus === "connected" && state.pubkeyHex) {
      const email = identityEmailDisplay(identityForNpub(npubFromHex(state.pubkeyHex)));
      return email || "Connected securely";
    }
    if (state.signerStatus === "ready") return "Ready to connect";
    if (state.signerStatus === "checking") return "Checking connection";
    if (state.signerStatus === "setup_required") return "Connection setup required";
    return "Connection unavailable";
  }

  const SETTINGS_SECTIONS = Object.freeze(["session", "brain", "access", "invitations"]);

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
        detail: sessionIdentityLabel() === "Connected securely"
          ? "A secure Brain connection is active on this device."
          : `Connected as ${sessionIdentityLabel()}.`,
        title: "Connection ready",
      };
    }
    if (state.signerStatus === "checking") {
      return {
        canConnect: false,
        detail: "Checking the secure Brain connection.",
        title: "Checking connection",
      };
    }
    return {
      canConnect: provider.canConnect,
      detail: provider.detail,
      title: provider.canConnect ? "Ready to connect" : provider.status === "setup_required" ? "Connection setup required" : "Connection unavailable",
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
      $("brainInvitationActionSection"),
      $("brainInvitationPanel"),
      $("brainInvitationListSection"),
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
          section === "brain"
            ? "settingsNavBrain"
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
    if (state.brainSwitcherOpen) closeBrainSwitcher({ restoreFocus: false });
    if (state.manageBrainsModalOpen) closeManageBrainsModal();
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

  function brainSwitcherFocusableElements() {
    return overlayFocusableElements("brainSwitcherMenu");
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

  function moveBrainSwitcherFocusOut(options = {}) {
    const menu = $("brainSwitcherMenu");
    const trigger = $("sessionAccountBrainButton");
    const focusable = documentFocusableElements(menu);
    const triggerIndex = focusable.indexOf(trigger);
    const direction = options.backwards ? -1 : 1;
    const nextTarget = triggerIndex >= 0 ? focusable[triggerIndex + direction] : null;
    closeBrainSwitcher({ restoreFocus: false });
    nextTarget?.focus?.();
  }

  function focusBrainSwitcherItem(index = 0) {
    const items = brainSwitcherFocusableElements();
    if (!items.length) return;
    const nextIndex = Math.min(Math.max(index, 0), items.length - 1);
    if (typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => items[nextIndex]?.focus?.());
    } else {
      items[nextIndex]?.focus?.();
    }
  }

  function openBrainSwitcher() {
    if (state.brainSwitcherOpen) {
      closeBrainSwitcher();
      return;
    }
    if (state.settingsModalOpen) closeSettingsModal();
    if (state.manageBrainsModalOpen) closeManageBrainsModal();
    state.brainSwitcherPreviousFocus = document.activeElement || null;
    state.brainSwitcherOpen = true;
    closeContextMenu();
    closeCommandPalette();
    closeEditorSlashMenu();
    render();
    focusBrainSwitcherItem(0);
  }

  function closeBrainSwitcher(options = {}) {
    if (!state.brainSwitcherOpen) return;
    state.brainSwitcherOpen = false;
    const previousFocus = state.brainSwitcherPreviousFocus;
    state.brainSwitcherPreviousFocus = null;
    render();
    if (options.restoreFocus !== false) previousFocus?.focus?.();
  }

  function manageBrainsModalFocusableElements() {
    return overlayFocusableElements("manageBrainsModal");
  }

  function focusManageBrainsReturnTarget() {
    if (state.settingsSection !== "brain") {
      focusSettingsSection(state.settingsSection);
      return;
    }
    const target = $("settingsManageBrainsButton");
    if (typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => target?.focus?.());
    } else {
      target?.focus?.();
    }
  }

  function openManageBrainsModal(options = {}) {
    if (state.manageBrainsModalOpen) return;
    const menuFocus = state.brainSwitcherPreviousFocus;
    const returnToSettings = Boolean(options.returnToSettings && state.settingsModalOpen);
    state.manageBrainsReturnToSettings = returnToSettings
      ? {
          previousFocus: state.settingsModalPreviousFocus,
          section: "brain",
        }
      : null;
    state.manageBrainsModalPreviousFocus = returnToSettings
      ? null
      : menuFocus || document.activeElement || null;
    closeBrainSwitcher({ restoreFocus: false });
    if (state.settingsModalOpen) {
      closeSettingsModal({ restoreFocus: false });
    }
    state.manageBrainsModalOpen = true;
    closeContextMenu();
    closeCommandPalette();
    closeEditorSlashMenu();
    render();
    const focusTarget = $("closeManageBrainsButton");
    if (typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => focusTarget?.focus?.());
    } else {
      focusTarget?.focus?.();
    }
  }

  function closeManageBrainsModal() {
    if (!state.manageBrainsModalOpen) return;
    state.manageBrainsModalOpen = false;
    const returnToSettings = state.manageBrainsReturnToSettings;
    state.manageBrainsReturnToSettings = null;
    const previousFocus = state.manageBrainsModalPreviousFocus;
    state.manageBrainsModalPreviousFocus = null;
    if (returnToSettings) {
      state.settingsSection = returnToSettings.section;
      state.settingsModalPreviousFocus = returnToSettings.previousFocus;
      state.settingsModalOpen = true;
      render();
      focusManageBrainsReturnTarget();
      return;
    }
    render();
    previousFocus?.focus?.();
  }

  function manageBrainsLoadAction() {
    const operation = state.sessionStatus === SESSION_STATUS.LOCKED
      ? resumeSession()
      : loadBrainReader();
    operation.catch((error) => {
      reportClientActionFailure(error);
      log("Failed to load Brain from Manage Brains.", { error: error.message });
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
    const brainNav = $("settingsNavBrain");
    const accessNav = $("settingsNavAccess");
    const invitationsNav = $("settingsNavInvitations");
    const sessionPanel = $("settingsSessionPanel");
    const brainPanel = $("settingsBrainPanel");
    const accessPanel = $("settingsAccessPanel");
    const invitationsPanel = $("settingsInvitationsPanel");
    const sessionActive = state.settingsSection === "session";
    const brainActive = state.settingsSection === "brain";
    const accessActive = state.settingsSection === "access";
    const invitationsActive = state.settingsSection === "invitations";
    if (sessionNav) {
      sessionNav.hidden = false;
      sessionNav.className = `settings-nav-item${sessionActive ? " active" : ""}`;
      sessionNav.setAttribute("aria-selected", String(sessionActive));
      sessionNav.tabIndex = sessionActive ? 0 : -1;
    }
    if (brainNav) {
      brainNav.hidden = sessionOnly;
      brainNav.className = `settings-nav-item${brainActive ? " active" : ""}`;
      brainNav.setAttribute("aria-selected", String(brainActive));
      brainNav.tabIndex = brainActive ? 0 : -1;
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
    if (brainPanel) {
      brainPanel.hidden = !brainActive;
      brainPanel.setAttribute("aria-hidden", String(!brainActive));
    }
    if (accessPanel) {
      accessPanel.hidden = !accessActive;
      accessPanel.setAttribute("aria-hidden", String(!accessActive));
    }
    if (invitationsPanel) {
      invitationsPanel.hidden = !invitationsActive;
      invitationsPanel.setAttribute("aria-hidden", String(!invitationsActive));
    }
    setText("settingsBrainName", activeBrainLabel());
    setText("settingsBrainIdentity", sessionIdentityLabel());
    setText("settingsBrainStatus", sessionStatusView(state.sessionStatus).title);
    const signer = settingsSignerView();
    setText("settingsSignerTitle", signer.title);
    setText("settingsSignerDetail", signer.detail);
    if (state.settingsModalOpen && forcedSessionSection) focusSettingsSection("session");
  }

  function renderBrainSwitcher() {
    const menu = $("brainSwitcherMenu");
    const trigger = $("sessionAccountBrainButton");
    if (!menu || !trigger) return;
    menu.hidden = !state.brainSwitcherOpen;
    trigger.setAttribute("aria-expanded", String(state.brainSwitcherOpen));
    setText("brainSwitcherCount", `${visibleBrainOptions().length}`);
    const rows = visibleBrainOptions();
    const emptyText = state.signerStatus === "connected"
      ? "No Brains available."
      : "Connect securely to list Brains.";
    setList("brainSwitcherList", rows, emptyText, (item, brain) => {
      const button = brainSwitchButton(brain, "switcher");
      button.setAttribute("role", "menuitem");
      item.appendChild(button);
    });
  }

  function renderManageBrainsModal() {
    const modal = $("manageBrainsModal");
    if (!modal) return;
    modal.hidden = !state.manageBrainsModalOpen;
    modal.setAttribute("aria-hidden", String(!state.manageBrainsModalOpen));
    const shell = document.querySelector?.(".obsidian-shell");
    if (shell) shell.dataset.manageBrainsOpen = state.manageBrainsModalOpen ? "true" : "false";
    setText("manageBrainsCurrentName", activeBrainLabel());
    const status = sessionStatusView(state.sessionStatus);
    setText(
      "manageBrainsCurrentDetail",
      state.metadata
        ? `${status.title}. ${brainManagementSummary(state.metadata)}`
        : `${status.title}. Select a Brain, then ${status.locked ? "open it" : "load it"} to view private content.`
    );
    const signerConnected = state.signerStatus === "connected";
    safeSetHidden("manageBrainsConnectSignerButton", signerConnected);
    setOptionalDisabled(
      "manageBrainsConnectSignerButton",
      !deriveBrainIdentityProviderState(state.identityProvider).canConnect
    );
    const action = state.sessionStatus === SESSION_STATUS.LOCKED
      ? "Open Brain"
      : state.sessionStatus === SESSION_STATUS.RESUMING
        ? "Opening…"
        : "Load Brain";
    setText("manageBrainsLoadButton", action);
    setOptionalDisabled(
      "manageBrainsLoadButton",
      state.sessionStatus === SESSION_STATUS.RESUMING || !canLoadBrain()
    );
    const personalBrain = visibleBrainOptions().find((brain) => brain.kind === "personal");
    safeSetHidden("managePersonalBrainCreate", Boolean(personalBrain));
    const suggestedAgent = suggestedAgentIdentityFromNavigation();
    const personalAgentInput = $("managePersonalAgentEmailInput");
    if (personalAgentInput && !personalAgentInput.value && suggestedAgent?.email) {
      personalAgentInput.value = suggestedAgent.email;
    }
    const addAgentInput = $("manageOrganizationAddAgentInput");
    const agentLabel = suggestedAgent?.email || suggestedAgent?.name || "selected agent";
    setText("manageOrganizationAgentLabel", agentLabel);
    if (addAgentInput) {
      addAgentInput.disabled = !suggestedAgent;
      if (!suggestedAgent) addAgentInput.checked = false;
    }
    safeSetHidden("manageBrainCreateDetails", false);
    const canCreate = Boolean(
      state.config &&
      !state.readerBusy &&
      (state.signerStatus === "connected" || deriveBrainIdentityProviderState(state.identityProvider).canConnect)
    );
    setOptionalDisabled("manageCreatePersonalBrainButton", !canCreate || Boolean(personalBrain));
    setOptionalDisabled(
      "manageCreateOrganizationBrainButton",
      !canCreate
    );
    const rows = visibleBrainOptions();
    const emptyText = signerConnected ? "No Brains available." : "Connect securely to list Brains.";
    setList("manageBrainsList", rows, emptyText, (item, brain) => {
      item.appendChild(brainSwitchButton(brain, "manage"));
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

  function folderKeyId(brainId, folderId, keyVersion) {
    return `${brainId}:${folderId}:${keyVersion}`;
  }

  async function importFolderKey(keyring, { brainId, folderId, keyVersion, folderKey }, options = {}) {
    options.assertCurrent?.();
    const rawKey = base64ToBytes(folderKey);
    if (rawKey.length !== 32) throw new Error("Folder Key must be 32 bytes");
    const cryptoKey = await crypto.subtle.importKey("raw", rawKey, "AES-GCM", false, [
      "encrypt",
      "decrypt",
    ]);
    options.assertCurrent?.();
    const id = folderKeyId(brainId, folderId, keyVersion);
    keyring.keys.set(id, {
      cryptoKey,
      folderId,
      keyVersion,
      rawKey,
      brainId,
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
        grant.brainId === grantPlaintext.brainId
    );
    if (!alreadyOpened) {
      keyring.openedGrants.push({
        folderId: grantPlaintext.folderId,
        issuerNpub: grantPlaintext.issuerNpub,
        keyVersion: grantPlaintext.keyVersion,
        recipientNpub: grantPlaintext.recipientNpub,
        brainId: grantPlaintext.brainId,
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
        brainId: options.expectedBrainId || grant?.brainId,
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

  async function openFolderKeyGrants(keyring, exportedBrain, expectedRecipientNpub = null, options = {}) {
    const opened = [];
    const skipped = [];
    for (const grant of exportedBrain?.keyGrants || []) {
      try {
        options.assertCurrent?.();
        const plaintext = await plaintextGrantFromGiftWrappedExportGrant(grant, expectedRecipientNpub, {
          ...options,
          expectedBrainId:
            options.expectedBrainId || exportedBrain?.brain?.id || exportedBrain?.brainId,
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

  async function openDevelopmentFolderKeyGrants(keyring, exportedBrain, expectedRecipientNpub = null) {
    const opened = [];
    const skipped = [];
    for (const grant of exportedBrain?.keyGrants || []) {
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

  function canonicalFolderObjectAad({ brainId, folderId, objectId, keyVersion }) {
    return `{"version":${JSON.stringify(FOLDER_OBJECT_VERSION)},"brainId":${JSON.stringify(
      brainId
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
    const key = keyring.keys.get(folderKeyId(input.brainId, input.folderId, input.keyVersion));
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
    const key = keyring.keys.get(folderKeyId(input.brainId, input.folderId, envelope.keyVersion));
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
        brainId: input.brainId,
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
    return `{"version":${JSON.stringify(REVISION_VERSION)},"brainId":${JSON.stringify(
      input.brainId
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
        `finite-folder-object-revision:${input.brainId}:${input.folderId}:${input.objectId}:${input.revision}`,
      ],
      ["brain", input.brainId],
      ["folder", input.folderId],
      ["object", input.objectId],
      ["operation", input.operation],
      ["keyVersion", String(input.keyVersion)],
    ];
  }

  function canonicalTombstonePayload(input) {
    return `{"version":${JSON.stringify(TOMBSTONE_VERSION)},"brainId":${JSON.stringify(
      input.brainId
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
        `finite-folder-object-tombstone:${input.brainId}:${input.folderId}:${input.objectId}:${input.revision}`,
      ],
      ["brain", input.brainId],
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
      brainId: input.brainId,
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
      brainId: input.brainId,
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
        brainId: input.brainId,
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
      brainId: input.brainId,
    });
    const eventTemplate = {
      kind: APP_EVENT_KIND,
      created_at: createdAtUnix,
      tags: tombstoneTags({
        folderId: input.folderId,
        objectId: input.objectId,
        revision,
        brainId: input.brainId,
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
      .replace(/^\.\//, "")
      .replace(/\.md$/, "")
      .replace(/^#/, "")
      .normalize("NFC");
  }

  function isExternalPageReference(value) {
    return /^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(String(value || "").trim());
  }

  function markdownDestination(value) {
    const source = String(value || "").trim();
    let target = source;
    if (source.startsWith("<")) {
      const close = source.indexOf(">");
      target = close >= 0 ? source.slice(1, close) : source;
    } else {
      let depth = 0;
      let escaped = false;
      for (let index = 0; index < source.length; index += 1) {
        const char = source[index];
        if (escaped) {
          escaped = false;
          continue;
        }
        if (char === "\\") {
          escaped = true;
          continue;
        }
        if (char === "(") depth += 1;
        else if (char === ")" && depth > 0) depth -= 1;
        else if (/\s/.test(char) && depth === 0) {
          target = source.slice(0, index);
          break;
        }
      }
    }
    return target.trim().replace(/\\(.)/g, "$1");
  }

  function isMarkdownPageDestination(value) {
    const target = String(value || "").split("#")[0];
    const filename = target.split("/").pop() || "";
    const extension = filename.includes(".") ? filename.split(".").pop() : "";
    return !extension || extension === "md";
  }

  function rangeContains(ranges, index) {
    return ranges.some(([start, end]) => index >= start && index < end);
  }

  function markdownIndentColumns(value, initialColumns = 0) {
    let columns = initialColumns;
    for (const char of String(value || "")) {
      if (char === " ") columns += 1;
      else if (char === "\t") columns += 4 - (columns % 4);
      else break;
    }
    return columns;
  }

  function markdownCodeRanges(source) {
    const fenced = [];
    const indented = [];
    const lines = [];
    let fence = null;
    let offset = 0;
    for (const line of source.match(/.*(?:\n|$)/g) || []) {
      if (!line && offset >= source.length) break;
      const content = line.replace(/\r?\n$/, "");
      lines.push({ content, start: offset, end: offset + line.length });
      const markerMatch = content.match(/^ {0,3}((?:\x60){3,}|~{3,})(.*)$/);
      const marker = markerMatch?.[1] || "";
      const trailing = markerMatch?.[2] || "";
      if (!fence && marker) {
        fence = { char: marker[0], length: marker.length, start: offset };
      } else if (
        fence &&
        marker[0] === fence.char &&
        marker.length >= fence.length &&
        !trailing.trim()
      ) {
        fenced.push([fence.start, offset + line.length]);
        fence = null;
      }
      offset += line.length;
    }
    if (fence) fenced.push([fence.start, source.length]);

    let activeListIndent = 0;
    let indentedBlock = false;
    let validCodeBoundary = true;
    for (const line of lines) {
      if (rangeContains(fenced, line.start)) {
        indentedBlock = false;
        validCodeBoundary = true;
        continue;
      }
      if (!line.content.trim()) {
        validCodeBoundary = true;
        continue;
      }

      const leading = line.content.match(/^[ \t]*/)?.[0] || "";
      const indent = markdownIndentColumns(leading);
      const listMarker = line.content.match(
        /^([ \t]*)([-+*]|\d{1,9}[.)])([ \t]+)/
      );
      if (activeListIndent && indent < activeListIndent && !listMarker) {
        activeListIndent = 0;
      }
      const relativeIndent = Math.max(0, indent - activeListIndent);
      if (relativeIndent >= 4 && (indentedBlock || validCodeBoundary)) {
        indented.push([line.start, line.end]);
        indentedBlock = true;
        validCodeBoundary = false;
        continue;
      }

      indentedBlock = false;
      validCodeBoundary = false;
      if (listMarker) {
        const markerIndent = markdownIndentColumns(listMarker[1]);
        activeListIndent = markdownIndentColumns(
          listMarker[3],
          markerIndent + listMarker[2].length
        );
      }
    }

    const ranges = [...fenced, ...indented];
    for (let index = 0; index < source.length; index += 1) {
      if (rangeContains(fenced, index) || source[index] !== "\x60") continue;
      let length = 1;
      while (source[index + length] === "\x60") length += 1;
      const marker = "\x60".repeat(length);
      const close = source.indexOf(marker, index + length);
      if (close < 0 || rangeContains(fenced, close)) {
        index += length - 1;
        continue;
      }
      ranges.push([index, close + length]);
      index = close + length - 1;
    }
    return ranges.sort((left, right) => left[0] - right[0]);
  }

  function markdownReferenceLabel(value) {
    return String(value || "").trim().replace(/\s+/g, " ").normalize("NFC").toLowerCase();
  }

  function markdownContext(text) {
    const source = String(text || "");
    const excluded = markdownCodeRanges(source);
    const references = new Map();
    const definitionPattern = /^ {0,3}\[([^\]\n]+)\]:[ \t]*(.+)$/gm;
    for (const match of source.matchAll(definitionPattern)) {
      if (rangeContains(excluded, match.index)) continue;
      const destination = markdownDestination(match[2]);
      if (!destination) continue;
      const label = markdownReferenceLabel(match[1]);
      if (!references.has(label)) references.set(label, destination);
      excluded.push([match.index, match.index + match[0].length]);
    }
    excluded.sort((left, right) => left[0] - right[0]);
    return { excluded, references };
  }

  function closingMarkdownBracket(source, start) {
    let depth = 1;
    let escaped = false;
    for (let index = start + 1; index < source.length; index += 1) {
      const char = source[index];
      if (escaped) {
        escaped = false;
        continue;
      }
      if (char === "\\") {
        escaped = true;
        continue;
      }
      if (char === "[") depth += 1;
      if (char === "]") {
        depth -= 1;
        if (depth === 0) return index;
      }
    }
    return -1;
  }

  function wikiLinkTokens(text, context = markdownContext(text)) {
    const source = String(text || "");
    const pattern = /\[\[([^\]|#]+)(?:#[^\]|]*)?(?:\|([^\]]+))?\]\]/g;
    return [...source.matchAll(pattern)]
      .filter((match) => !rangeContains(context.excluded, match.index))
      .map((match) => ({
        end: match.index + match[0].length,
        label: String(match[2] || match[1]).trim(),
        start: match.index,
        target: normalizePageReference(match[1]),
      }));
  }

  function markdownLinkTokens(text, context = markdownContext(text)) {
    const source = String(text || "");
    const tokens = [];
    for (let start = 0; start < source.length; start += 1) {
      if (
        source[start] !== "[" ||
        source[start - 1] === "!" ||
        source[start - 1] === "[" ||
        rangeContains(context.excluded, start)
      ) {
        continue;
      }
      const labelEnd = closingMarkdownBracket(source, start);
      if (labelEnd < 0 || source[labelEnd + 1] !== "(") continue;
      let depth = 1;
      let quote = "";
      let escaped = false;
      let end = labelEnd + 2;
      for (; end < source.length; end += 1) {
        const char = source[end];
        if (escaped) {
          escaped = false;
          continue;
        }
        if (char === "\\") {
          escaped = true;
          continue;
        }
        if (quote) {
          if (char === quote) quote = "";
          continue;
        }
        if (char === '"' || char === "'") {
          quote = char;
          continue;
        }
        if (char === "(") depth += 1;
        if (char === ")") {
          depth -= 1;
          if (depth === 0) break;
        }
      }
      if (depth !== 0) continue;
      const destination = markdownDestination(source.slice(labelEnd + 2, end));
      if (!destination) continue;
      tokens.push({
        destination,
        end: end + 1,
        label: source.slice(start + 1, labelEnd),
        start,
      });
      start = end;
    }

    for (let start = 0; start < source.length; start += 1) {
      if (
        source[start] !== "[" ||
        source[start - 1] === "!" ||
        source[start - 1] === "[" ||
        source[start + 1] === "[" ||
        rangeContains(context.excluded, start) ||
        tokens.some((token) => start >= token.start && start < token.end)
      ) {
        continue;
      }
      const labelEnd = closingMarkdownBracket(source, start);
      if (labelEnd < 0 || source[labelEnd + 1] === "(") continue;
      const label = source.slice(start + 1, labelEnd);
      let reference = label;
      let end = labelEnd + 1;
      if (source[labelEnd + 1] === "[") {
        const referenceEnd = closingMarkdownBracket(source, labelEnd + 1);
        if (referenceEnd < 0) continue;
        reference = source.slice(labelEnd + 2, referenceEnd) || label;
        end = referenceEnd + 1;
      }
      const destination = context.references.get(markdownReferenceLabel(reference));
      if (!destination) continue;
      tokens.push({ destination, end, label, start });
      start = end - 1;
    }
    return tokens.sort((left, right) => left.start - right.start);
  }

  function extractPageLinks(text) {
    const links = new Set();
    const context = markdownContext(text);
    for (const token of wikiLinkTokens(text, context)) {
      links.add(token.target);
    }
    for (const token of markdownLinkTokens(text, context)) {
      const target = token.destination;
      if (!isExternalPageReference(target) && isMarkdownPageDestination(target)) {
        links.add(normalizePageReference(target.split("#")[0]));
      }
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

  function resolvePageReference(reference, pages = readablePages(), sourcePage = null) {
    const normalizedReference = normalizePageReference(reference);
    const matches = new Map();
    for (const page of pages.filter(isReadablePage)) {
      if (!pageReferencesForPage(page).includes(normalizedReference)) continue;
      matches.set(pageKeyForPage(page), page);
    }
    const readableMatches = [...matches.values()];
    const localMatches = sourcePage
      ? readableMatches.filter((page) => page.folderId === sourcePage.folderId)
      : [];
    const candidates = localMatches.length ? localMatches : readableMatches;
    if (candidates.length === 1) {
      return { matches: candidates, status: "resolved", target: candidates[0] };
    }
    return {
      matches: candidates,
      status: candidates.length ? "ambiguous" : "missing",
      target: null,
    };
  }

  function pageForReference(reference, pages = readablePages(), sourcePage = null) {
    return resolvePageReference(reference, pages, sourcePage).target;
  }

  function pageKeyForReference(reference, pages = readablePages(), sourcePage = null) {
    const page = pageForReference(reference, pages, sourcePage);
    return page ? pageKeyForPage(page) : null;
  }

  function inlineLinkSegments(text) {
    const source = String(text || "");
    const segments = [];
    const context = markdownContext(source);
    const tokens = [
      ...wikiLinkTokens(source, context).map((token) => ({
        end: token.end,
        kind: "internal",
        start: token.start,
        target: token.target,
        text: token.label,
      })),
      ...markdownLinkTokens(source, context).map((token) => {
        const external =
          isExternalPageReference(token.destination) ||
          !isMarkdownPageDestination(token.destination);
        return {
          end: token.end,
          kind: external ? "external" : "internal",
          start: token.start,
          target: external
            ? token.destination
            : normalizePageReference(token.destination.split("#")[0]),
          text: token.label.trim(),
        };
      }),
    ].sort((left, right) => left.start - right.start);
    let cursor = 0;
    for (const token of tokens) {
      if (token.start < cursor) continue;
      if (token.start > cursor) {
        segments.push({ kind: "text", text: source.slice(cursor, token.start) });
      }
      segments.push({ kind: token.kind, target: token.target, text: token.text });
      cursor = token.end;
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

  function requireOkfDestinationFolderId(...candidates) {
    const folderId = candidates
      .map((candidate) => String(candidate || "").trim())
      .find(Boolean);
    if (!folderId) {
      throw new Error("Create or select a Folder before importing OKF content");
    }
    return folderId;
  }

  function parseOkfBundle(input, options = {}) {
    const source = typeof input === "string" ? JSON.parse(input) : input;
    if (!source || typeof source !== "object") throw new Error("OKF bundle must be a JSON object");

    const sourceFiles = source.files || source;
    const files = new Map();
    for (const [path, content] of Object.entries(sourceFiles || {})) {
      if (typeof content === "string" && (path.endsWith(".md") || path === "okf-brain.json")) {
        files.set(normalizeSafeRelativePath(path, "OKF file path"), content);
      }
    }

    const manifest =
      source.manifest ||
      (files.has("okf-brain.json") ? JSON.parse(files.get("okf-brain.json")) : null);
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
          folderId: requireOkfDestinationFolderId(
            options.destinationFolderId,
            page.targetFolderId,
            page.folderId
          ),
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
          folderId: requireOkfDestinationFolderId(
            options.destinationFolderId,
            asset.targetFolderId,
            asset.folderId
          ),
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
            folderId: requireOkfDestinationFolderId(
              options.destinationFolderId,
              object.targetFolderId,
              object.folderId
            ),
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
            folderId: requireOkfDestinationFolderId(
              options.destinationFolderId,
              object.targetFolderId,
              object.folderId
            ),
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
        if (sourcePath === "okf-brain.json" || sourcePath.startsWith("_wiki/")) continue;
        pages.push({
          sourceFolderId: null,
          sourceObjectId: null,
          sourcePath,
          folderId: requireOkfDestinationFolderId(options.destinationFolderId),
          targetPath: targetPathFromBundlePath(sourcePath),
          markdown,
          contentType: "text/markdown",
          links: extractPageLinks(markdown),
        });
      }
    }

    return {
      version: manifest?.version || source.version || "finite-okf-brain-import-v1",
      assets,
      pages,
      omissions: manifest?.omissions || source.omissions || [],
    };
  }

  function normalizeExistingPageRecord(record) {
    const folderId = requireOkfDestinationFolderId(record.folderId);
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
      const folderId = requireOkfDestinationFolderId(
        options.destinationFolderId,
        page.folderId
      );
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
      const folderId = requireOkfDestinationFolderId(
        options.destinationFolderId,
        asset.folderId
      );
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
    if (!options?.brainId) throw new Error("OKF import requires a destination Brain");
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
      const keyId = folderKeyId(options.brainId, entry.folderId, keyVersion);
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
        brainId: options.brainId,
      });
      writes.push({
        action: entry.action,
        body,
        folderId: entry.folderId,
        objectId: entry.objectId,
        path: `/_admin/brains/${encodeURIComponent(options.brainId)}/folders/${encodeURIComponent(
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
      const id = pageKeyForPage(page);
      const title = pageTitleForPage(page);
      return {
        id,
        folderId: page.folderId,
        objectId: page.objectId,
        title,
      };
    });
    const nodesByPageKey = new Map(nodes.map((node) => [node.id, node]));
    const edges = [];
    const edgeIds = new Set();
    for (const page of visiblePages) {
      const source = nodesByPageKey.get(pageKeyForPage(page));
      if (!source) continue;
      for (const targetRef of extractPageLinks(page.text)) {
        const targetPage = resolvePageReference(targetRef, visiblePages, page).target;
        const target = targetPage ? nodesByPageKey.get(pageKeyForPage(targetPage)) : null;
        if (!target) continue;
        const id = `${source.id}->${target.id}`;
        if (edgeIds.has(id)) continue;
        edgeIds.add(id);
        edges.push({
          id,
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

  function readerEmptyStateCopy(metadata, sessionStatus, selectedFolderId, actorNpub = null) {
    const sessionLocked = sessionStatus !== SESSION_STATUS.UNLOCKED;
    const brainLoaded = Boolean(metadata?.brainId || metadata?.id || metadata?.kind);
    const brainIsEmpty = brainLoaded && (metadata?.folders || []).length === 0;
    const canCreateFolder = actorHasDestructiveAuthority(metadata, actorNpub);
    if (sessionLocked) {
      return {
        list: brainIsEmpty
          ? canCreateFolder
            ? "This Brain is empty. Open it to create a Folder."
            : "This Brain is empty. Ask a Brain admin to create the first Folder."
          : "Load a Brain to browse Folders.",
        path: "Open the Brain to view your private content",
        title: "Brain locked",
      };
    }
    if (selectedFolderId) {
      return {
        list: "No Folders available.",
        path: selectedFolderId,
        title: "No Page selected",
      };
    }
    if (brainIsEmpty) {
      return {
        list: canCreateFolder
          ? "This Brain is empty. Create a Folder to get started."
          : "This Brain is empty. Ask a Brain admin to create the first Folder.",
        path: canCreateFolder
          ? "Create a Folder to add Pages."
          : "A Brain admin must create the first Folder.",
        title: "This Brain is empty",
      };
    }
    return {
      list: "Load a Brain to browse Folders.",
      path: "No Page path loaded",
      title: "No Folder selected",
    };
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
    const currentKey = pageKeyForPage(page);
    const outgoing = extractPageLinks(page.text).map((targetRef) => {
      const resolution = resolvePageReference(targetRef, readable, page);
      if (!resolution.target) {
        return {
          detail:
            resolution.status === "ambiguous"
              ? `${resolution.matches.length} readable matches`
              : "unresolved",
          key: null,
          label: targetRef,
          status: resolution.status,
        };
      }
      return {
        detail: resolution.target.folderId,
        key: pageKeyForPage(resolution.target),
        label: pageTitleForPage(resolution.target),
        status: "resolved",
      };
    });
    const backlinks = readable
      .filter((candidate) => pageKeyForPage(candidate) !== currentKey)
      .filter((candidate) =>
        extractPageLinks(candidate.text).some((targetRef) => {
          const target = resolvePageReference(targetRef, readable, candidate).target;
          return target ? pageKeyForPage(target) === currentKey : false;
        })
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
    $("pageFolderIdInput").value = page?.folderId || state.selectedFolderId || "";
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
    return page?.title || metadata?.name || "Open a Brain";
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
      { id: "access", kind: "command", label: "Brain access", detail: "Settings", target: "access" },
      { id: "graph", kind: "command", label: "Graph View", detail: "Workspace", target: "graph" },
      { id: "new-page", kind: "command", label: "New Page", detail: "Current Folder", target: "new-page" },
      { id: "refresh", kind: "command", label: "Refresh Brain", detail: "Sync", target: "refresh" },
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

  function contextMenuItemsForTarget(
    target,
    metadata = state.metadata,
    actorNpub = state.pubkeyHex ? npubFromHex(state.pubkeyHex) : null
  ) {
    if (!target) return [];
    if (target.type === "page") {
      const discardLocalDraft = pageDeletionDisposition(target) === "discard-local";
      const pageKeyValue = target.pageKey || pageKey(target.folderId, target.objectId);
      const saveInFlight = state.pageSaveInFlight?.key === pageKeyValue;
      const items = [
        { action: "open-page", label: "Open Page" },
        { action: "new-page", label: "New Page in Folder" },
        { action: "open-graph", label: "Show in Graph View" },
        { separator: true },
        { action: "copy-page-id", label: "Copy Page ID" },
        { action: "copy-folder-id", label: "Copy Folder ID" },
      ];
      if (discardLocalDraft || actorHasDestructiveAuthority(metadata, actorNpub)) {
        items.push(
          { separator: true },
          {
          action: "delete-page",
          label: saveInFlight ? "Saving Page…" : discardLocalDraft ? "Discard unsaved Page" : "Delete Page",
          disabled: saveInFlight,
          danger: true,
          }
        );
      }
      return items;
    }
    const items = [
      { action: "open-folder", label: "Open Folder" },
      { action: "new-page", label: "New Page" },
      { action: "new-folder", label: "New Folder Inside" },
      { separator: true },
      { action: "copy-folder-id", label: "Copy Folder ID" },
      { action: "manage-access", label: "Manage Access" },
      { action: "share-folder", label: "Share Folder" },
    ];
    if (actorHasDestructiveAuthority(metadata, actorNpub)) {
      items.push(
        { separator: true },
        { action: "delete-folder", label: "Delete Folder", danger: true }
      );
    }
    return items;
  }

  function folderSubtreeSummary(folderId, metadata = state.metadata, pages = projectionPages()) {
    const folders = metadata?.folders || [];
    const ids = new Set([folderId]);
    for (;;) {
      const before = ids.size;
      for (const folder of folders) {
        if (folder.parentFolderId && ids.has(folder.parentFolderId)) ids.add(folder.id);
      }
      if (ids.size === before) break;
    }
    const root = folders.find((folder) => folder.id === folderId);
    return {
      folderIds: [...ids],
      folderCount: [...ids].filter((id) => folders.some((folder) => folder.id === id)).length,
      objectCount: (pages || []).filter((page) => ids.has(page.folderId) && !page.deleted).length,
      name: root?.name || root?.path || folderId,
    };
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
    shell.dataset.brainLoaded = state.metadata ? "true" : "false";
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
    const folderId = folderIdOverride || state.selectedFolderId;
    if (!folderId) {
      state.sessionNotice = "Create or select a Folder before adding a Page.";
      render();
      return;
    }
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
    const brainId = state.activeBrainId;
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
      disposition !== "discard-local" &&
      !actorHasDestructiveAuthority(state.metadata, currentActorNpub())
    ) {
      throw new Error("Your Brain role cannot permanently delete Pages");
    }
    if (
      window.confirm &&
      !window.confirm(
        disposition === "discard-local"
          ? `Discard unsaved "${title}"? This only removes the local draft.`
          : `Permanently delete "${title}"? This cannot be undone. Downloaded copies and backups may still exist.`
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
      brainId,
    });
    requireCurrentSessionEpoch(sessionEpoch);
    const route = `/_admin/brains/${encodeURIComponent(brainId)}/folders/${encodeURIComponent(
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
    log("Permanently deleted Page.", {
      folderId: page.folderId,
      objectId: page.objectId,
      revision: result.revision,
      sequence: result.sequence,
    });
    render();
  }

  async function deleteFolderFromContextTarget(target) {
    const sessionEpoch = captureSessionOperationEpoch();
    const brainId = state.activeBrainId;
    const folder = (state.metadata?.folders || []).find((row) => row.id === target.folderId);
    if (!folder) throw new Error("Select an existing Folder before deleting");
    if (!actorHasDestructiveAuthority(state.metadata, currentActorNpub())) {
      throw new Error("Your Brain role cannot permanently delete Folders");
    }
    const summary = folderSubtreeSummary(folder.id);
    const folderLabel = summary.folderCount === 1 ? "1 Folder" : `${summary.folderCount} Folders`;
    const objectLabel = summary.objectCount === 1 ? "1 item" : `${summary.objectCount} items`;
    if (
      window.confirm &&
      !window.confirm(
        `Permanently delete "${summary.name}" and its complete subtree (${folderLabel}, ${objectLabel})? This cannot be undone. Downloaded copies and backups may still exist.`
      )
    ) {
      return;
    }
    const deletionEvent = await buildAdminAccessChangeEvent({
      action: "delete-folder",
      folderId: folder.id,
      keyVersion: folder.currentKeyVersion || 1,
      note: "permanent Folder subtree deletion",
      brainId,
    });
    requireCurrentSessionEpoch(sessionEpoch);
    const route = `/_admin/brains/${encodeURIComponent(brainId)}/folders/${encodeURIComponent(folder.id)}`;
    const result = await protectedRequest(route, {
      method: "DELETE",
      // protectedRequest signs this complete body, binding the exact scope
      // shown in the destructive confirmation to the atomic store snapshot.
      body: JSON.stringify({
        deletionEvent,
        expectedFolderIds: summary.folderIds,
        expectedObjectCount: summary.objectCount,
      }),
    });
    requireCurrentSessionEpoch(sessionEpoch);
    const deletedIds = new Set(summary.folderIds);
    for (const [key, page] of state.projection.pages) {
      if (deletedIds.has(page.folderId)) state.projection.pages.delete(key);
    }
    for (const [key, page] of state.projection.localDrafts) {
      if (deletedIds.has(page.folderId)) state.projection.localDrafts.delete(key);
    }
    for (const key of [...(state.keyring?.keys?.keys?.() || [])]) {
      if (summary.folderIds.some((folderId) => key.startsWith(`${brainId}:${folderId}:`))) {
        state.keyring.keys.delete(key);
      }
    }
    await loadBrainMetadata({ preserveActive: true });
    requireCurrentSessionEpoch(sessionEpoch);
    selectDefaultReaderTargets();
    log("Permanently deleted Folder subtree.", { ...result, folderId: folder.id });
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
    const key = pageKeyForReference(reference, readablePages(), selectedReaderPage());
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
        copy: "Open a brain to build the local graph.",
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
    const folderId = $("pageFolderIdInput")?.value.trim() || state.selectedFolderId || "";
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
          ? "Open a brain to read pages."
          : "Brain locked. Open it to view your private content.";
      return;
    }
    if (!isReadablePage(page)) {
      content.className = "note-content note-content-empty";
      content.textContent = "This Page is private until the Brain is open.";
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
        brainInviteRecipientEmailInput: "createBrainInvitationButton",
        brainInviteExpiresAtInput: "createBrainInvitationButton",
        brainInviteCodeInput: "getBrainInvitationButton",
        brainInviteEmailInput: "getEmailInviteInstructionsButton",
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
        log("Failed to refresh Brain reader.", { error: error.message });
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

  async function copyBrainInviteUrl() {
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
    if (item.action === "delete-folder") {
      deleteFolderFromContextTarget(target).catch((error) => {
        state.lastError = error.message;
        log("Failed to permanently delete Folder.", { error: error.message });
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

  function renderBrainInvitationPanel() {
    renderBrainInvitationFolderOptions();
    if (!$("brainInviteExpiresAtInput").value) {
      $("brainInviteExpiresAtInput").value = defaultShareExpiryDateTimeLocal();
    }
    if (state.lastEmailInviteSecret && $("brainInviteSecretInput") && !$("brainInviteSecretInput").value) {
      $("brainInviteSecretInput").value = state.lastEmailInviteSecret;
    }
    const inviteUrlVisible =
      state.sessionStatus === SESSION_STATUS.UNLOCKED && Boolean(state.lastEmailInviteUrl);
    safeSetHidden("brainInviteUrlOutput", !inviteUrlVisible);
    const inviteUrlInput = $("brainInviteUrlInput");
    if (inviteUrlInput) inviteUrlInput.value = inviteUrlVisible ? state.lastEmailInviteUrl : "";
    setOptionalDisabled("copyBrainInviteUrlButton", !inviteUrlVisible);
    const controls = brainInvitationPanelState({
      activeBrainAvailable: Boolean(state.activeBrainId),
      busy: state.accessBusy,
      code: $("brainInviteCodeInput").value.trim() || state.lastBrainInvitationCode || "",
      email: $("brainInviteEmailInput")?.value,
      inviteSecret: $("brainInviteSecretInput")?.value,
      organizationBrain: state.metadata?.kind === "organization",
      sessionStatus: state.sessionStatus,
      signerCanConnect: deriveBrainIdentityProviderState(state.identityProvider).canConnect,
      signerStatus: state.signerStatus,
    });
    safeSetHidden("brainInviteConnectSignerButton", controls.connected);
    setOptionalDisabled("brainInviteConnectSignerButton", controls.connectDisabled);
    $("createBrainInvitationButton").disabled = controls.createDisabled;
    $("getBrainInvitationButton").disabled = controls.inspectDisabled;
    setOptionalDisabled("getEmailInviteInstructionsButton", controls.emailScopeDisabled);
    $("acceptBrainInvitationButton").disabled = controls.acceptDisabled;
    $("revokeBrainInvitationButton").disabled = controls.revokeDisabled;
    setText("brainInvitationHint", controls.hint);
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

    renderBrainAccessManagement(state.metadata);

    // Update access result panel (for feedback)
    renderAccessResultPanel();

    // Render brain admin panel
    renderBrainInvitationPanel();
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

  function actorIsBrainAdmin(metadata) {
    const actorNpub = state.pubkeyHex ? npubFromHex(state.pubkeyHex) : null;
    return actorHasDestructiveAuthority(metadata, actorNpub);
  }

  function actorHasDestructiveAuthority(metadata, actorNpub) {
    if (!metadata || !actorNpub) return false;
    if (metadata.kind === "personal") {
      return metadata.ownerUserId === actorNpub || metadata.personalAgent?.agentNpub === actorNpub;
    }
    return (metadata.admins || []).includes(actorNpub);
  }

  function actorCanCreateFolder(metadata, sessionStatus, actorNpub) {
    return (
      sessionStatus === SESSION_STATUS.UNLOCKED &&
      actorHasDestructiveAuthority(metadata, actorNpub)
    );
  }

  function hasOrganizationBrainControls(metadata) {
    return metadata?.kind === "organization";
  }

  function showsCreateOrganizationControl(metadata) {
    return !hasOrganizationBrainControls(metadata);
  }

  function canManageBrainPeople(metadata) {
    return (
      Boolean(metadata) &&
      hasOrganizationBrainControls(metadata) &&
      state.signerStatus === "connected" &&
      actorIsBrainAdmin(metadata) &&
      !state.accessBusy
    );
  }

  function linkStatusRank(status) {
    if (status === "pending" || status === "active") return 0;
    if (status === "accepted") return 1;
    return 2;
  }

  function brainInvitationRows(invitations) {
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
          counterpartBrainId:
            direction === "outgoing" ? connection.destinationBrainId : connection.sourceBrainId,
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
          counterpartBrainId:
            direction === "outgoing" ? invitation.destinationBrainId : invitation.sourceBrainId,
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

  function brainPeopleRows(metadata) {
    if (!metadata) return [];
    const brainPersonRow = (npub, role, type, removable) => {
      const identity = identityMetadataForNpub(npub);
      return {
        canMutate: Boolean(identity.email),
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
      return owner ? [brainPersonRow(owner, "owner", "owner", false)] : [];
    }
    const rows = [];
    const admins = uniqueNpubs(metadata.admins || []);
    const members = uniqueNpubs(metadata.members || []);
    for (const admin of admins) {
      rows.push(brainPersonRow(admin, "admin", "admin", true));
    }
    for (const member of members) {
      if (admins.includes(member)) continue;
      rows.push(brainPersonRow(member, "member", "member", true));
    }
    return rows;
  }

  function brainHealthBadges(metadata, signerStatus = state.signerStatus) {
    if (!metadata) {
      return [{ label: "no brain", tone: "muted" }];
    }
    const badges = [
      { label: metadata.kind === "organization" ? "organization" : "personal", tone: "ready" },
      { label: `${(metadata.folders || []).length} folders`, tone: "muted" },
      { label: `${metadata.grantCount || 0} grants`, tone: "muted" },
    ];
    badges.unshift(
      signerStatus === "connected"
        ? { label: "connection ready", tone: "ready" }
        : { label: "connection unavailable", tone: "warn" }
    );
    if ((metadata.mountedFolders || []).length) {
      badges.push({ label: `${metadata.mountedFolders.length} mounts`, tone: "muted" });
    }
    if (state.lastBrainInvitationCode) {
      badges.push({ label: "invite ready", tone: "ready" });
    }
    return badges;
  }

  function brainManagementSummary(metadata) {
    if (!metadata) {
      return state.sessionStatus === SESSION_STATUS.LOCKED
        ? "Choose a Brain, then unlock it to open encrypted content."
        : "Choose a Brain, then load it to decrypt its readable Folders.";
    }
    if (metadata.kind === "personal") {
      return "Personal Brain loaded. Use Access for Folder permissions and share links.";
    }
    return `Organization loaded. ${countLabel((metadata.members || []).length, "member")} • ${countLabel(
      (metadata.admins || []).length,
      "admin"
    )} • ${countLabel((metadata.folders || []).length, "Folder")}`;
  }

  function brainSwitchRowMeta(brain, isLoaded) {
    const kind = brain.kind === "personal" ? "personal" : "organization";
    const role = brain.role || (brain.kind === "personal" ? "owner" : "member");
    return `${kind} - ${role}${isLoaded ? " - loaded" : ""}`;
  }

  function brainSwitchButton(brain, surface = "management") {
    const isSelected = brain.brainId === state.activeBrainId;
    const isLoaded = state.metadata?.brainId === brain.brainId;
    const isLocked = state.sessionStatus === SESSION_STATUS.LOCKED && !state.visibleBrains.length;
    const statusText = isLoaded
      ? "loaded"
      : isLocked
        ? "locked"
        : isSelected
          ? "selected"
          : "available";
    const button = document.createElement("button");
    button.type = "button";
    button.dataset.brainId = brain.brainId;
    button.className = `brain-switch-button${isSelected ? " selected" : ""}${isLoaded ? " loaded" : ""}${isLocked ? " locked" : ""}`;
    button.setAttribute("aria-pressed", String(isSelected));
    button.setAttribute(
      "aria-label",
      `${brain.name || brain.brainId}, ${brainSwitchRowMeta(brain, isLoaded)}, ${
        statusText
      }`
    );
    button.addEventListener("click", () => {
      if (brain.brainId === state.activeBrainId) {
        if (surface === "switcher") closeBrainSwitcher();
        return;
      }
      setActiveBrainId(brain.brainId);
      log("Selected Brain.", { brainId: brain.brainId });
      if (surface === "switcher") {
        closeBrainSwitcher();
        return resumeSession().catch((error) => {
          reportClientActionFailure(error);
          log("Failed to load selected Brain from switcher.", { error: error.message });
          state.readerBusy = false;
          render();
        });
      }
      render();
    });

    const title = document.createElement("span");
    title.className = "brain-switch-title";
    title.textContent = brain.name || brain.brainId;

    const meta = document.createElement("span");
    meta.className = "brain-switch-meta";
    meta.textContent = brainSwitchRowMeta(brain, isLoaded);

    const status = document.createElement("span");
    status.className = `pill ${isLoaded ? "ready" : isLocked ? "warn" : isSelected ? "warn" : "muted"}`;
    status.textContent = statusText;
    status.setAttribute("aria-hidden", "true");

    button.appendChild(title);
    button.appendChild(meta);
    button.appendChild(status);
    return button;
  }

  function renderBrainAccessManagement(metadata) {
    const organizationBrain = hasOrganizationBrainControls(metadata);
    const inviteInProgress = Boolean(
      state.lastBrainInvitationCode ||
        $("brainInviteCodeInput")?.value.trim() ||
        $("brainInviteSecretInput")?.value.trim()
    );
    safeSetHidden("brainInvitationActionSection", !organizationBrain);
    safeSetElement("brainPeopleActionPanel", (panel) => {
      panel.hidden = !organizationBrain;
      if (!organizationBrain) panel.open = false;
    });
    safeSetHidden("brainPeopleSection", !organizationBrain);
    safeSetHidden("brainInvitationListSection", !organizationBrain);
    safeSetHidden("sharedFolderSection", !organizationBrain);
    safeSetElement("brainInvitationPanel", (panel) => {
      panel.hidden = false;
      if (inviteInProgress) {
        panel.open = true;
      }
    });
    renderBrainPeopleList(metadata);
    renderBrainPeopleControls(metadata);
    const actorNpub = state.pubkeyHex ? npubFromHex(state.pubkeyHex) : null;
    const showPersonalAgent = metadata?.kind === "personal" && metadata.ownerUserId === actorNpub;
    const currentAgentEmail = personalAgentEmail(metadata);
    const currentAgentUnresolved = Boolean(metadata?.personalAgent && !currentAgentEmail);
    safeSetHidden("personalAgentSection", !showPersonalAgent);
    safeSetText(
      "personalAgentCurrent",
      metadata?.personalAgent
        ? currentAgentEmail
          ? `Current: ${currentAgentEmail}`
          : "Current agent email unavailable. Changes are disabled."
        : "No Personal Agent is assigned."
    );
    setOptionalDisabled(
      "personalAgentEmailInput",
      state.accessBusy || !showPersonalAgent || currentAgentUnresolved
    );
    setOptionalDisabled(
      "replacePersonalAgentButton",
      state.accessBusy || !showPersonalAgent || currentAgentUnresolved
    );
    setOptionalDisabled(
      "removePersonalAgentButton",
      state.accessBusy || !showPersonalAgent || !metadata?.personalAgent || currentAgentUnresolved
    );
    renderBrainInvitationList();
    renderSharedFolderList();
  }

  function renderBrainPeopleList(metadata) {
    const rows = brainPeopleRows(metadata);
    setPill("brainPeopleCount", `${rows.length}`, rows.length ? "ready" : "muted");
    const emptyText = metadata?.kind === "personal"
      ? "Personal Brains do not use a member list."
      : "Load an Organization Brain to manage people.";
    const canManage = canManageBrainPeople(metadata);
    setList("brainPeopleList", rows, emptyText, (item, person) => {
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

      if (person.removable && person.canMutate && canManage) {
        const removeButton = document.createElement("button");
        removeButton.className = "access-remove-person brain-person-action";
        removeButton.type = "button";
        removeButton.textContent = person.type === "admin" ? "Remove admin" : "Remove";
        removeButton.addEventListener("click", () => {
          const action = person.type === "admin" ? removeBrainAdminFromPanel : removeBrainMemberFromPanel;
          action(person.id).catch((error) => {
            reportClientActionFailure(error);
            log("Failed to update Brain members.", { error: error.message });
          });
        });
        item.appendChild(removeButton);
      }
      item.appendChild(detailPanel);
    });
  }

  function renderBrainPeopleControls(metadata) {
    const canManage = canManageBrainPeople(metadata);
    setOptionalDisabled("addBrainMemberButton", !canManage);
    setOptionalDisabled("addBrainAdminButton", !canManage);
    const hint = !metadata
      ? "Load an Organization Brain to manage people."
      : metadata.kind !== "organization"
        ? "Personal Brains use Folder access and share links instead of member lists."
        : actorIsBrainAdmin(metadata)
          ? "Admins must already be Brain members."
          : "Only Brain admins can change organization members and admins.";
    setText("brainPeopleHint", hint);
    setText("brainPeopleActionHint", canManage ? "Invite, add, or promote by email" : "Admin-only");
  }

  function linkRowActionButton(label, onClick, options = {}) {
    const button = document.createElement("button");
    button.className = `access-remove-person brain-person-action${options.danger ? " danger-action" : ""}`;
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

  function renderBrainInvitationList() {
    const rows = brainInvitationRows(state.brainInvitations);
    const pendingCount = rows.filter((row) => row.status === "pending").length;
    setPill("brainInvitationCount", `${pendingCount}`, pendingCount ? "ready" : "muted");
    const emptyText = canLoadBrainAdminLists()
      ? "No invitations yet. Invite someone above."
      : "Brain admins see pending invitations here.";
    setList("brainInvitationList", rows, emptyText, (item, row) => {
      const identity = identityMetadataForNpub(row.targetNpub);
      const title = identity.email || `Invite code ${row.inviteCode}`;
      linkRowInfo(
        item,
        title,
        row.status,
        `${row.inviteCode} · expires ${row.expiresAt.slice(0, 10)}`
      );
      if (!row.revocable) return;
      item.appendChild(
        linkRowActionButton("Use code", async () => {
          rememberBrainInvitationSelection(row);
          setAccessResult("ready", "Invite code loaded", `${row.inviteCode} is in the invite field.`);
        })
      );
      item.appendChild(
        linkRowActionButton("Revoke", () => revokeBrainInvitationById(row.id), { danger: true })
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
    const emptyText = canLoadBrainAdminLists()
      ? "No shared Folders yet. Sharing across Brains starts with a shared Folder invitation."
      : "Brain admins see cross-Brain shared Folders here.";
    setList("sharedFolderList", rows, emptyText, (item, row) => {
      const directionLabel = row.direction === "outgoing" ? "to" : "from";
      const title = `${row.folderId} ${directionLabel} ${row.counterpartBrainId}`;
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
    const emptyText = canLoadBrainAdminLists()
      ? "No share links for this Folder yet."
      : "Brain admins see this Folder's share links here.";
    setList("folderShareLinkList", rows, emptyText, (item, linkRow) => {
      const identity = identityMetadataForNpub(linkRow.recipientNpub);
      linkRowInfo(
        item,
        identity.email || `Share link ${linkRow.id}`,
        linkRow.status,
        `${linkRow.id} · expires ${linkRow.expiresAt.slice(0, 10)}`
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
    setList("accessFolderList", rows, "Load a Brain to inspect access", (item, row) => {
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
      setText("accessSummaryLine", "Load a Brain and select a Folder to inspect access.");
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

        if (person.removable && person.canMutate && canManage) {
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
    const email =
      (typeof person === "object" ? identityEmailDisplay(person) : null) ||
      identityMetadataForNpub(id).email;
    accessList.push({
      canMutate: Boolean(email),
      id,
      name: email || accessPersonName(person),
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
          const personalAgent = metadata?.personalAgent?.agentNpub === userId;
          addAccessListPerson(
            accessList,
            member || userId,
            personalAgent ? "personal agent" : "explicit access",
            personalAgent ? "agent" : "explicit",
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
      addHint.textContent = "Connect securely to grant access to this Folder.";
    } else if (!folderAllowsDirectGrant(row) || !keyOpen) {
      addHint.textContent = accessFlowHint(row, "people", keyOpen);
    } else {
      addHint.textContent = row.access === "all_members"
        ? `Enter an existing member's email to update access for "${row.path}"`
        : `Enter an email to grant access to "${row.path}"`;
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
        shareHint.textContent = "Connect securely to create or accept share links.";
      } else if (!keyOpen) {
        shareHint.textContent = accessFlowHint(row, "links", keyOpen);
      } else if (!isRestricted) {
        shareHint.textContent = "Share links are for restricted Folders. Choose a restricted Folder to create one.";
      } else {
        shareHint.textContent = "The selected email receives a private, single-use share link.";
      }
    }
    if (shareMountHint) {
      shareMountHint.textContent = canCreateShare
        ? "When accepted, this adds a shortcut to the shared Folder in their Personal Brain. It does not copy data or change Folder access."
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
    const emptyState = readerEmptyStateCopy(
      state.metadata,
      state.sessionStatus,
      state.selectedFolderId,
      state.pubkeyHex ? npubFromHex(state.pubkeyHex) : null
    );

    setList("readerFolderList", folderRows, emptyState.list, (item, row) => {
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
      setText("readerPageTitle", emptyState.title);
      setText("readerPagePath", emptyState.path);
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
    setText("sessionAccountBrain", activeBrainLabel());
    setText("sessionAccountIdentity", sessionIdentityLabel());
    setText("sessionAccountStatus", view.title);
    const brainTrigger = $("sessionAccountBrainButton");
    brainTrigger?.setAttribute("aria-label", `Switch Brain (current: ${activeBrainLabel()})`);
    brainTrigger?.setAttribute("title", "Switch Brain");
    brainTrigger?.setAttribute("aria-expanded", String(state.brainSwitcherOpen));
    setText("sessionSecurityTitle", view.title);
    setText("sessionSecurityDetail", state.sessionNotice || view.detail);
    safeSetHidden("resumeSessionButton", !view.locked);
    safeSetHidden("lockSessionButton", view.locked);
    setOptionalDisabled(
      "resumeSessionButton",
      state.sessionStatus === SESSION_STATUS.RESUMING || !canLoadBrain()
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
    const actorNpub = state.pubkeyHex ? npubFromHex(state.pubkeyHex) : null;
    setOptionalDisabled(
      "obsidianNewPageButton",
      state.sessionStatus !== SESSION_STATUS.UNLOCKED || !(state.metadata?.folders || []).length
    );
    setOptionalDisabled(
      "obsidianNewFolderButton",
      !actorCanCreateFolder(state.metadata, state.sessionStatus, actorNpub)
    );
    setOptionalDisabled(
      "refreshReaderButton",
      state.sessionStatus !== SESSION_STATUS.UNLOCKED || state.readerBusy || state.signerStatus !== "connected" || !state.metadata
    );
    renderSessionSecurity();
    renderSettingsModal();
    renderBrainSwitcher();
    renderManageBrainsModal();
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

  function isActiveBrainAuthorizationLoss(error, activeBrainId) {
    if (
      !error ||
      error.status !== 403 ||
      error.reason !== BRAIN_ACCESS_REQUIRED_REASON
    ) {
      return false;
    }
    const brainId = String(activeBrainId || "").trim();
    if (!brainId) return false;
    const brainPath = `/_admin/brains/${encodeURIComponent(brainId)}`;
    return [
      `${brainPath}/metadata`,
      `${brainPath}/export`,
      `${brainPath}/sync/bootstrap`,
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
      if (lockSessionForBrainAccessChange(error, sessionEpoch)) {
        await refreshVisibleBrainsAfterAccessChange().catch((refreshError) => {
          log("Failed to refresh Brains after access changed.", { error: refreshError.message });
        });
      }
      throw error;
    }
    rememberIdentitiesFrom(body);
    return body;
  }

  async function loadVisibleBrains(options = {}) {
    if (state.signerStatus !== "connected") {
      state.visibleBrains = [];
      render();
      return [];
    }
    const previousIds = new Set(state.visibleBrains.map((brain) => normalizeVisibleBrain(brain)?.brainId).filter(Boolean));
    const response = await protectedRequest("/_admin/brains");
    state.visibleBrains = (response.brains || []).map(normalizeVisibleBrain).filter(Boolean);
    const selection = selectAccessibleBrain({
      brains: state.visibleBrains,
      currentBrainId: state.activeBrainId,
      explicitTargetBrainId: options.ignoreTarget
        ? null
        : options.explicitTargetBrainId || state.requestedBrainId || null,
    });
    if (selection.reason === "target_unavailable") {
      const error = new Error("You do not have access to the requested Brain yet");
      error.code = "brain_target_unavailable";
      error.targetBrainId = selection.targetBrainId;
      throw error;
    }
    setActiveBrainId(selection.brainId, { reset: false });
    if (selection.reason === "explicit_target") state.requestedBrainId = null;
    const discovered = state.visibleBrains.filter((brain) => !previousIds.has(brain.brainId));
    if (previousIds.size && discovered.length && selection.reason === "current_session") {
      state.sessionNotice = `${discovered.length} new ${discovered.length === 1 ? "Brain is" : "Brains are"} available.`;
    }
    render();
    return state.visibleBrains;
  }

  async function loadVisibleBrainsWithTargetRetry() {
    try {
      return await loadVisibleBrains();
    } catch (error) {
      if (error?.code !== "brain_target_unavailable") throw error;
      return loadVisibleBrains();
    }
  }

  async function createBrain(brainId, kind, name, options = {}) {
    const sessionEpoch = state.sessionEpoch;
    const agentIdentity =
      options.agentIdentity ||
      (kind === "personal" ? suggestedAgentIdentityFromNavigation() : null);
    const body = brainCreateBody({
      brainId,
      kind,
      name,
      bootstrapGrants: [],
      agentIdentity,
      includeAgentAdmin: options.includeAgentAdmin === true,
    });
    if (kind === "personal" && !body.personalAgentEmail && !body.personalAgentNpub) {
      throw new Error("Select your agent by email before creating your Personal Brain");
    }
    if (kind === "personal" && options.confirmPersonalAgent !== false) {
      if (!(await confirmPersonalBrainAgent(body))) {
        const error = new Error("Personal Brain setup was cancelled");
        error.code = "brain_setup_cancelled";
        throw error;
      }
    }
    requireCurrentSessionEpoch(sessionEpoch);
    const metadata = await protectedRequest("/_admin/brains", {
      method: "POST",
      body: JSON.stringify(body),
    });
    requireCurrentSessionEpoch(sessionEpoch);
    state.keyring = createSessionKeyring();
    return metadata;
  }

  function personalBrainAgentConfirmationMessage(body) {
    const agentLabel = body?.personalAgentEmail || body?.personalAgentNpub;
    if (!agentLabel) throw new Error("Personal Agent identity is required for confirmation");
    return `Create your Personal Brain and pair ${agentLabel} as your Personal Agent?`;
  }

  async function confirmPersonalBrainAgent(body) {
    const message = personalBrainAgentConfirmationMessage(body);
    const identity = String(body?.personalAgentEmail || body?.personalAgentNpub || "")
      .trim()
      .toLowerCase();
    const parentOrigin = String(
      document.querySelector('meta[name="finite-brain-parent-origin"]')?.getAttribute("content") || ""
    ).replace(/\/$/, "");
    if (!parentOrigin || !window.parent || window.parent === window) {
      return window.confirm ? window.confirm(message) : false;
    }
    const requestId = bytesToHex(crypto.getRandomValues(new Uint8Array(16)));
    return new Promise((resolve, reject) => {
      const request = {
        type: BRAIN_PERSONAL_AGENT_CONFIRMATION_REQUEST,
        requestId,
        identity,
      };
      const send = () => window.parent.postMessage(request, parentOrigin);
      let retry = null;
      const timeout = setTimeout(() => {
        if (retry) clearInterval(retry);
        window.removeEventListener("message", handleResponse);
        reject(new Error("Your dashboard could not confirm Personal Agent setup."));
      }, 5000);
      function handleResponse(event) {
        if (
          event.source !== window.parent ||
          event.origin !== parentOrigin ||
          event.data?.type !== BRAIN_PERSONAL_AGENT_CONFIRMATION_RESPONSE ||
          event.data?.requestId !== requestId ||
          typeof event.data.confirmed !== "boolean"
        ) {
          return;
        }
        clearTimeout(timeout);
        if (retry) clearInterval(retry);
        window.removeEventListener("message", handleResponse);
        resolve(event.data.confirmed);
      }
      window.addEventListener("message", handleResponse);
      send();
      retry = setInterval(send, 250);
    });
  }

  async function beginExplicitBrainCreation() {
    const beganLocked = state.sessionStatus === SESSION_STATUS.LOCKED;
    if (beganLocked) state.sessionStatus = SESSION_STATUS.RESUMING;
    const sessionEpoch = state.sessionEpoch;
    try {
      await connectSigner({ loadVisibleBrains: false, sessionEpoch });
      requireCurrentSessionEpoch(sessionEpoch);
      if (state.signerStatus !== "connected") throw new Error("Connect your Brain identity first");
      await loadVisibleBrains({ ignoreTarget: true });
      requireCurrentSessionEpoch(sessionEpoch);
      return { beganLocked, sessionEpoch };
    } catch (error) {
      if (beganLocked && state.sessionEpoch === sessionEpoch) resetBrainSessionState();
      throw error;
    }
  }

  async function finishExplicitBrainCreation(metadata, creation) {
    requireCurrentSessionEpoch(creation.sessionEpoch);
    rememberVisibleBrain(metadata);
    setActiveBrainId(metadata.brainId, { reset: false });
    state.metadata = metadata;
    state.keyring = state.keyring || createSessionKeyring();
    state.sessionStatus = SESSION_STATUS.UNLOCKED;
    await loadVisibleBrains();
    requireCurrentSessionEpoch(creation.sessionEpoch);
    render();
  }

  async function createPersonalBrainFromInput() {
    const creation = await beginExplicitBrainCreation();
    try {
      const existing = visibleBrainOptions().find((brain) => brain.kind === "personal");
      if (existing) {
        setActiveBrainId(existing.brainId, { reset: false });
        throw new Error("Your Personal Brain already exists. Open it instead.");
      }
      const email = String($("managePersonalAgentEmailInput")?.value || "").trim().toLowerCase();
      if (!looksLikeEmailIdentity(email)) {
        throw new Error("Enter your agent's Managed Agent Email");
      }
      const brainId = personalBrainIdForPubkey(state.pubkeyHex);
      const metadata = await createBrain(brainId, "personal", "Personal Brain", {
        agentIdentity: { email },
      });
      await finishExplicitBrainCreation(metadata, creation);
      log("Created Personal Brain.", { brainId: metadata.brainId });
    } catch (error) {
      if (creation.beganLocked && state.sessionEpoch === creation.sessionEpoch) resetBrainSessionState();
      throw error;
    }
  }

  async function ensureInvitedBrainAcceptedForActiveSelection() {
    const active = activeBrainOption();
    if (!active || active.role !== "invited" || !active.inviteCode) return;
    const invitation = await protectedRequest(brainInvitationAcceptPath(active.inviteCode), {
      method: "POST",
    });
    rememberBrainInvitationSelection(invitation);
    setActiveBrainId(invitation.brainId, { reset: false });
    await loadVisibleBrains();
  }

  async function createOrganizationBrainFromInput(inputId) {
    const input = $(inputId);
    const name = input?.value.trim() || "New organization";
    const includeAgentAdmin = Boolean($("manageOrganizationAddAgentInput")?.checked);
    const creation = await beginExplicitBrainCreation();
    try {
      const agentIdentity = suggestedAgentIdentityFromNavigation();
      const signature = JSON.stringify({
        agent: includeAgentAdmin ? agentIdentity?.email || agentIdentity?.npub || null : null,
        includeAgentAdmin,
        name,
      });
      if (state.pendingOrganizationCreation?.signature !== signature) {
        state.pendingOrganizationCreation = {
          brainId: brainIdFromName("org", name),
          signature,
        };
      }
      const brainId = state.pendingOrganizationCreation.brainId;
      const metadata = await createBrain(brainId, "organization", name, {
        agentIdentity,
        includeAgentAdmin,
      });
      if (input) input.value = "";
      await finishExplicitBrainCreation(metadata, creation);
      state.pendingOrganizationCreation = null;
      log("Created Organization Brain.", { brainId: metadata.brainId });
    } catch (error) {
      if (Number(error?.status) >= 400 && Number(error?.status) < 500) {
        state.pendingOrganizationCreation = null;
      }
      if (creation.beganLocked && state.sessionEpoch === creation.sessionEpoch) resetBrainSessionState();
      throw error;
    }
  }

  async function loadConfig() {
    const response = await fetch("/client/config.json");
    state.config = await response.json();
    log("Loaded Product Client config.", state.config);
    render();
  }

  async function detectSigner() {
    const hostedState = hostedIdentityProviderStates.get(state.identityProvider);
    if (hostedState) {
      try {
        await resumeSession();
        return;
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
    const hostedState = hostedIdentityProviderStates.get(provider);
    if (!derived.canConnect && hostedState?.status !== "checking") {
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
      resetBrainSessionState({ preserveManageBrainsReturnToSettings: false });
      setActiveBrainId(null, { reset: false });
    }
    state.pubkeyHex = pubkey;
    state.signerStatus = "connected";
    log(identityChanged ? "Connected a different signer identity." : "Connected signer.", {
      status: "connected",
    });
    if (options.loadVisibleBrains !== false && state.sessionStatus !== SESSION_STATUS.LOCKED) {
      await loadVisibleBrains().catch((error) => {
        state.lastError = error.message;
        log("Failed to load visible Brains.", { error: error.message });
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

  async function loadBrainMetadata(options = {}) {
    if (!options.preserveActive) {
      await ensureInvitedBrainAcceptedForActiveSelection();
    }
    if (!state.activeBrainId) throw new Error("Choose a Brain to open");
    const path = `/_admin/brains/${encodeURIComponent(state.activeBrainId)}/metadata`;
    const metadata = await protectedRequest(path);
    state.metadata = metadata;
    rememberVisibleBrain(metadata);
    log("Loaded Brain metadata.", metadata);
    render();
    if (state.settingsModalOpen && state.settingsSection === "access") {
      refreshAccessManagementListsInBackground();
    }
  }

  function canLoadBrainAdminLists() {
    return Boolean(
      state.metadata &&
        state.metadata.kind === "organization" &&
        state.signerStatus === "connected" &&
        actorIsBrainAdmin(state.metadata)
    );
  }

  async function refreshBrainAdminLists() {
    if (!canLoadBrainAdminLists()) {
      state.brainInvitations = null;
      state.sharedFolderInvitations = null;
      state.sharedFolderConnections = null;
      return;
    }
    const brainPath = `/_admin/brains/${encodeURIComponent(state.activeBrainId)}`;
    const invitationList = await protectedRequest(`${brainPath}/invitations`);
    state.brainInvitations = invitationList.invitations || [];
    state.sharedFolderInvitations = await protectedRequest(
      `${brainPath}/shared-folder-invitations`
    );
    state.sharedFolderConnections = await protectedRequest(
      `${brainPath}/shared-folder-connections`
    );
  }

  async function refreshFolderShareLinks(folderId) {
    if (!folderId || !canLoadBrainAdminLists()) {
      state.folderShareLinks = null;
      state.folderShareLinksFolderId = null;
      return;
    }
    const path = `/_admin/brains/${encodeURIComponent(
      state.activeBrainId
    )}/folders/${encodeURIComponent(folderId)}/share-links`;
    const list = await protectedRequest(path);
    state.folderShareLinks = list.shareLinks || [];
    state.folderShareLinksFolderId = folderId;
  }

  function refreshAccessManagementListsInBackground() {
    const work = async () => {
      await refreshBrainAdminLists();
      await refreshFolderShareLinks(state.activeAccessFolderId);
      render();
    };
    work().catch((error) => {
      log("Failed to refresh access management lists.", { error: error.message });
    });
  }

  async function revokeBrainInvitationById(invitationId) {
    requireUnlockedBrainInvitationAction("revoking an invitation");
    const sessionEpoch = captureSessionOperationEpoch();
    const brainId = state.activeBrainId;
    beginAccessOperation(sessionEpoch);
    try {
      const invitation = await protectedRequest(
        brainInvitationRevokePath(brainId, invitationId),
        { method: "DELETE" }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      setAccessResult("warn", "Invitation revoked", `${invitation.id} is ${invitation.status}.`, {
        updatedAt: invitation.updatedAt,
      });
      log("Revoked Brain invitation from pending list.", { invitationId });
      await refreshBrainAdminLists();
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
      await protectedRequest(
        `/_admin/shared-folder-invitations/${encodeURIComponent(invitationId)}/accept`,
        { method: "POST" }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      setAccessResult(
        "ready",
        "Shared Folder ready",
        "The shared Folder is now available in this Brain."
      );
      log("Accepted shared Folder invitation.", { invitationId });
      await loadBrainMetadata();
      requireCurrentSessionEpoch(sessionEpoch);
      await refreshBrainAdminLists();
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
      await refreshBrainAdminLists();
      requireCurrentSessionEpoch(sessionEpoch);
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function openAvailableFolderKeyGrants(options = {}) {
    if (!sessionGrantOpeningAllowed(state.sessionStatus)) {
      throw new Error("Brain is locked. Open the Brain before loading private content");
    }
    const sessionEpoch = state.sessionEpoch;
    const assertCurrent = () => requireCurrentSessionEpoch(sessionEpoch);
    assertCurrent();
    const keyring = options.keyring || state.keyring || createSessionKeyring();
    const brainId = options.brainId || state.activeBrainId;
    if (!options.keyring && !state.keyring) state.keyring = keyring;
    const exported = await protectedRequest(`/_admin/brains/${encodeURIComponent(brainId)}/export`);
    assertCurrent();
    const expectedRecipient = state.pubkeyHex ? npubFromHex(state.pubkeyHex) : null;
    return openFolderKeyGrants(keyring, exported, expectedRecipient, {
      assertCurrent,
      expectedBrainId: brainId,
    });
  }

  function canLoadBrain() {
    const provider = deriveBrainIdentityProviderState(state.identityProvider);
    return Boolean(
      state.config &&
        !state.readerBusy &&
        (state.signerStatus === "connected" || provider.canConnect)
    );
  }

  async function loadBrainReader(options = {}) {
    const allowResume = options.allowResume === true;
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED && !allowResume) {
      throw new Error("Brain is locked. Open the Brain before loading private content");
    }
    let relockOnFailure = state.sessionStatus !== SESSION_STATUS.UNLOCKED;
    let sessionEpoch = state.sessionEpoch;
    state.readerBusy = true;
    render();
    try {
      await connectSigner({ loadVisibleBrains: false, sessionEpoch });
      if (state.signerStatus !== "connected") throw new Error("Connect a Brain Identity Provider first");
      if (state.sessionStatus !== SESSION_STATUS.UNLOCKED && !allowResume) {
        throw new Error("The Brain identity changed. Open the Brain again");
      }
      relockOnFailure = relockOnFailure || state.sessionStatus !== SESSION_STATUS.UNLOCKED;
      if (state.sessionStatus !== SESSION_STATUS.UNLOCKED) state.sessionStatus = SESSION_STATUS.RESUMING;
      sessionEpoch = state.sessionEpoch;
      render();
      await loadVisibleBrainsWithTargetRetry();
      requireCurrentSessionEpoch(sessionEpoch);
      await loadBrainMetadata();
      requireCurrentSessionEpoch(sessionEpoch);
      const grants = await openAvailableFolderKeyGrants();
      requireCurrentSessionEpoch(sessionEpoch);
      await pullSyncBootstrap();
      requireCurrentSessionEpoch(sessionEpoch);
      selectDefaultReaderTargets();
      renderGraphView();
      state.sessionStatus = SESSION_STATUS.UNLOCKED;
      if (applyPendingInviteNavigation()) {
        state.sessionNotice = "Invitation details are ready in this open Brain.";
      }
      log("Loaded Brain reader.", {
        openedFolderKeys: grants.opened.length,
        skippedFolderKeyGrants: grants.skipped.length,
        readablePages: readablePages().length,
      });
    } catch (error) {
      if (relockOnFailure && state.sessionEpoch === sessionEpoch) resetBrainSessionState();
      throw error;
    } finally {
      if (state.sessionEpoch === sessionEpoch) state.readerBusy = false;
      render();
    }
  }

  async function refreshReader() {
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED) {
      throw new Error("Brain is locked. Open the Brain before refreshing private content");
    }
    const sessionEpoch = state.sessionEpoch;
    state.readerBusy = true;
    render();
    try {
      await loadVisibleBrainsWithTargetRetry();
      requireCurrentSessionEpoch(sessionEpoch);
      await loadBrainMetadata();
      requireCurrentSessionEpoch(sessionEpoch);
      if (state.keyring?.openedGrants.length) await pullSyncBootstrap();
      requireCurrentSessionEpoch(sessionEpoch);
      selectDefaultReaderTargets();
      log("Refreshed Brain reader.", {
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
    const folderId = $("pageFolderIdInput").value.trim() || state.selectedFolderId;
    if (!folderId) throw new Error("Create or select a Folder before saving a Page");
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
    if (!state.pubkeyHex) throw new Error("Connect securely first");
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
    const key = state.keyring?.keys.get(folderKeyId(state.activeBrainId, row.id, keyVersion));
    if (!key) throw new Error(`Open the Folder Key for ${row.path} before sharing`);
    return key;
  }

  function hasOpenedAccessFolderKey(row) {
    if (!row) return false;
    const keyVersion = row.currentKeyVersion || currentFolderKeyVersion(row.id);
    return Boolean(state.keyring?.keys.has(folderKeyId(state.activeBrainId, row.id, keyVersion)));
  }

  async function normalizedNpubValue(value, message) {
    const identity = await resolveIdentityInputValue(String(value || "").trim(), message);
    return identity.npub;
  }

  async function normalizedEmailNpubInput(inputId, message) {
    return normalizedEmailNpubValue($(inputId).value, message);
  }

  async function normalizedEmailNpubValue(value, message) {
    let email;
    try {
      email = canonicalInviteEmail(value);
    } catch (_) {
      throw new Error(message);
    }
    const identity = await resolveIdentityInputValue(email, message);
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

  function folderRecipientsForAccess(access, accessUserIds = [], metadata = state.metadata) {
    const recipients = new Set();
    if (metadata?.kind === "personal" && metadata.ownerUserId) {
      recipients.add(metadata.ownerUserId);
    }
    if (access === "owner") {
      if (metadata?.ownerUserId) recipients.add(metadata.ownerUserId);
      else recipients.add(currentActorNpub());
    } else {
      if (access === "admin_only" || access === "all_members" || access === "restricted") {
        for (const admin of metadata?.admins || []) recipients.add(admin);
      }
      if (access === "all_members") {
        for (const member of metadata?.members || []) recipients.add(member);
      }
      if (access === "restricted") {
        for (const user of accessUserIds) recipients.add(user);
      }
    }
    if (metadata?.kind === "personal" && metadata.personalAgent?.agentNpub) {
      recipients.add(metadata.personalAgent.agentNpub);
    }
    if (!recipients.size) recipients.add(currentActorNpub());
    return [...recipients];
  }

  async function createFolderFromToolbar(parentFolderId = null) {
    if (!state.metadata) throw new Error("Open a Brain before creating a Folder");
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED) {
      throw new Error("Brain is locked. Open the Brain before creating a Folder");
    }
    const sessionEpoch = state.sessionEpoch;
    const brainId = state.activeBrainId;
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
          brainId,
        })
      );
      requireCurrentSessionEpoch(sessionEpoch);
    }
    await importFolderKey(
      sessionKeyring,
      {
        brainId,
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
      `/_admin/brains/${encodeURIComponent(brainId)}/folders`,
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

  function brainInvitationExpiryIso() {
    const value = $("brainInviteExpiresAtInput").value.trim();
    const date = value ? new Date(value) : new Date(Date.now() + 7 * 24 * 60 * 60 * 1000);
    if (Number.isNaN(date.getTime())) throw new Error("Brain invitation expiry is invalid");
    return date.toISOString();
  }

  function emailProofCreatedAtIso() {
    return new Date().toISOString();
  }

  function initialBrainInvitationFolders(value = null) {
    if (value === null) return [...state.brainInvitationFolderIds];
    const values = Array.isArray(value)
      ? value
      : String(value || "")
          .split(/[,\s]+/)
          .map((part) => part.trim())
          .filter(Boolean);
    return uniqueValues(values);
  }

  function renderBrainInvitationFolderOptions() {
    const container = $("brainInviteFoldersOptions");
    if (!container) return;
    const folders = metadataFolderRows(state.metadata).filter(
      (folder) => folder.access === "restricted"
    );
    const availableIds = new Set(folders.map((folder) => folder.id));
    for (const folderId of [...state.brainInvitationFolderIds]) {
      if (!availableIds.has(folderId)) state.brainInvitationFolderIds.delete(folderId);
    }
    container.replaceChildren();
    if (!folders.length) {
      const empty = document.createElement("p");
      empty.className = "access-field-hint";
      empty.textContent = state.metadata
        ? "This Brain has no restricted Folders to add."
        : "Load a Brain to choose restricted Folders.";
      container.appendChild(empty);
      return;
    }
    for (const folder of folders) {
      const label = document.createElement("label");
      label.className = "settings-folder-choice";
      const input = document.createElement("input");
      input.type = "checkbox";
      input.value = folder.id;
      input.checked = state.brainInvitationFolderIds.has(folder.id);
      input.addEventListener("change", () => {
        if (input.checked) state.brainInvitationFolderIds.add(folder.id);
        else state.brainInvitationFolderIds.delete(folder.id);
      });
      const name = document.createElement("span");
      name.textContent = folder.path || folder.name || folder.id;
      label.appendChild(input);
      label.appendChild(name);
      container.appendChild(label);
    }
  }

  function buildBrainInvitationRequest(input) {
    const targetNpub = input.targetNpub;
    npubToHex(targetNpub);
    return {
      targetNpub,
      initialFolderAccess: initialBrainInvitationFolders(input.initialFolderAccess || ""),
      expiresAt: input.expiresAt,
    };
  }

  function brainInvitationIdentifierHint(input) {
    const value = String(input || "").trim();
    if (!value) return null;
    if (value.startsWith("invitation-")) {
      return "That is an internal invitation reference. Paste the Invite Code that starts with invite-.";
    }
    if (!value.startsWith("invite-")) {
      return "Invite Codes start with invite-. Check the copied code and secure connection.";
    }
    return null;
  }

  function brainInvitationPanelState(input = {}) {
    const code = String(input.code || "").trim();
    const connected = input.signerStatus === "connected";
    const unlocked = input.sessionStatus === SESSION_STATUS.UNLOCKED;
    const busy = Boolean(input.busy);
    const organizationBrain = Boolean(input.organizationBrain);
    const codeHint = brainInvitationIdentifierHint(code);
    const inviteCodeUsable = Boolean(code) && !codeHint;
    const emailProvided = Boolean(String(input.email || "").trim());
    const secretProvided = Boolean(String(input.inviteSecret || "").trim());
    const emailClaimIncomplete = emailProvided !== secretProvided;
    const emailClaimReady = Boolean(
      inviteCodeUsable && emailProvided && secretProvided
    );
    const protectedActionDisabled = !connected || !unlocked || busy;
    let hint;
    if (!unlocked) {
      hint = "Open the Brain to inspect, accept, or manage invitations.";
    } else if (!connected) {
      hint = "Connect securely";
    } else if (codeHint) {
      hint = codeHint;
    } else if (inviteCodeUsable && emailClaimIncomplete) {
      hint = "Open the private invite link to verify an email invitation.";
    } else if (inviteCodeUsable) {
      hint = "Ready to join Brain";
    } else {
      hint = "Enter an Invite Code";
    }
    return {
      acceptDisabled: protectedActionDisabled || !inviteCodeUsable || emailClaimIncomplete,
      codeHint,
      connectDisabled: busy || !input.signerCanConnect,
      connected,
      createDisabled:
        protectedActionDisabled || !organizationBrain || input.activeBrainAvailable === false,
      emailScopeDisabled: protectedActionDisabled || !emailClaimReady,
      hint,
      inspectDisabled: protectedActionDisabled || !inviteCodeUsable,
      inviteCodeUsable,
      revokeDisabled: protectedActionDisabled || !organizationBrain || !code,
    };
  }

  function requireUnlockedBrainInvitationAction(action) {
    if (state.sessionStatus !== SESSION_STATUS.UNLOCKED) {
      throw new Error(`Brain is locked. Open the Brain before ${action}`);
    }
  }

  function clearRememberedEmailInvitationMaterial() {
    const rememberedSecret = state.lastEmailInviteSecret;
    const secretInput = $("brainInviteSecretInput");
    if (rememberedSecret && secretInput?.value === rememberedSecret) {
      secretInput.value = "";
    }
    state.lastEmailInviteSecret = null;
    state.lastEmailInviteUrl = null;
    const inviteUrlInput = $("brainInviteUrlInput");
    if (inviteUrlInput) inviteUrlInput.value = "";
    safeSetHidden("brainInviteUrlOutput", true);
    setOptionalDisabled("copyBrainInviteUrlButton", true);
  }

  function rememberBrainInvitationSelection(invitation) {
    const inviteCode = String(invitation?.inviteCode || "").trim();
    const invitationId = String(invitation?.id || "").trim() || null;
    const changed = inviteCode !== state.lastBrainInvitationCode;
    state.lastBrainInvitationCode = inviteCode || null;
    state.lastBrainInvitationId = invitationId;
    if (changed) {
      state.lastEmailInvitePostProof = null;
      clearRememberedEmailInvitationMaterial();
    }
    const codeInput = $("brainInviteCodeInput");
    if (codeInput) codeInput.value = inviteCode;
  }

  function handleBrainInvitationInput(inputId) {
    if (inputId === "brainInviteCodeInput") {
      const inviteCode = $("brainInviteCodeInput")?.value.trim() || "";
      if (inviteCode !== state.lastBrainInvitationCode) {
        state.lastBrainInvitationCode = inviteCode || null;
        state.lastBrainInvitationId = null;
        state.lastEmailInvitePostProof = null;
        clearRememberedEmailInvitationMaterial();
      }
    } else if (inputId === "brainInviteEmailInput") {
      state.lastEmailInvitePostProof = null;
    }
    renderBrainInvitationPanel();
  }

  function brainInvitationRevokeTarget(input = {}) {
    const value = String(input.input || "").trim();
    if (!value) throw new Error("Paste an Invite Code or invitation id first");
    const brainId = String(input.activeBrainId || "").trim();
    if (!brainId) throw new Error("Select a Brain before revoking an invitation");
    const invitations = input.invitations || [];
    const knownInvitation = invitations.find(
      (invitation) => invitation?.id === value || invitation?.inviteCode === value
    );
    if (knownInvitation?.id) {
      return { invitationId: knownInvitation.id, brainId: knownInvitation.brainId || brainId };
    }
    if (
      value === String(input.lastBrainInvitationCode || "").trim() &&
      input.lastBrainInvitationId
    ) {
      return { invitationId: input.lastBrainInvitationId, brainId };
    }
    if (value.startsWith("invitation-")) {
      return { invitationId: value, brainId };
    }
    throw new Error(
      "Revoke an invitation created by this Brain admin from the pending invitation list, or paste its invitation id."
    );
  }

  function currentBrainInvitationInput() {
    const value = $("brainInviteCodeInput").value.trim() || state.lastBrainInvitationCode;
    if (!value) throw new Error("Paste an Invite Code or invitation id first");
    return value;
  }

  function currentBrainInvitationCode() {
    const value = currentBrainInvitationInput();
    const hint = brainInvitationIdentifierHint(value);
    if (hint) throw new Error(hint);
    return value;
  }

  function brainInvitationUnavailableDetail(error) {
    const message = error?.message || String(error || "");
    if (message === "brain invitation unavailable") {
      return "Brain invitation unavailable. Check the Invite Code, secure connection, expiry, or whether the invite was already used.";
    }
    return message;
  }

  function activeSignerInviteDetail() {
    if (state.signerStatus !== "connected" || !state.pubkeyHex) return "Connect securely";
    return "Connection ready. Invitations are bound to the recipient email.";
  }

  function brainInvitationCreatePath(brainId) {
    return `/_admin/brains/${encodeURIComponent(brainId)}/invitations`;
  }

  function suggestedAgentIdentityFromNavigation(search = window.location?.search || "") {
    let params;
    try {
      params = new URLSearchParams(search);
    } catch (_) {
      return null;
    }
    const emailCandidate = params.get("agentEmail")?.trim().toLowerCase() || "";
    const email =
      emailCandidate.length <= 254 && looksLikeEmailIdentity(emailCandidate)
        ? emailCandidate
        : null;
    const nameCandidate = params.get("agentName")?.trim() || "";
    const name =
      nameCandidate.length > 0 &&
      nameCandidate.length <= 80 &&
      !/[\u0000-\u001f\u007f]/u.test(nameCandidate)
        ? nameCandidate
        : null;
    const npubCandidate = params.get("agentNpub")?.trim() || "";
    const npub = publicKeyIdentityFromInput(npubCandidate)?.npub || null;
    return email || npub ? { email, name, npub } : null;
  }

  function brainCreateBody(input) {
    const body = {
      brainId: input.brainId,
      kind: input.kind,
      name: input.name,
      bootstrapGrants: input.bootstrapGrants || [],
    };
    if (input.kind === "organization") {
      if (!input.includeAgentAdmin) return body;
      const email = String(input.agentIdentity?.email || "").trim().toLowerCase();
      if (email && looksLikeEmailIdentity(email)) {
        body.initialAgentEmail = email;
        return body;
      }
      const npub = publicKeyIdentityFromInput(input.agentIdentity?.npub)?.npub;
      if (npub) body.initialAgentNpub = npub;
      return body;
    }
    const email = String(input.agentIdentity?.email || "").trim().toLowerCase();
    if (email && looksLikeEmailIdentity(email)) {
      body.personalAgentEmail = email;
      return body;
    }
    const npub = publicKeyIdentityFromInput(input.agentIdentity?.npub)?.npub;
    if (npub) body.personalAgentNpub = npub;
    return body;
  }

  function brainInvitationLinkPath(code) {
    return `/_admin/brain-invitation-links/${encodeURIComponent(code)}`;
  }

  function brainInvitationAcceptPath(code) {
    return `${brainInvitationLinkPath(code)}/accept`;
  }

  function brainInvitationRevokePath(brainId, invitationId) {
    return `/_admin/brains/${encodeURIComponent(brainId)}/invitations/${encodeURIComponent(invitationId)}`;
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
    const recipients = uniqueNpubs(
      folderRecipientsForAccess(row.access, remainingAccessUsers, metadata)
    );
    if (!recipients.length) throw new Error("Folder Key rotation needs at least one remaining recipient");
    return { remainingAccessUsers, recipients };
  }

  function liveReadableFolderObjects(objects, folderId) {
    const rows = (objects || [])
      .filter((object) => object.folderId === folderId && !object.deleted)
      .sort((left, right) => String(left.objectId).localeCompare(String(right.objectId)));
    const unreadable = rows.filter(
      (object) =>
        object.status !== "ready" ||
        (isAssetObject(object)
          ? typeof object.bytesBase64 !== "string"
          : typeof object.text !== "string")
    );
    if (unreadable.length) {
      throw new Error("Every live object in this Folder must be readable before rotating access");
    }
    return rows;
  }

  function validateFolderRotationFanout(operation, rotations) {
    const personalAgent = operation === "personal-agent";
    if (!personalAgent && operation !== "folder-access-removal") {
      throw new Error("Unknown Folder rotation operation");
    }
    const operationLabel = personalAgent ? "Personal Agent rotation" : "Folder access removal";
    const maxRotations = personalAgent ? MAX_PERSONAL_AGENT_ROTATION_FOLDERS : 1;
    const maxTotalGrants = personalAgent
      ? MAX_PERSONAL_AGENT_ROTATION_GRANTS
      : MAX_FOLDER_ROTATION_GRANTS;
    const maxTotalRecords = personalAgent
      ? MAX_PERSONAL_AGENT_ROTATION_RECORDS
      : MAX_FOLDER_ROTATION_RECORDS;
    if (rotations.length > maxRotations) {
      throw new Error(
        `${operationLabel} exceeds Folder rotations limit: ${rotations.length} supplied, maximum ${maxRotations}`
      );
    }
    let totalGrants = 0;
    let totalRecords = 0;
    for (const rotation of rotations) {
      const grants = Number(rotation.grants || 0);
      const records = Number(rotation.reencryptedRecords || 0);
      if (!Number.isSafeInteger(grants) || grants < 0 || !Number.isSafeInteger(records) || records < 0) {
        throw new Error("Folder rotation fanout counts must be non-negative integers");
      }
      if (grants > MAX_FOLDER_ROTATION_GRANTS) {
        throw new Error(
          `${operationLabel} exceeds grants per Folder rotation limit: ${grants} supplied, maximum ${MAX_FOLDER_ROTATION_GRANTS}`
        );
      }
      if (records > MAX_FOLDER_ROTATION_RECORDS) {
        throw new Error(
          `${operationLabel} exceeds re-encrypted records per Folder rotation limit: ${records} supplied, maximum ${MAX_FOLDER_ROTATION_RECORDS}`
        );
      }
      totalGrants += grants;
      totalRecords += records;
      if (totalGrants > maxTotalGrants) {
        throw new Error(
          `${operationLabel} exceeds aggregate grants limit: ${totalGrants} supplied, maximum ${maxTotalGrants}`
        );
      }
      if (totalRecords > maxTotalRecords) {
        throw new Error(
          `${operationLabel} exceeds aggregate re-encrypted records limit: ${totalRecords} supplied, maximum ${maxTotalRecords}`
        );
      }
    }
  }

  function randomFolderKeyBytes() {
    return crypto.getRandomValues(new Uint8Array(32));
  }

  function deterministicClientId(prefix, parts) {
    return sha256Hex(parts.join("\n")).then((digest) => `${prefix}-${digest.slice(0, 16)}`);
  }

  function canonicalAdminAccessChangePayload(input) {
    const fields = [
      `"version":${JSON.stringify("finite-brain-admin-access-change-v1")}`,
      `"brainId":${JSON.stringify(input.brainId)}`,
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
      ["d", `finite-brain-admin-access-change:${input.brainId}:${input.changeId}`],
      ["brain", input.brainId],
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
    const signEvent = requireBrainEventAuthorizer("brain-access-change", input);
    const createdAtUnix = input.createdAtUnix || Math.floor(Date.now() / 1000);
    const createdAt = accessChangeCreatedAt(createdAtUnix);
    const adminNpub = input.adminNpub || currentActorNpub();
    const brainId = input.brainId || state.activeBrainId;
    const changeId =
      input.changeId ||
      (await deterministicClientId("access-change", [
        brainId,
        input.action,
        input.folderId || "-",
        input.targetNpub || "-",
        createdAt,
      ]));
    const payload = {
      version: "finite-brain-admin-access-change-v1",
      brainId,
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
        input.brainId,
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
        brainId: input.brainId,
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
      brainId: input.brainId,
      folderId: input.folderId,
      keyVersion: input.keyVersion,
      folderKey,
      issuerNpub,
      recipientNpub: input.recipientNpub,
      createdAt,
    };
    const rumorTags = [
      ["d", `finite-folder-key-grant:${input.brainId}:${input.folderId}:${input.keyVersion}`],
      ["brain", input.brainId],
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
      : initialBrainInvitationFolders(selectedFolders || "");
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
      brainId: input.brainId,
      invitedEmail: input.invitedEmail,
      inviteUnwrapNpub: input.inviteUnwrapNpub,
      bootstrapPayloadHash: input.bootstrapPayloadHash,
      expiresAt: input.expiresAt,
      folders: emailInviteScopeJson(input.scope),
    });
  }

  function emailInviteAuthorizationTags(input) {
    return [
      ["d", `finite-email-invite-bootstrap-authorization:${input.brainId}:${input.invitedEmail}`],
      ["brain", input.brainId],
      ["email", input.invitedEmail],
    ];
  }

  async function buildEmailInviteAuthorizationEvent(input) {
    const signEvent = requireBrainEventAuthorizer("brain-invite-authorization", input);
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
      brainId: input.brainId,
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
        purpose: "brain-invite-bootstrap",
        brainId: input.brainId,
        recipientNpub: input.inviteUnwrapNpub,
        plaintext: input.bootstrapPayloadJson,
        createdAtUnixSeconds: createdAtUnix,
      });
    }
    const signSeal = requireBrainEventAuthorizer("brain-invite-bootstrap-seal", input);
    const signWrap = requireBrainEventAuthorizer("brain-invite-bootstrap-wrap", input);
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
        ["d", `finite-email-invite-bootstrap:${input.brainId}`],
        ["brain", input.brainId],
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

  function openedKeyForScopeItem(keyring, brainId, item) {
    const key = keyring?.keys?.get(folderKeyId(brainId, item.folderId, item.keyVersion));
    if (!key) throw new Error(`Open Folder Key for ${item.folderId} v${item.keyVersion} before creating the invite`);
    return key;
  }

  async function buildEmailBrainInvitationRequest(keyring, input) {
    const invitedEmail = canonicalInviteEmail(input.target || input.invitedEmail);
    const brainId = input.brainId || state.activeBrainId;
    const issuerNpub = input.issuerNpub || currentActorNpub();
    const inviteKeypair = input.inviteKeypair || createInviteUnwrapKeypair();
    const inviteUnwrapNpub = inviteKeypair.npub || inviteKeypair.inviteUnwrapNpub;
    const inviteSecret = inviteKeypair.secretHex || inviteKeypair.inviteSecret;
    const scope = input.scope || emailInviteScope(input.metadata || state.metadata, input.initialFolderAccess || []);
    const initialFolderAccess =
      input.initialFolderAccess === undefined || input.initialFolderAccess === null
        ? scope.filter((folder) => folder.access === "restricted").map((folder) => folder.folderId)
        : initialBrainInvitationFolders(input.initialFolderAccess || "");
    const bootstrapGrants = [];
    for (const item of scope) {
      const key = openedKeyForScopeItem(keyring, brainId, item);
      bootstrapGrants.push({
        folderId: item.folderId,
        grant: await buildFolderKeyGrantRequest({
          id: input.grantIdFactory ? input.grantIdFactory(item) : undefined,
          brainId,
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
      brainId,
      invitedEmail,
      inviteUnwrapNpub,
      scope,
      grants: bootstrapGrants,
    });
    const bootstrapPayloadJson = JSON.stringify(bootstrapPayload);
    const bootstrapPayloadHash = `sha256:${await sha256Hex(bootstrapPayloadJson)}`;
    const bootstrapWrappedEventJson = await buildEmailInviteBootstrapWrappedEvent({
      ...input,
      brainId,
      issuerNpub,
      inviteUnwrapNpub,
      bootstrapPayloadJson,
      brainIdentityProvider: input.brainIdentityProvider,
      signEvent: input.signEvent,
    });
    const bootstrapAuthorizationEventJson = JSON.stringify(
      await buildEmailInviteAuthorizationEvent({
        ...input,
        brainId,
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
    return `${brainInvitationLinkPath(code)}/bootstrap`;
  }

  function emailInviteInstructionsPath(code) {
    return `${brainInvitationLinkPath(code)}/instructions`;
  }

  function emailInviteClaimPath(code) {
    return `${brainInvitationLinkPath(code)}/claim`;
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
      brainId: input.brainId,
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
    if (payload.brainId !== invitation.brainId) throw new Error("Email Invite Bootstrap Brain mismatch");
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
          brainId: plaintext.brainId,
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
        brainId: invitation.brainId,
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
      brainId: state.activeBrainId,
      folderId: row.id,
      keyVersion: key.keyVersion,
      rawKey: key.rawKey,
      recipientNpub,
    });
  }

  async function buildBrainPeopleMutationRequest(action, targetNpub) {
    npubToHex(targetNpub);
    return {
      targetNpub,
      accessChangeEvent: await buildAdminAccessChangeEvent({
        action,
        targetNpub,
      }),
    };
  }

  async function mutateBrainPeople(path, options, sessionEpoch) {
    requireCurrentSessionEpoch(sessionEpoch);
    const metadata = await protectedRequest(path, options);
    requireCurrentSessionEpoch(sessionEpoch);
    state.metadata = metadata;
    rememberVisibleBrain(metadata);
    try {
      await loadVisibleBrains();
    } catch (error) {
      requireCurrentSessionEpoch(sessionEpoch);
      log("Failed to refresh visible Brains after member update.", { error: error.message });
    }
    requireCurrentSessionEpoch(sessionEpoch);
    return metadata;
  }

  async function addBrainMemberFromPanel() {
    const sessionEpoch = captureSessionOperationEpoch();
    const brainId = state.activeBrainId;
    const targetNpub = await normalizedEmailNpubInput("brainMemberEmailInput", "Enter a valid member email first");
    requireCurrentSessionEpoch(sessionEpoch);
    beginAccessOperation(sessionEpoch);
    try {
      const body = JSON.stringify(await buildBrainPeopleMutationRequest("add-member", targetNpub));
      requireCurrentSessionEpoch(sessionEpoch);
      await mutateBrainPeople(`/_admin/brains/${encodeURIComponent(brainId)}/members`, {
        method: "POST",
        body,
      }, sessionEpoch);
      requireCurrentSessionEpoch(sessionEpoch);
      $("brainMemberEmailInput").value = "";
      setAccessResult("ready", "Member added", `${identityDisplay(targetNpub)} can now belong to this Brain.`);
      log("Added Brain member.", { targetNpub: identityDisplay(targetNpub), brainId });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Add member failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function addBrainAdminFromPanel() {
    const sessionEpoch = captureSessionOperationEpoch();
    const brainId = state.activeBrainId;
    const targetNpub = await normalizedEmailNpubInput("brainAdminEmailInput", "Enter a valid member email first");
    requireCurrentSessionEpoch(sessionEpoch);
    beginAccessOperation(sessionEpoch);
    try {
      const body = JSON.stringify(await buildBrainPeopleMutationRequest("add-admin", targetNpub));
      requireCurrentSessionEpoch(sessionEpoch);
      await mutateBrainPeople(`/_admin/brains/${encodeURIComponent(brainId)}/admins`, {
        method: "POST",
        body,
      }, sessionEpoch);
      requireCurrentSessionEpoch(sessionEpoch);
      $("brainAdminEmailInput").value = "";
      setAccessResult("ready", "Admin added", `${identityDisplay(targetNpub)} can manage this Brain.`);
      log("Added Brain admin.", { targetNpub: identityDisplay(targetNpub), brainId });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Add admin failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function removeBrainMemberFromPanel(targetNpub) {
    const sessionEpoch = captureSessionOperationEpoch();
    const brainId = state.activeBrainId;
    beginAccessOperation(sessionEpoch);
    try {
      const accessChangeEvent = await buildAdminAccessChangeEvent({
        action: "remove-member",
        targetNpub,
      });
      requireCurrentSessionEpoch(sessionEpoch);
      await mutateBrainPeople(
        `/_admin/brains/${encodeURIComponent(brainId)}/members/${encodeURIComponent(targetNpub)}`,
        {
          method: "DELETE",
          body: JSON.stringify({ accessChangeEvent }),
        },
        sessionEpoch
      );
      requireCurrentSessionEpoch(sessionEpoch);
      setAccessResult("warn", "Member removed", `${identityDisplay(targetNpub)} was removed from this Brain.`);
      log("Removed Brain member.", { targetNpub: identityDisplay(targetNpub), brainId });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Remove member failed", error);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function removeBrainAdminFromPanel(targetNpub) {
    const sessionEpoch = captureSessionOperationEpoch();
    const brainId = state.activeBrainId;
    beginAccessOperation(sessionEpoch);
    try {
      const accessChangeEvent = await buildAdminAccessChangeEvent({
        action: "remove-admin",
        targetNpub,
      });
      requireCurrentSessionEpoch(sessionEpoch);
      await mutateBrainPeople(
        `/_admin/brains/${encodeURIComponent(brainId)}/admins/${encodeURIComponent(targetNpub)}`,
        {
          method: "DELETE",
          body: JSON.stringify({ accessChangeEvent }),
        },
        sessionEpoch
      );
      requireCurrentSessionEpoch(sessionEpoch);
      setAccessResult("warn", "Admin removed", `${identityDisplay(targetNpub)} is still a member.`);
      log("Removed Brain admin.", { targetNpub: identityDisplay(targetNpub), brainId });
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
    const brainId = input.brainId || state.activeBrainId;
    const metadata = input.metadata || state.metadata;
    const targetNpub = input.targetNpub;
    if (targetNpub) npubToHex(targetNpub);
    if (!targetNpub && !input.recipients) {
      throw new Error("Folder access removal requires a target identity");
    }
    const currentKeyVersion = row.currentKeyVersion || 1;
    const currentKey = keyring.keys.get(folderKeyId(brainId, row.id, currentKeyVersion));
    if (!currentKey) throw new Error(`Open the Folder Key for ${row.path} before removing access`);

    const recipients = input.recipients
      ? uniqueNpubs(input.recipients)
      : folderAccessRemovalRecipients(metadata, row, targetNpub).recipients;
    const liveObjects = input.liveObjects || liveReadableFolderObjects(input.objects, row.id);
    validateFolderRotationFanout("folder-access-removal", [{
      grants: recipients.length,
      reencryptedRecords: liveObjects.length,
    }]);
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
      brainId,
      folderId: row.id,
      keyVersion: newKeyVersion,
      folderKey,
    });

    const grants = [];
    for (const recipientNpub of recipients) {
      grants.push(
        await buildFolderKeyGrantRequest({
          brainId,
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
    for (const object of liveObjects) {
      const plaintext = isAssetObject(object)
        ? await encodeFolderObjectAssetPlaintext(
            object.path,
            base64ToBytes(object.bytesBase64),
            object.contentType || "application/octet-stream"
          )
        : encodeFolderObjectPagePlaintext(object.path || `${object.objectId}.md`, object.text);
      const write = await buildPageWriteRequest(keyring, {
        authorNpub: actorNpub,
        baseRevision: object.revision,
        createdAtUnix,
        folderId: row.id,
        keyVersion: newKeyVersion,
        objectId: object.objectId,
        operation: "update",
        plaintext,
        signEvent: requireBrainEventAuthorizer("folder-object-revision", input),
        brainId,
      });
      reencryptedRecords.push({
        objectId: object.objectId,
        ...write,
      });
    }

    const accessChangeEvent = await buildAdminAccessChangeEvent({
      action: input.action || "remove-folder-access",
      adminNpub: actorNpub,
      createdAtUnix,
      folderId: row.id,
      keyVersion: newKeyVersion,
      brainIdentityProvider: input.brainIdentityProvider,
      provider: input.provider,
      signEvent: input.signEvent,
      targetNpub: input.eventTargetNpub === undefined ? targetNpub : input.eventTargetNpub,
      brainId,
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

  async function replacePersonalAgentFromPanel(remove = false) {
    const sessionEpoch = captureSessionOperationEpoch();
    const metadata = state.metadata;
    const brainId = state.activeBrainId;
    const actorNpub = currentActorNpub();
    if (metadata?.kind !== "personal" || metadata.ownerUserId !== actorNpub) {
      throw new Error("Only the Personal Brain owner can replace its Personal Agent");
    }
    const oldAgent = metadata.personalAgent?.agentNpub;
    if (remove && !oldAgent) throw new Error("No Personal Agent is assigned");
    const oldAgentEmail = personalAgentEmail(metadata);
    if (oldAgent && !oldAgentEmail) {
      throw new Error("Current Personal Agent email is unavailable. Refresh before changing access");
    }
    const agentEmail = remove ? null : $("personalAgentEmailInput")?.value.trim().toLowerCase();
    if (!remove && !looksLikeEmailIdentity(agentEmail)) throw new Error("Enter the replacement agent email");
    if (
      window.confirm &&
      !window.confirm(
        remove
          ? `Remove ${oldAgentEmail} as your Personal Agent? Access will be updated securely; your Brain and content will remain.`
          : oldAgent
            ? `Replace ${oldAgentEmail} with ${agentEmail} as your Personal Agent? Access will be updated securely.`
            : `Assign ${agentEmail} as your Personal Agent? Access will be updated securely.`
      )
    ) return;
    const replacementNpub = remove
      ? null
      : await normalizedNpubValue(agentEmail, "Enter the replacement agent email");
    requireCurrentSessionEpoch(sessionEpoch);
    const rotationMetadata = {
      ...metadata,
      personalAgent: replacementNpub ? { agentNpub: replacementNpub } : null,
    };
    const objects = projectionPages();
    const rotationPlans = metadataFolderRows(metadata).map((row) => {
      const recipients = folderRecipientsForAccess(row.access, row.accessUserIds, rotationMetadata);
      const liveObjects = liveReadableFolderObjects(objects, row.id);
      return { row, recipients, liveObjects };
    });
    validateFolderRotationFanout(
      "personal-agent",
      rotationPlans.map((plan) => ({
        grants: plan.recipients.length,
        reencryptedRecords: plan.liveObjects.length,
      }))
    );
    const operationKeyring = cloneSessionKeyring(state.keyring);
    const rotations = [];
    for (const { row, recipients, liveObjects } of rotationPlans) {
      const rotation = await buildFolderAccessRemovalRequest(operationKeyring, {
        action: "rotate-folder-key",
        actorNpub,
        eventTargetNpub: replacementNpub,
        metadata: rotationMetadata,
        objects,
        recipients,
        liveObjects,
        row,
        targetNpub: oldAgent || null,
        brainId,
      });
      rotations.push({ folderId: row.id, ...rotation });
      requireCurrentSessionEpoch(sessionEpoch);
    }
    const updated = await protectedRequest(
      `/_admin/brains/${encodeURIComponent(brainId)}/personal-agent`,
      {
        method: "PUT",
        body: JSON.stringify({ agentEmail, rotations }),
      }
    );
    requireCurrentSessionEpoch(sessionEpoch);
    state.keyring = operationKeyring;
    state.metadata = updated;
    if ($("personalAgentEmailInput")) $("personalAgentEmailInput").value = "";
    await refreshReader();
  }

  async function grantFolderAccessFromPanel(targetValue) {
    const sessionEpoch = captureSessionOperationEpoch();
    const brainId = state.activeBrainId;
    const row = requireGrantableAccessRow();
    const targetNpub = await normalizedEmailNpubValue(targetValue, "Enter a valid email first");
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
        `/_admin/brains/${encodeURIComponent(brainId)}/folders/${encodeURIComponent(row.id)}/access`,
        { method: "POST", body }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      state.metadata = metadata;
      setAccessResult("ready", "Access granted", `${identityDisplay(targetNpub)} can open ${row.path}.`);
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
    const brainId = state.activeBrainId;
    const row = requireRestrictedAccessRow();
    const targetNpub = await normalizedNpubValue(targetValue, "Choose a person first");
    requireCurrentSessionEpoch(sessionEpoch);
    const operationKeyring = cloneSessionKeyring(state.keyring);
    const metadataSnapshot = state.metadata;
    const objectSnapshot = [...state.projection.pages.values()];
    beginAccessOperation(sessionEpoch);
    try {
      const removal = await buildFolderAccessRemovalRequest(operationKeyring, {
        brainId,
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
        `/_admin/brains/${encodeURIComponent(brainId)}/folders/${encodeURIComponent(
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
      setAccessResult("warn", "Access removed", `${identityDisplay(targetNpub)} was removed from ${row.path}.`);
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
    const brainId = state.activeBrainId;
    const row = requireRestrictedAccessRow();
    const recipientNpub = await normalizedEmailNpubInput("accessShareTargetInput", "Enter a valid recipient email first");
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
        `/_admin/brains/${encodeURIComponent(brainId)}/folders/${encodeURIComponent(row.id)}/share-links`,
        { method: "POST", body }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      state.lastShareLinkId = shareLink.id;
      $("accessShareLinkInput").value = shareLink.id;
      setAccessResult(
        "ready",
        "Share link created",
        `The private share link is ready for ${identityDisplay(recipientNpub)}.`,
        { Expires: shareLink.expiresAt }
      );
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
      await loadBrainMetadata();
      requireCurrentSessionEpoch(sessionEpoch);
      await openAvailableFolderKeyGrants();
      requireCurrentSessionEpoch(sessionEpoch);
      await pullSyncBootstrap();
      requireCurrentSessionEpoch(sessionEpoch);
      selectDefaultReaderTargets();
      setAccessResult(
        "ready",
        shareLink.duplicateAccept ? "Share link already accepted" : "Share link accepted",
        "The shared Folder is now available in this Brain."
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

  async function createBrainInvitationFromPanel() {
    requireUnlockedBrainInvitationAction("creating an invitation");
    const sessionEpoch = state.sessionEpoch;
    const brainId = state.activeBrainId;
    const metadata = state.metadata;
    const publicBaseUrl = state.config?.publicBaseUrl;
    const targetInput = canonicalInviteEmail($("brainInviteRecipientEmailInput").value);
    state.accessBusy = true;
    state.accessResult = null;
    render();
    try {
      let body;
      let localInviteSecret = null;
      let targetLabel = targetInput;
      let resolvedNpub = null;
      if (finiteVipEmail(targetInput)) {
        try {
          resolvedNpub = (await resolveIdentityInputValue(targetInput, "Enter a valid email address first")).npub;
          requireCurrentSessionEpoch(sessionEpoch);
        } catch (error) {
          if (state.sessionEpoch !== sessionEpoch) throw error;
          resolvedNpub = null;
        }
      }
      if (resolvedNpub) {
        body = JSON.stringify(
          buildBrainInvitationRequest({
            targetNpub: resolvedNpub,
            initialFolderAccess: initialBrainInvitationFolders(),
            expiresAt: brainInvitationExpiryIso(),
          })
        );
        targetLabel = targetInput;
      } else {
        const sessionKeyring = state.keyring || createSessionKeyring();
        await openAvailableFolderKeyGrants({ keyring: sessionKeyring, brainId });
        requireCurrentSessionEpoch(sessionEpoch);
        const request = await buildEmailBrainInvitationRequest(sessionKeyring, {
          target: targetInput,
          metadata,
          initialFolderAccess: initialBrainInvitationFolders(),
          expiresAt: brainInvitationExpiryIso(),
          brainId,
        });
        requireCurrentSessionEpoch(sessionEpoch);
        body = JSON.stringify(request.body);
        localInviteSecret = request.inviteSecret;
        state.keyring = sessionKeyring;
      }
      requireCurrentSessionEpoch(sessionEpoch);
      const invitation = await protectedRequest(
        brainInvitationCreatePath(brainId),
        { method: "POST", body }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      rememberBrainInvitationSelection(invitation);
      if (localInviteSecret && invitation.targetKind === "email_bootstrap") {
        const invitedEmail = invitation.invitedEmail || canonicalInviteEmail(targetInput);
        state.lastEmailInviteSecret = localInviteSecret;
        state.lastEmailInviteUrl = emailInviteClientUrl({
          publicBaseUrl,
          inviteCode: invitation.inviteCode,
          invitedEmail,
          inviteSecret: localInviteSecret,
        });
        $("brainInviteSecretInput").value = localInviteSecret;
        $("brainInviteEmailInput").value = invitedEmail;
        $("brainInviteUrlInput").value = state.lastEmailInviteUrl;
      } else {
        clearRememberedEmailInvitationMaterial();
      }
      const invitationAccessDetail = invitation.targetKind === "email_bootstrap"
        ? "They can claim the selected Folder access after verifying the invited email."
        : "They can join with this one-time invite; an admin may still need to finish their Folder access.";
      setAccessResult("ready", "Invitation created", `${targetLabel} can join ${invitation.brainId}. ${invitationAccessDetail}`, {
        "Invite Code": invitation.inviteCode,
        Expires: invitation.expiresAt,
        Recipient: invitation.invitedEmail || targetLabel,
      });
      log("Created Brain invitation.", {
        invitationId: invitation.id,
        targetKind: invitation.targetKind,
        brainId: invitation.brainId,
      });
      await refreshBrainAdminLists();
    } catch (error) {
      markAccessFailureHandled(error);
      if (state.sessionEpoch === sessionEpoch) {
        setAccessResult("error", "Invite failed", brainInvitationUnavailableDetail(error));
      }
      throw error;
    } finally {
      if (state.sessionEpoch === sessionEpoch) {
        state.accessBusy = false;
        render();
      }
    }
  }

  async function inspectBrainInvitationFromPanel() {
    requireUnlockedBrainInvitationAction("inspecting an invitation");
    const sessionEpoch = captureSessionOperationEpoch();
    const code = currentBrainInvitationCode();
    beginAccessOperation(sessionEpoch);
    try {
      const invitation = await protectedRequest(brainInvitationLinkPath(code));
      requireCurrentSessionEpoch(sessionEpoch);
      rememberBrainInvitationSelection(invitation);
      setAccessResult("ready", "Invitation loaded", `${identityDisplay(invitation.userId)} is ${invitation.status}.`, {
        Brain: invitation.brainId,
        Status: invitation.status,
        Email: identityDisplay(invitation.userId),
      });
      log("Loaded Brain invitation.", { invitationId: invitation.id, brainId: invitation.brainId });
      return invitation;
    } catch (error) {
      failAccessOperation(sessionEpoch, "Inspect failed", error, brainInvitationUnavailableDetail);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function loadEmailInviteInstructionsFromPanel() {
    requireUnlockedBrainInvitationAction("verifying email and loading invitation access");
    const sessionEpoch = captureSessionOperationEpoch();
    const code = currentBrainInvitationCode();
    const email = canonicalInviteEmail($("brainInviteEmailInput").value);
    const inviteSecret = $("brainInviteSecretInput").value.trim();
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
      rememberBrainInvitationSelection(invitation);
      state.lastEmailInvitePostProof = invitation;
      const folderCount = (invitation.bootstrapScope || []).length;
      setAccessResult("ready", "Email verified", `${email} is verified. Selected Folder access is ready to claim.`, {
        "Invite Code": invitation.inviteCode,
        Folders: countLabel(folderCount, "Folder"),
        Status: invitation.status,
      });
      log("Verified email invitation scope.", {
        invitationId: invitation.id,
        brainId: invitation.brainId,
      });
      return invitation;
    } catch (error) {
      failAccessOperation(sessionEpoch, "Email verification failed", error, brainInvitationUnavailableDetail);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function acceptBrainInvitationFromPanel() {
    requireUnlockedBrainInvitationAction("accepting an invitation");
    const code = currentBrainInvitationCode();
    const email = $("brainInviteEmailInput")?.value.trim();
    const inviteSecret = $("brainInviteSecretInput")?.value.trim();
    if (email && inviteSecret) {
      return claimEmailBrainInvitationFromPanel(code);
    }
    if (email || inviteSecret) {
      throw new Error("Open the private invite link to verify an email invitation");
    }
    const sessionEpoch = captureSessionOperationEpoch();
    beginAccessOperation(sessionEpoch);
    try {
      const invitation = await protectedRequest(brainInvitationAcceptPath(code), {
        method: "POST",
      });
      requireCurrentSessionEpoch(sessionEpoch);
      await loadVisibleBrains({ ignoreTarget: true });
      requireCurrentSessionEpoch(sessionEpoch);
      setActiveBrainId(invitation.brainId);
      state.sessionNotice = invitation.duplicateAccept
        ? "This person already joined the selected Brain. An admin must finish granting access before private content can open."
        : "Joined the selected Brain. An admin must finish granting access before private content can open.";
      render();
      log("Accepted Brain invitation.", { invitationId: invitation.id, brainId: invitation.brainId });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Accept failed", error, brainInvitationUnavailableDetail);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function claimEmailBrainInvitationFromPanel(code) {
    requireUnlockedBrainInvitationAction("claiming Folder access");
    const sessionEpoch = captureSessionOperationEpoch();
    const email = canonicalInviteEmail($("brainInviteEmailInput").value);
    const inviteSecret = $("brainInviteSecretInput").value.trim();
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
      await loadVisibleBrains({ ignoreTarget: true });
      requireCurrentSessionEpoch(sessionEpoch);
      setActiveBrainId(claimed.brainId);
      state.sessionNotice = claimed.duplicateAccept
        ? "Email invitation was already claimed. Open the selected Brain to continue."
        : "Email invitation claimed. Open the selected Brain to continue.";
      render();
      log("Claimed email Brain invitation.", {
        invitationId: claimed.id,
        brainId: claimed.brainId,
      });
    } catch (error) {
      failAccessOperation(sessionEpoch, "Claim failed", error, brainInvitationUnavailableDetail);
      throw error;
    } finally {
      finishAccessOperation(sessionEpoch);
    }
  }

  async function revokeBrainInvitationFromPanel() {
    requireUnlockedBrainInvitationAction("revoking an invitation");
    const sessionEpoch = captureSessionOperationEpoch();
    const value = currentBrainInvitationInput();
    const target = brainInvitationRevokeTarget({
      activeBrainId: state.activeBrainId,
      input: value,
      invitations: state.brainInvitations,
      lastBrainInvitationCode: state.lastBrainInvitationCode,
      lastBrainInvitationId: state.lastBrainInvitationId,
    });
    beginAccessOperation(sessionEpoch);
    try {
      const invitation = await protectedRequest(
        brainInvitationRevokePath(target.brainId, target.invitationId),
        { method: "DELETE" }
      );
      requireCurrentSessionEpoch(sessionEpoch);
      rememberBrainInvitationSelection(invitation);
      setAccessResult("warn", "Invitation revoked", `${invitation.id} is ${invitation.status}.`, {
        updatedAt: invitation.updatedAt,
      });
      log("Revoked Brain invitation.", { invitationId: invitation.id, brainId: invitation.brainId });
      await refreshBrainAdminLists();
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
    if (!state.pubkeyHex) throw new Error("Connect securely before saving this Page");
    const sessionEpoch = state.sessionEpoch;
    const keyring = state.keyring;
    const brainId = state.activeBrainId;
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
      brainId,
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
    const brainId = state.activeBrainId;
    const preparedWrite = state.preparedWrite;
    const savedInput = activePageInput();
    const target = state.preparedWriteTarget || savedInput;
    const savedText = savedInput.text;
    const savedPath = target.path || savedInput.path || `${target.objectId}.md`;
    const path = `/_admin/brains/${encodeURIComponent(brainId)}/folders/${encodeURIComponent(
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
    const brainId = state.activeBrainId;
    const path = `/_admin/brains/${encodeURIComponent(brainId)}/sync/bootstrap`;
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
    $("sessionAccountBrainButton")?.addEventListener("click", () => {
      openBrainSwitcher();
    });
    $("manageBrainsButton")?.addEventListener("click", () => {
      openManageBrainsModal();
    });
    $("closeManageBrainsButton")?.addEventListener("click", () => {
      closeManageBrainsModal();
    });
    $("manageBrainsModal")?.addEventListener("click", (event) => {
      if (event.target === $("manageBrainsModal")) closeManageBrainsModal();
    });
    $("manageBrainsConnectSignerButton")?.addEventListener("click", () => {
      connectSigner().catch((error) => {
        state.lastError = error.message;
        log("Failed to connect signer from Manage Brains.", { error: error.message });
        render();
      });
    });
    $("manageBrainsLoadButton")?.addEventListener("click", () => {
      manageBrainsLoadAction();
    });
    $("manageCreatePersonalBrainButton")?.addEventListener("click", () => {
      createPersonalBrainFromInput().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to create Personal Brain from Manage Brains.", { error: error.message });
        render();
      });
    });
    $("manageCreateOrganizationBrainButton")?.addEventListener("click", () => {
      createOrganizationBrainFromInput("manageOrganizationBrainNameInput").catch((error) => {
        reportClientActionFailure(error);
        log("Failed to create Organization Brain from Manage Brains.", { error: error.message });
        render();
      });
    });
    $("manageOrganizationBrainNameInput")?.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") return;
      event.preventDefault();
      $("manageCreateOrganizationBrainButton")?.click?.();
    });
    $("settingsManageBrainsButton")?.addEventListener("click", () => {
      openManageBrainsModal({ returnToSettings: true });
    });
    $("closeSettingsButton")?.addEventListener("click", () => {
      closeSettingsModal();
    });
    $("settingsNavSession")?.addEventListener("click", () => {
      setSettingsSection("session");
    });
    $("settingsNavBrain")?.addEventListener("click", () => {
      setSettingsSection("brain");
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
        $("settingsNavBrain"),
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
      setSettingsSection(["session", "brain", "access", "invitations"][nextIndex] || "session");
    });
    bindAccessFolderSelector();
    $("refreshReaderButton").addEventListener("click", () => {
      refreshReader().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to refresh Brain reader.", { error: error.message });
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
    onOptionalClick("addBrainMemberButton", () => {
      addBrainMemberFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to add Brain member.", { error: error.message });
      });
    });
    onOptionalClick("addBrainAdminButton", () => {
      addBrainAdminFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to add Brain admin.", { error: error.message });
      });
    });
    onOptionalClick("replacePersonalAgentButton", () => {
      replacePersonalAgentFromPanel(false).catch((error) => reportClientActionFailure(error));
    });
    onOptionalClick("removePersonalAgentButton", () => {
      replacePersonalAgentFromPanel(true).catch((error) => reportClientActionFailure(error));
    });
    for (const [inputId, buttonId] of [
      ["brainMemberEmailInput", "addBrainMemberButton"],
      ["brainAdminEmailInput", "addBrainAdminButton"],
      ["personalAgentEmailInput", "replacePersonalAgentButton"],
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
    onOptionalClick("createBrainInvitationButton", () => {
      createBrainInvitationFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to create Brain invitation.", { error: error.message });
      });
    });
    onOptionalClick("copyBrainInviteUrlButton", () => {
      void copyBrainInviteUrl();
    });
    onOptionalClick("getBrainInvitationButton", () => {
      inspectBrainInvitationFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to inspect Brain invitation.", { error: error.message });
      });
    });
    onOptionalClick("getEmailInviteInstructionsButton", () => {
      loadEmailInviteInstructionsFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to load email Brain invitation scope.", { error: error.message });
      });
    });
    onOptionalClick("acceptBrainInvitationButton", () => {
      acceptBrainInvitationFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to accept Brain invitation.", { error: error.message });
      });
    });
    onOptionalClick("brainInviteConnectSignerButton", () => {
      connectSigner().catch((error) => {
        state.lastError = error.message;
        log("Failed to connect signer for Brain invitation.", { error: error.message });
        render();
      });
    });
    onOptionalClick("revokeBrainInvitationButton", () => {
      revokeBrainInvitationFromPanel().catch((error) => {
        reportClientActionFailure(error);
        log("Failed to revoke Brain invitation.", { error: error.message });
      });
    });
    for (const inputId of [
      "accessShareTargetInput",
      "accessShareExpiresAtInput",
      "accessShareLinkInput",
      "brainInviteRecipientEmailInput",
      "brainInviteExpiresAtInput",
      "brainInviteCodeInput",
      "brainInviteEmailInput",
    ]) {
      bindPrimaryFormAction(inputId);
    }
    for (const inputId of [
      "brainInviteCodeInput",
      "brainInviteEmailInput",
    ]) {
      const input = $(inputId);
      if (input) input.addEventListener("input", () => handleBrainInvitationInput(inputId));
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
      const brainSwitcher = $("brainSwitcherMenu");
      const brainSwitcherTrigger = $("sessionAccountBrainButton");
      if (
        state.brainSwitcherOpen &&
        brainSwitcher &&
        !brainSwitcher.contains(event.target) &&
        !brainSwitcherTrigger?.contains?.(event.target)
      ) {
        closeBrainSwitcher();
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
      if (state.manageBrainsModalOpen) {
        if (event.key === "Escape") {
          event.preventDefault();
          closeManageBrainsModal();
          return;
        }
        if (event.key === "Tab") {
          const focusable = manageBrainsModalFocusableElements();
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
      if (state.brainSwitcherOpen) {
        if (event.key === "Escape") {
          event.preventDefault();
          closeBrainSwitcher();
          return;
        }
        if (event.key === "Tab") {
          event.preventDefault();
          moveBrainSwitcherFocusOut({ backwards: event.shiftKey });
          return;
        }
        const direction =
          event.key === "ArrowDown" ? 1 :
          event.key === "ArrowUp" ? -1 :
          event.key === "Home" ? 0 :
          event.key === "End" ? Number.POSITIVE_INFINITY :
          null;
        if (direction !== null) {
          const items = brainSwitcherFocusableElements();
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
      rememberBrainInvitationSelection({ inviteCode: pending.inviteCode });
      populated = true;
    }
    if (pending.inviteEmail) {
      if ($("brainInviteEmailInput")) $("brainInviteEmailInput").value = pending.inviteEmail;
      populated = true;
    }
    if (pending.inviteSecret) {
      state.lastEmailInviteSecret = pending.inviteSecret;
      if ($("brainInviteSecretInput")) $("brainInviteSecretInput").value = pending.inviteSecret;
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
    state.requestedBrainId = brainTargetFromSearch(window.location?.search);
    await loadConfig();
    await detectSigner();
  }

  return {
    accessActionRoute,
    accessBadgesForFolder,
    accessIntentValue,
    accessPanelState,
    accessPeopleSummary,
    actorHasDestructiveAuthority,
    actorCanCreateFolder,
    adminAccessChangeTags,
    buildAdminAccessChangeEvent,
    buildFolderKeyGrantRequest,
    buildPageDeleteRequest,
    buildPageWriteRequest,
    buildAuthEventTemplate,
    buildBrainAuthorizationHeader,
    brainTargetFromSearch,
    brainIdFromName,
    buildFolderAccessRemovalRequest,
    buildEmailInviteAuthorizationEvent,
    buildEmailInviteClaimProofEvent,
    buildEmailInviteClaimRequest,
    buildEmailBrainInvitationRequest,
    buildBrainInvitationRequest,
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
    emailInviteAuthorizationTags,
    emailInviteBootstrapPath,
    emailInviteClaimPath,
    emailInviteClientUrl,
    emailInviteInstructionsPath,
    emailInviteScope,
    emailInviteScopeJson,
    decodeFolderObjectPlaintext,
    encryptFolderObject,
    encodeFolderObjectAssetPlaintext,
    encodeFolderObjectPagePlaintext,
    editorSlashCommandRows,
    extractPageLinks,
    folderAllowsDirectGrant,
    folderCreationHierarchy,
    folderCreationParent,
    folderRecipientsForAccess,
    validateFolderRotationFanout,
    folderSubtreeSummary,
    folderShareLinkRows,
    graphEmptyStateCopy,
    graphLayout,
    graphNeighborIds,
    graphStats,
    graphViewBoxForZoom,
    handlePageHide,
    handlePageShow,
    inlineLinkSegments,
    initialBrainInvitationFolders,
    isActiveBrainAuthorizationLoss,
    applyPendingInviteNavigation,
    inviteNavigationFromHash,
    inviteUnwrapKeypairFromSecret,
    nip44DecryptWithSecret,
    nip44EncryptWithSecret,
    markdownFromEditorElement,
    markdownPreviewBlocks,
    mergeSyncProjection,
    metadataBrainRole,
    metadataFolderRows,
    metadataMountRows,
    nextDraftObjectId,
    normalizeSidebarMode,
    normalizeSettingsSection,
    normalizeVisibleBrain,
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
    personalBrainAgentConfirmationMessage,
    personalBrainIdForPubkey,
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
    readerEmptyStateCopy,
    readerSearchHighlightForPage,
    readerPageDetail,
    readerPageRows,
    resumeSession,
    searchHighlightSegments,
    searchPageRows,
    searchResultSnippet,
    selectAccessibleBrain,
    settingsSectionsForSession,
    sharedFolderRelationshipRows,
    sessionGrantOpeningAllowed,
    sessionOperationIsCurrent,
    sessionStatusView,
    suggestedAgentIdentityFromNavigation,
    signedEventMatchesPinnedIdentity,
    signerIdentityChanged,
    hasOrganizationBrainControls,
    showsCreateOrganizationControl,
    brainCreateBody,
    sidebarAccessBadgesForFolder,
    sidebarModeLabel,
    shortKey,
    start,
    lockSession,
    rememberIdentity,
    identityMetadataForNpub,
    identityDisplay,
    clientFailureMessage,
    lockedBrainSelection,
    visibleBrainOptions,
    brainHealthBadges,
    workspaceChromeState,
    workspaceTabTitle,
    brainInvitationAcceptPath,
    brainInvitationCreatePath,
    brainInvitationIdentifierHint,
    brainInvitationLinkPath,
    brainInvitationPanelState,
    brainInvitationRevokePath,
    brainInvitationRevokeTarget,
    brainInvitationRows,
    brainInvitationUnavailableDetail,
    brainPeopleRows,
    toggleMarkdownTask,
    taskCheckboxAriaLabel,
  };
})();

window.FiniteBrainProductClient = FiniteBrainProductClient;
if (!window.__FINITE_BRAIN_DISABLE_AUTOSTART__) {
  FiniteBrainProductClient.start();
}
