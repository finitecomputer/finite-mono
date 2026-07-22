const path = require("node:path");
const fs = require("node:fs");
const os = require("node:os");
const { spawn } = require("node:child_process");
const { app, BrowserWindow, ipcMain, protocol, safeStorage, session, shell } = require("electron");
const {
  attachmentActionUsesBinaryTransport,
  forwardAttachmentUpload,
} = require("./attachment-upload.cjs");
const {
  attachmentMediaScheme,
  parseAttachmentMediaUrl,
} = require("./attachment-media.cjs");
const {
  MAX_DESKTOP_ACTION_BYTES,
  assertDesktopChatAction,
  dashboardDestination,
  isAllowedUnprivilegedNavigation,
  isDashboardDocumentUrl,
  isDashboardOriginUrl,
  isGoogleWorkspaceStartUrl,
  normalizeDashboardBaseUrl,
  parseAccountBinding,
  parseDeviceLinkPublicRequest,
  parseDeviceLinkPublicResponse,
  parseLocalDaemonIdentity,
  trustedDashboardIpcFrame,
} = require("./dashboard-security.cjs");
const {
  AccountSecretStore,
  DaemonUpdateRelay,
  DaemonSupervisor,
  DeviceLinkSupervisor,
  archiveRevokedDeviceProfile,
  daemonRequestVersionMatches,
  deviceLinkFailureMessage,
  loadOrCreateDeviceId,
  resolveDaemonBinary,
  startDaemonRuntime,
  validDeviceId,
} = require("./daemon-process.cjs");

let mainWindow = null;
let authWindow = null;
let dashboardSession = null;
let daemonSupervisor = null;
let activeDeviceLink = null;
let daemonFailure = null;
let daemonLifecycle = Promise.resolve();
let updateAbortController = null;
let updateGeneration = 0;
let quitAfterDaemonStops = false;
let daemonShutdownPromise = null;
let daemonUpdateRelay = null;
let accountSecretStore = null;
let localDeviceLifecycle = null;
let verifiedLocalDevice = null;
let localDeviceGeneration = 0;
let deviceLinkCancellation = Promise.resolve();
let lastDeviceLinkStatus = null;

const defaultServerUrl = process.env.FINITECHAT_SERVER_URL || "https://chat.finite.computer";
const defaultDashboardUrl = normalizeDashboardBaseUrl(
  process.env.FINITECHAT_DASHBOARD_URL || "https://finite.computer"
);
const dashboardStartUrl = dashboardDestination(
  defaultDashboardUrl,
  process.env.FINITECHAT_DASHBOARD_PATH || "/dashboard"
);
if (!isDashboardDocumentUrl(dashboardStartUrl, defaultDashboardUrl)) {
  throw new Error("FINITECHAT_DASHBOARD_PATH must stay inside /dashboard");
}
const dashboardPartition = "persist:finite-dashboard-v1";
const deviceLinkPollIntervalMs = 750;
const maxDashboardResponseBytes = 64 * 1024;

if (process.env.FINITECHAT_USER_DATA_DIR) {
  app.setPath("userData", process.env.FINITECHAT_USER_DATA_DIR);
}

protocol.registerSchemesAsPrivileged([
  {
    scheme: attachmentMediaScheme,
    privileges: {
      standard: true,
      secure: true,
      supportFetchAPI: true,
      corsEnabled: true,
    },
  },
]);

function commonWindowOptions() {
  return {
    width: 1280,
    height: 860,
    minWidth: 900,
    minHeight: 640,
    backgroundColor: "#f6f7f9",
    show: false,
    title: "Finite",
  };
}

function showWindowWhenReady(window) {
  window.once("ready-to-show", () => {
    if (!window.isDestroyed()) {
      window.show();
      window.focus();
    }
  });
}

function hasTrustedDashboardWindow() {
  return Boolean(
    mainWindow
    && !mainWindow.isDestroyed()
    && !mainWindow.webContents.isDestroyed()
    && isDashboardDocumentUrl(mainWindow.webContents.getURL(), defaultDashboardUrl)
  );
}

function downloadDashboardAttachment(url) {
  try {
    parseAttachmentMediaUrl(url);
  } catch {
    return false;
  }
  if (!hasTrustedDashboardWindow()) {
    return false;
  }
  mainWindow.webContents.downloadURL(url);
  return true;
}

function createDashboardWindow(targetUrl = dashboardStartUrl) {
  if (!isDashboardDocumentUrl(targetUrl, defaultDashboardUrl)) {
    throw new Error("Finite dashboard tried to open an untrusted privileged document");
  }
  if (mainWindow && !mainWindow.isDestroyed()) {
    void mainWindow.loadURL(targetUrl);
    mainWindow.show();
    mainWindow.focus();
    return mainWindow;
  }

  mainWindow = new BrowserWindow({
    ...commonWindowOptions(),
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      partition: dashboardPartition,
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webSecurity: true,
    },
  });

  showWindowWhenReady(mainWindow);
  mainWindow.on("closed", () => {
    mainWindow = null;
  });
  const transitionToAuth = (url) => {
    const window = mainWindow;
    if (!window || window.isDestroyed()) return;
    mainWindow = null;
    window.destroy();
    createAuthWindow(url);
  };
  mainWindow.webContents.on("will-navigate", (event) => {
    const url = event.url;
    if (isDashboardDocumentUrl(url, defaultDashboardUrl)) return;
    event.preventDefault();
    if (downloadDashboardAttachment(url)) {
      return;
    } else if (isGoogleWorkspaceStartUrl(url, defaultDashboardUrl)) {
      void shell.openExternal(url);
    } else if (isDashboardOriginUrl(url, defaultDashboardUrl)) {
      transitionToAuth(url);
    } else if (url.startsWith("https://")) {
      void shell.openExternal(url);
    }
  });
  mainWindow.webContents.on("will-redirect", (event) => {
    if (!event.isMainFrame) return;
    const url = event.url;
    if (isDashboardDocumentUrl(url, defaultDashboardUrl)) return;
    event.preventDefault();
    if (isAllowedUnprivilegedNavigation(url, defaultDashboardUrl)) {
      transitionToAuth(url);
    }
  });
  mainWindow.webContents.on("did-navigate-in-page", (_event, url, isMainFrame) => {
    if (!isMainFrame || isDashboardDocumentUrl(url, defaultDashboardUrl)) return;
    if (isAllowedUnprivilegedNavigation(url, defaultDashboardUrl)) {
      transitionToAuth(url);
    }
  });
  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (downloadDashboardAttachment(url)) {
      // Custom-scheme content stays out of a new privileged renderer.
    } else if (isDashboardDocumentUrl(url, defaultDashboardUrl)) {
      void mainWindow?.loadURL(url);
    } else if (isGoogleWorkspaceStartUrl(url, defaultDashboardUrl)) {
      void shell.openExternal(url);
    } else if (isDashboardOriginUrl(url, defaultDashboardUrl)) {
      transitionToAuth(url);
    } else if (url.startsWith("https://")) {
      void shell.openExternal(url);
    }
    return { action: "deny" };
  });
  void mainWindow.loadURL(targetUrl);

  mainWindow.webContents.once("did-finish-load", () => {
    maybeCaptureWindow();
  });
  return mainWindow;
}

function createAuthWindow(targetUrl = dashboardStartUrl) {
  if (!isAllowedUnprivilegedNavigation(targetUrl, defaultDashboardUrl)) {
    throw new Error("Finite sign-in tried to open an invalid address");
  }
  invalidateLocalDeviceVerification({ cancelLink: true });
  if (authWindow && !authWindow.isDestroyed()) {
    void authWindow.loadURL(targetUrl);
    authWindow.show();
    authWindow.focus();
    return authWindow;
  }

  authWindow = new BrowserWindow({
    ...commonWindowOptions(),
    webPreferences: {
      partition: dashboardPartition,
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webSecurity: true,
    },
  });
  showWindowWhenReady(authWindow);
  authWindow.on("closed", () => {
    authWindow = null;
  });
  const enterDashboardIfReady = (url) => {
    if (!isDashboardDocumentUrl(url, defaultDashboardUrl)) return;
    const window = authWindow;
    if (!window || window.isDestroyed()) return;
    authWindow = null;
    window.destroy();
    createDashboardWindow(url);
  };
  authWindow.webContents.on("did-navigate", (_event, url) => enterDashboardIfReady(url));
  authWindow.webContents.on("did-navigate-in-page", (_event, url, isMainFrame) => {
    if (isMainFrame) enterDashboardIfReady(url);
  });
  authWindow.webContents.on(
    "did-redirect-navigation",
    (event) => {
      if (event.isMainFrame) enterDashboardIfReady(event.url);
    }
  );
  authWindow.webContents.on("will-navigate", (event) => {
    if (!isAllowedUnprivilegedNavigation(event.url, defaultDashboardUrl)) {
      event.preventDefault();
    }
  });
  authWindow.webContents.on("will-redirect", (event) => {
    if (!isAllowedUnprivilegedNavigation(event.url, defaultDashboardUrl)) {
      event.preventDefault();
    }
  });
  authWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (isAllowedUnprivilegedNavigation(url, defaultDashboardUrl)) {
      void authWindow?.loadURL(url);
    }
    return { action: "deny" };
  });
  void authWindow.loadURL(targetUrl);
  return authWindow;
}

function repoRoot() {
  return path.resolve(__dirname, "../../../..");
}

function registerAttachmentMediaProtocol() {
  dashboardSession.protocol.handle(attachmentMediaScheme, async (request) => {
    if (request.method !== "GET") {
      return new Response("Method not allowed", { status: 405 });
    }
    if (
      !mainWindow
      || mainWindow.isDestroyed()
      || !isDashboardDocumentUrl(mainWindow.webContents.getURL(), defaultDashboardUrl)
    ) {
      return new Response("Forbidden", { status: 403 });
    }
    const referrer = request.headers.get("referer");
    if (referrer && !isDashboardOriginUrl(referrer, defaultDashboardUrl)) {
      return new Response("Forbidden", { status: 403 });
    }
    let coordinates;
    try {
      coordinates = parseAttachmentMediaUrl(request.url);
    } catch {
      return new Response("Not found", { status: 404 });
    }
    if (!daemonSupervisor) {
      return new Response("Local service unavailable", { status: 503 });
    }
    try {
      const requestGeneration = updateGeneration;
      const connection = daemonSupervisor.connectionForUpdateStream();
      const path = [coordinates.room_id, coordinates.message_id, coordinates.attachment_id]
        .map(encodeURIComponent)
        .join("/");
      const response = await fetch(`${connection.url}/v1/app/attachments/${path}`, {
        headers: { authorization: `Bearer ${connection.authToken}` },
        signal: updateAbortController?.signal,
      });
      if (!daemonRequestIsCurrent(requestGeneration, connection)) {
        return new Response("Local service restarted", { status: 409 });
      }
      const headers = new Headers();
      for (const name of [
        "content-type",
        "content-disposition",
        "content-length",
        "cache-control",
        "x-content-type-options",
        "content-security-policy",
      ]) {
        const value = response.headers.get(name);
        if (value) {
          headers.set(name, value);
        }
      }
      return new Response(response.body, { status: response.status, headers });
    } catch {
      return new Response("Local service unavailable", { status: 503 });
    }
  });
}

function configureSessionSecurity() {
  dashboardSession = session.fromPartition(dashboardPartition);
  dashboardSession.setPermissionRequestHandler((_webContents, _permission, callback) => {
    callback(false);
  });
  dashboardSession.setPermissionCheckHandler(() => false);
  dashboardSession.setDevicePermissionHandler(() => false);
  dashboardSession.webRequest.onBeforeRequest((details, callback) => {
    if (details.resourceType === "mainFrame") {
      const allowed = isAllowedUnprivilegedNavigation(details.url, defaultDashboardUrl);
      callback({ cancel: !allowed });
      return;
    }
    callback({});
  });
}

async function maybeCaptureWindow() {
  const capturePath = process.env.FINITECHAT_CAPTURE_PATH;
  if (!capturePath || !mainWindow) {
    return;
  }
  setTimeout(async () => {
    if (!mainWindow) {
      return;
    }
    const image = await mainWindow.webContents.capturePage();
    fs.mkdirSync(path.dirname(capturePath), { recursive: true });
    fs.writeFileSync(capturePath, image.toPNG());
    console.log(`[finitechat-electron] captured ${capturePath}`);
    if (process.env.FINITECHAT_EXIT_AFTER_CAPTURE === "1") {
      app.quit();
    }
  }, 2200);
}

function daemonDataDir() {
  return process.env.FINITECHAT_HOME || path.join(app.getPath("userData"), "finitechatd");
}

function identitySecretPath() {
  return path.join(app.getPath("userData"), "account-secret.safe");
}

function identityStore() {
  if (!accountSecretStore) {
    accountSecretStore = new AccountSecretStore({
      secretPath: identitySecretPath(),
      safeStorage,
    });
  }
  return accountSecretStore;
}

function settingsPath() {
  return path.join(app.getPath("userData"), "desktop-settings.json");
}

function readDesktopSettings() {
  try {
    return JSON.parse(fs.readFileSync(settingsPath(), "utf8"));
  } catch (error) {
    if (error?.code !== "ENOENT") {
      console.error(`failed to read Finite Chat desktop settings: ${error.message}`);
    }
    return {};
  }
}

function writeDesktopSettings(settings) {
  fs.mkdirSync(path.dirname(settingsPath()), { recursive: true });
  const temporaryPath = `${settingsPath()}.${process.pid}.tmp`;
  fs.writeFileSync(temporaryPath, `${JSON.stringify(settings, null, 2)}\n`, { mode: 0o600 });
  fs.renameSync(temporaryPath, settingsPath());
}

function pendingDeviceLink() {
  const candidate = readDesktopSettings().pendingDeviceLink;
  if (!candidate) return null;
  try {
    return parseDeviceLinkPublicRequest(candidate);
  } catch {
    throw new Error("Finite Chat desktop settings contain an invalid pending Device link");
  }
}

function storePendingDeviceLink(value) {
  const settings = readDesktopSettings();
  settings.pendingDeviceLink = parseDeviceLinkPublicRequest(value);
  writeDesktopSettings(settings);
}

function clearPendingDeviceLink(expected) {
  const settings = readDesktopSettings();
  if (!settings.pendingDeviceLink) return;
  const current = parseDeviceLinkPublicRequest(settings.pendingDeviceLink);
  const requested = parseDeviceLinkPublicRequest(expected);
  if (
    current.link_session_id !== requested.link_session_id
    || current.target_device_id !== requested.target_device_id
  ) {
    return;
  }
  delete settings.pendingDeviceLink;
  writeDesktopSettings(settings);
}

function secureStorageAvailable() {
  return identityStore().isAvailable();
}

function readStoredAccountSecret() {
  if (!secureStorageAvailable()) {
    return null;
  }
  try {
    return identityStore().read();
  } catch (error) {
    console.error(`failed to read stored Finite identity: ${error.message}`);
    return null;
  }
}

function writeProvisionalAccountSecret(secret) {
  identityStore().writeProvisional(secret);
}

function promoteProvisionalAccountSecret() {
  identityStore().promoteProvisional();
}

function discardProvisionalAccountSecret() {
  try {
    identityStore().discardProvisional();
  } catch (error) {
    console.error(`failed to discard provisional Finite identity: ${error.message}`);
  }
}

function daemonDeviceId() {
  if (process.env.FINITECHAT_DEVICE_ID) {
    if (!validDeviceId(process.env.FINITECHAT_DEVICE_ID)) {
      throw new Error("FINITECHAT_DEVICE_ID is invalid");
    }
    return process.env.FINITECHAT_DEVICE_ID;
  }
  return loadOrCreateDeviceId({
    settingsFile: settingsPath(),
    daemonDataDirectory: daemonDataDir(),
    hostname: os.hostname(),
  });
}

function broadcastToRenderer(channel, value) {
  if (hasTrustedDashboardWindow()) {
    mainWindow.webContents.send(channel, value);
  }
}

function updateRelay() {
  if (!daemonUpdateRelay) {
    daemonUpdateRelay = new DaemonUpdateRelay(broadcastToRenderer);
  }
  return daemonUpdateRelay;
}

function rendererDaemonError() {
  return daemonFailure || "Finite Chat's local service is unavailable";
}

function daemonRequestIsCurrent(requestGeneration, requestConnection) {
  if (requestGeneration !== updateGeneration || !daemonSupervisor) {
    return false;
  }
  try {
    const current = daemonSupervisor.connectionForUpdateStream();
    return daemonRequestVersionMatches(requestGeneration, requestConnection, updateGeneration, current);
  } catch {
    return false;
  }
}

function assertCurrentDaemonRequest(requestGeneration, requestConnection) {
  if (!daemonRequestIsCurrent(requestGeneration, requestConnection)) {
    throw new Error("Finite Chat request was interrupted by a local service restart");
  }
}

function recordDaemonFailure(error) {
  daemonFailure = "Finite Chat's local service could not start. Restart Finite Chat to try again.";
  console.error(`[finitechat-electron] local daemon failure: ${error?.message || "unknown failure"}`);
  broadcastToRenderer("finitechat:daemon-error", daemonFailure);
}

function daemonBinaryPath() {
  return resolveDaemonBinary({
    explicitPath: process.env.FINITECHAT_DAEMON_BINARY,
    isPackaged: app.isPackaged,
    resourcesPath: process.resourcesPath,
    cwd: repoRoot(),
  });
}

function createDaemonSupervisor(accountSecret) {
  const binaryPath = daemonBinaryPath();
  return new DaemonSupervisor({
    spawnProcess: spawn,
    binaryPath,
    args: [
      "--bind",
      "127.0.0.1:0",
      "--data-dir",
      daemonDataDir(),
      "--server-url",
      defaultServerUrl,
      "--device-id",
      daemonDeviceId(),
    ],
    cwd: app.isPackaged ? path.dirname(binaryPath) : repoRoot(),
    accountSecret,
    onUnexpectedExit: (error) => {
      const reconnect = Boolean(verifiedLocalDevice);
      invalidateLocalDeviceVerification();
      const reconnectGeneration = localDeviceGeneration;
      stopUpdatePump();
      recordDaemonFailure(error);
      if (reconnect && !quitAfterDaemonStops) {
        setTimeout(() => {
          if (
            reconnectGeneration === localDeviceGeneration
            && hasTrustedDashboardWindow()
            && !quitAfterDaemonStops
          ) {
            void ensureLocalDevice().catch(() => {});
          }
        }, 250);
      }
    },
  });
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function stateFromSseBlock(block) {
  let eventName = "message";
  const dataLines = [];
  for (const line of block.split(/\r?\n/)) {
    if (line.startsWith("event:")) {
      eventName = line.slice("event:".length).trimStart();
    } else if (line.startsWith("data:")) {
      dataLines.push(line.slice("data:".length).trimStart());
    }
  }
  if (eventName !== "state" || dataLines.length === 0) {
    return null;
  }
  const state = JSON.parse(dataLines.join("\n"));
  return state && typeof state === "object" && !Array.isArray(state) ? state : null;
}

async function runUpdatePump(generation, signal) {
  let reportedDisconnect = false;
  while (generation === updateGeneration && !signal.aborted) {
    try {
      const connection = daemonSupervisor.connectionForUpdateStream();
      const response = await fetch(`${connection.url}/v1/app/updates`, {
        headers: { authorization: `Bearer ${connection.authToken}` },
        signal,
      });
      if (!response.ok || !response.body) {
        throw new Error("update stream rejected");
      }
      const decoder = new TextDecoder();
      let buffered = "";
      for await (const chunk of response.body) {
        if (generation !== updateGeneration || signal.aborted) {
          return;
        }
        buffered += decoder.decode(chunk, { stream: true });
        while (true) {
          const boundary = /\r?\n\r?\n/.exec(buffered);
          if (!boundary) {
            break;
          }
          const block = buffered.slice(0, boundary.index);
          buffered = buffered.slice(boundary.index + boundary[0].length);
          const state = stateFromSseBlock(block);
          if (state) {
            reportedDisconnect = false;
            updateRelay().update(state);
          }
        }
        if (buffered.length > 16 * 1024 * 1024) {
          throw new Error("update stream frame is too large");
        }
      }
      if (!signal.aborted) {
        throw new Error("update stream ended");
      }
    } catch {
      if (generation !== updateGeneration || signal.aborted) {
        return;
      }
      if (!reportedDisconnect) {
        broadcastToRenderer("finitechat:daemon-error", "Finite Chat's local update stream disconnected; reconnecting…");
        reportedDisconnect = true;
      }
      await delay(250);
    }
  }
}

function startUpdatePump() {
  stopUpdatePump();
  updateRelay().beginGeneration();
  updateAbortController = new AbortController();
  const generation = updateGeneration;
  void runUpdatePump(generation, updateAbortController.signal);
}

function stopUpdatePump() {
  updateGeneration += 1;
  if (updateAbortController) {
    updateAbortController.abort();
    updateAbortController = null;
  }
}

function enqueueDaemonLifecycle(operation) {
  const result = daemonLifecycle.then(operation, operation);
  daemonLifecycle = result.catch(() => {});
  return result;
}

async function startDaemonNow() {
  const accountSecret = readStoredAccountSecret();
  if (!accountSecret) {
    daemonFailure = null;
    return;
  }
  try {
    if (!daemonSupervisor) {
      daemonSupervisor = createDaemonSupervisor(accountSecret);
    }
    await startDaemonProcessRuntime(() => daemonSupervisor.start());
  } catch (error) {
    stopUpdatePump();
    recordDaemonFailure(error);
    throw error;
  }
}

function startDaemon() {
  return enqueueDaemonLifecycle(startDaemonNow);
}

function startDaemonProcessRuntime(startDaemonProcess) {
  return startDaemonRuntime({
    startDaemonProcess: async () => {
      const started = await startDaemonProcess();
      if (started) {
        daemonFailure = null;
        startUpdatePump();
      }
      return started;
    },
    dispatchDaemonAction: dispatchInternalDaemonAction,
  });
}

function stopDaemon() {
  return enqueueDaemonLifecycle(async () => {
    stopUpdatePump();
    if (activeDeviceLink) {
      const link = activeDeviceLink;
      activeDeviceLink = null;
      await link.cancel();
      discardProvisionalAccountSecret();
    }
    if (daemonSupervisor) {
      await daemonSupervisor.stop();
    }
  });
}

async function requestDaemonState() {
  if (!daemonSupervisor) {
    throw new Error(rendererDaemonError());
  }
  try {
    const requestGeneration = updateGeneration;
    const requestConnection = daemonSupervisor.connectionForUpdateStream();
    const state = await daemonSupervisor.requestJson("/v1/app/state");
    assertCurrentDaemonRequest(requestGeneration, requestConnection);
    updateRelay().update(state);
    return state;
  } catch (error) {
    if (daemonFailure) {
      throw new Error(rendererDaemonError());
    }
    throw error;
  }
}

async function requestDaemonMutation(pathname, body) {
  if (!daemonSupervisor) {
    throw new Error(rendererDaemonError());
  }
  try {
    const requestGeneration = updateGeneration;
    const requestConnection = daemonSupervisor.connectionForUpdateStream();
    const state = await daemonSupervisor.requestJson(pathname, {
      method: "POST",
      body,
    });
    assertCurrentDaemonRequest(requestGeneration, requestConnection);
    updateRelay().update(state);
    return state;
  } catch (error) {
    if (daemonFailure) {
      throw new Error(rendererDaemonError());
    }
    throw error;
  }
}

function dispatchInternalDaemonAction(action) {
  const body = JSON.stringify(action);
  if (typeof body !== "string" || Buffer.byteLength(body) > MAX_DESKTOP_ACTION_BYTES) {
    throw new Error("Finite Chat internal action is invalid");
  }
  return requestDaemonMutation("/v1/app/actions", body);
}

function dispatchDaemonAction(action) {
  const validated = assertDesktopChatAction(action);
  if (attachmentActionUsesBinaryTransport(action)) {
    throw new Error("Finite Chat attachments must use the binary upload transport");
  }
  return validated.operation === "StartTopicChatIntent"
    ? requestDaemonMutation("/v1/app/new-chat", JSON.stringify(action.StartTopicChatIntent))
    : requestDaemonMutation("/v1/app/actions", validated.encoded);
}

async function uploadDaemonAttachments(upload) {
  if (!daemonSupervisor) {
    throw new Error(rendererDaemonError());
  }
  return forwardAttachmentUpload(upload, async (form) => {
    const requestGeneration = updateGeneration;
    const requestSignal = updateAbortController?.signal;
    try {
      const connection = daemonSupervisor.connectionForUpdateStream();
      const response = await fetch(`${connection.url}/v1/app/attachments`, {
        method: "POST",
        headers: { authorization: `Bearer ${connection.authToken}` },
        body: form,
        signal: requestSignal,
      });
      if (!response.ok) {
        const body = await response.text();
        let message = `Finite Chat attachment upload failed (${response.status})`;
        try {
          const parsed = JSON.parse(body);
          if (typeof parsed?.error === "string" && parsed.error) {
            message = parsed.error;
          }
        } catch {
          // Never reflect an arbitrary local HTTP response into the renderer.
        }
        throw new Error(message);
      }
      const state = await response.json();
      if (!daemonRequestIsCurrent(requestGeneration, connection)) {
        throw new Error("Finite Chat attachment upload was interrupted by a local service restart");
      }
      updateRelay().update(state);
      return state;
    } catch (error) {
      if (daemonFailure) {
        throw new Error(rendererDaemonError());
      }
      if (error?.name === "AbortError") {
        throw new Error("Finite Chat attachment upload was interrupted by a local service restart");
      }
      throw error;
    }
  });
}

function deviceLinkStatus(status) {
  lastDeviceLinkStatus = status;
  broadcastToRenderer("finitechat:device-link-status", status);
}

async function dashboardJson(pathname, init = {}) {
  if (!dashboardSession) {
    throw new Error("Finite dashboard session is unavailable");
  }
  const url = new URL(pathname, `${defaultDashboardUrl}/`);
  if (url.origin !== defaultDashboardUrl || !url.pathname.startsWith("/api/")) {
    throw new Error("Finite dashboard request address is invalid");
  }
  const headers = new Headers(init.headers);
  headers.set("accept", "application/json");
  if (typeof init.body === "string") {
    headers.set("content-type", "application/json");
  }
  let response;
  try {
    response = await dashboardSession.fetch(url.toString(), {
      ...init,
      cache: "no-store",
      credentials: "include",
      headers,
      signal: AbortSignal.timeout(15_000),
    });
  } catch {
    throw new Error("Finite dashboard is unavailable right now");
  }
  if (!response.ok) {
    if (response.status === 401 || response.status === 403) {
      throw new Error("Sign in again to prepare local chat on this desktop");
    }
    throw new Error("Finite dashboard could not prepare local chat right now");
  }
  const contentType = response.headers.get("content-type") ?? "";
  if (!contentType.toLowerCase().startsWith("application/json")) {
    throw new Error("Finite dashboard returned an invalid local-chat response");
  }
  if (!response.body) {
    throw new Error("Finite dashboard returned an invalid local-chat response");
  }
  const reader = response.body.getReader();
  const chunks = [];
  let totalBytes = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    totalBytes += value.byteLength;
    if (totalBytes > maxDashboardResponseBytes) {
      await reader.cancel().catch(() => {});
      throw new Error("Finite dashboard returned an invalid local-chat response");
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  try {
    return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  } catch {
    throw new Error("Finite dashboard returned an invalid local-chat response");
  }
}

async function currentDashboardAccountBinding(deviceId = null) {
  const suffix = deviceId === null
    ? ""
    : `?target_device_id=${encodeURIComponent(deviceId)}`;
  return parseAccountBinding(
    await dashboardJson(`/api/device-links/account-binding${suffix}`),
    deviceId
  );
}

async function dashboardDeviceLinkRequest(pathname, request) {
  const expected = parseDeviceLinkPublicRequest(request);
  const value = await dashboardJson(pathname, {
    method: "POST",
    body: JSON.stringify(expected),
  });
  return parseDeviceLinkPublicResponse(value, expected);
}

async function waitForDeviceLinkReady(request, initial = null, generation = null) {
  const expected = parseDeviceLinkPublicRequest(request);
  let current = initial ? parseDeviceLinkPublicResponse(initial, expected) : null;
  while (true) {
    if (generation !== null) assertLocalDeviceGeneration(generation);
    if (!current) {
      current = await dashboardDeviceLinkRequest("/api/device-links/status", expected);
      if (generation !== null) assertLocalDeviceGeneration(generation);
    }
    if (current.status === "ready") {
      return current;
    }
    if (
      current.status === "expired"
      || Date.now() >= current.expires_at_unix_seconds * 1_000
    ) {
      throw new Error("This desktop's automatic Device setup expired. Restart Finite and try again.");
    }
    deviceLinkStatus({
      status: current.status === "joining_rooms" ? "joining_rooms" : "linking",
    });
    await delay(deviceLinkPollIntervalMs);
    if (generation !== null) assertLocalDeviceGeneration(generation);
    current = null;
  }
}

async function createAndApproveDeviceLink(generation) {
  assertLocalDeviceGeneration(generation);
  if (!secureStorageAvailable()) {
    throw new Error("Secure storage is required to prepare local chat on this desktop");
  }
  const binaryPath = daemonBinaryPath();
  const link = new DeviceLinkSupervisor({
    spawnProcess: spawn,
    binaryPath,
    serverUrl: defaultServerUrl,
    dashboardUrl: defaultDashboardUrl,
    deviceId: daemonDeviceId(),
    cwd: app.isPackaged ? path.dirname(binaryPath) : repoRoot(),
    storeAccountSecret: async (accountSecret) => writeProvisionalAccountSecret(accountSecret),
    promoteAccountSecret: async () => promoteProvisionalAccountSecret(),
  });
  activeDeviceLink = link;
  try {
    assertLocalDeviceGeneration(generation);
    deviceLinkStatus({ status: "linking" });
    const ready = await link.begin();
    assertLocalDeviceGeneration(generation);
    if (activeDeviceLink !== link) {
      throw new Error("Finite Chat Device setup was cancelled");
    }
    const request = parseDeviceLinkPublicRequest(ready);
    storePendingDeviceLink(request);
    const approved = await dashboardDeviceLinkRequest("/api/device-links/approve", request);
    assertLocalDeviceGeneration(generation);
    await link.completion;
    assertLocalDeviceGeneration(generation);
    return { request, approved };
  } catch (error) {
    if (activeDeviceLink === link) {
      await link.cancel().catch(() => {});
    }
    discardProvisionalAccountSecret();
    throw new Error(deviceLinkFailureMessage(error, error?.message || "This desktop could not be linked"));
  } finally {
    if (activeDeviceLink === link) {
      activeDeviceLink = null;
    }
  }
}

function assertLocalDeviceGeneration(generation) {
  if (generation !== localDeviceGeneration) {
    throw new Error("Finite dashboard session changed while local chat was being prepared");
  }
}

function invalidateLocalDeviceVerification({ cancelLink = false } = {}) {
  localDeviceGeneration += 1;
  verifiedLocalDevice = null;
  if (cancelLink) {
    lastDeviceLinkStatus = null;
    deviceLinkCancellation = deviceLinkCancellation
      .then(() => cancelDeviceLink())
      .catch(() => {});
  }
}

async function ensureLocalDeviceNow(generation) {
  await deviceLinkCancellation;
  assertLocalDeviceGeneration(generation);
  deviceLinkStatus({ status: "preparing" });
  const deviceId = daemonDeviceId();
  const binding = await currentDashboardAccountBinding(deviceId);
  assertLocalDeviceGeneration(generation);
  if (binding.local_device.status === "revoked") {
    const recovery = {
      status: "recovery_required",
      reason: "device_revoked",
      device_id: deviceId,
      message:
        "This desktop Device was revoked. Relink this Mac to create a fresh Device. Your existing encrypted local store will be kept as a backup.",
    };
    deviceLinkStatus(recovery);
    return recovery;
  }
  let pending = pendingDeviceLink();
  let initial = null;
  if (!readStoredAccountSecret()) {
    const linked = await createAndApproveDeviceLink(generation);
    pending = linked.request;
    initial = linked.approved;
  }
  assertLocalDeviceGeneration(generation);
  await startDaemon();
  assertLocalDeviceGeneration(generation);
  if (pending) {
    await waitForDeviceLinkReady(pending, initial, generation);
    clearPendingDeviceLink(pending);
  }
  const state = await requestDaemonState();
  assertLocalDeviceGeneration(generation);
  const identity = parseLocalDaemonIdentity(state, binding.account_id);
  deviceLinkStatus({ status: "ready" });
  return { status: "ready", ...identity };
}

function ensureLocalDevice() {
  if (verifiedLocalDevice) {
    return Promise.resolve(verifiedLocalDevice);
  }
  const generation = localDeviceGeneration;
  if (!localDeviceLifecycle || localDeviceLifecycle.generation !== generation) {
    const promise = ensureLocalDeviceNow(generation)
      .then((device) => {
        assertLocalDeviceGeneration(generation);
        verifiedLocalDevice = device;
        return device;
      })
      .catch((error) => {
        if (generation === localDeviceGeneration) {
          deviceLinkStatus({ status: "failed", message: error.message });
        }
        throw error;
      })
      .finally(() => {
        if (localDeviceLifecycle?.promise === promise) {
          localDeviceLifecycle = null;
        }
      });
    localDeviceLifecycle = { generation, promise };
  }
  return localDeviceLifecycle.promise;
}

async function cancelDeviceLink() {
  const link = activeDeviceLink;
  if (!link) {
    return;
  }
  activeDeviceLink = null;
  await link.cancel();
  discardProvisionalAccountSecret();
}

async function recoverRevokedLocalDevice() {
  if (process.env.FINITECHAT_DEVICE_ID || process.env.FINITECHAT_HOME) {
    throw new Error("Revoked Device recovery is unavailable for an explicitly configured development profile");
  }
  const deviceId = daemonDeviceId();
  const binding = await currentDashboardAccountBinding(deviceId);
  if (binding.local_device.status !== "revoked") {
    throw new Error("This desktop Device is no longer marked as revoked");
  }

  await stopDaemon();
  daemonSupervisor = null;
  archiveRevokedDeviceProfile({
    userDataDirectory: app.getPath("userData"),
    daemonDataDirectory: daemonDataDir(),
    settingsFile: settingsPath(),
    secretFile: identitySecretPath(),
    deviceId,
  });
  daemonFailure = null;
  invalidateLocalDeviceVerification({ cancelLink: true });
  return ensureLocalDevice();
}

function focusAppWindow() {
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.focus();
  } else if (authWindow && !authWindow.isDestroyed()) {
    authWindow.focus();
  } else if (app.isReady()) {
    createAuthWindow();
  }
}

const useSingleInstanceLock = process.env.FINITECHAT_DISABLE_SINGLE_INSTANCE_LOCK !== "1";
const gotLock = useSingleInstanceLock ? app.requestSingleInstanceLock() : true;
if (!gotLock) {
  app.quit();
} else {
  if (useSingleInstanceLock) {
    app.on("second-instance", () => focusAppWindow());
  }

  // Older alpha bundles registered finite://. Do not interpret those invite
  // payloads in the remote-dashboard shell; only bring the app forward.
  app.on("open-url", (event) => {
    event.preventDefault();
    focusAppWindow();
  });

  app.whenReady().then(() => {
    discardProvisionalAccountSecret();
    configureSessionSecurity();
    registerAttachmentMediaProtocol();
    createAuthWindow();
  });

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createAuthWindow();
    }
  });

  app.on("window-all-closed", () => {
    if (process.platform !== "darwin") {
      app.quit();
    }
  });

  app.on("before-quit", (event) => {
    if (quitAfterDaemonStops) {
      return;
    }
    event.preventDefault();
    if (!daemonShutdownPromise) {
      daemonShutdownPromise = stopDaemon().finally(() => {
        quitAfterDaemonStops = true;
        app.quit();
      });
    }
  });
}

function assertTrustedIpcSender(event) {
  const frame = event.senderFrame;
  if (
    !mainWindow
    || mainWindow.isDestroyed()
    || event.sender !== mainWindow.webContents
    || !frame
    || !trustedDashboardIpcFrame(
      {
        frameUrl: frame.url,
        isMainFrame: frame === mainWindow.webContents.mainFrame,
      },
      defaultDashboardUrl
    )
  ) {
    throw new Error("Finite Chat desktop request came from an untrusted renderer");
  }
}

ipcMain.handle("finitechat:ensure-local-device", async (event) => {
  assertTrustedIpcSender(event);
  return ensureLocalDevice();
});

ipcMain.handle("finitechat:recover-local-device", async (event) => {
  assertTrustedIpcSender(event);
  return recoverRevokedLocalDevice();
});

ipcMain.handle("finitechat:daemon-state", async (event) => {
  assertTrustedIpcSender(event);
  await ensureLocalDevice();
  return requestDaemonState();
});

ipcMain.handle("finitechat:daemon-action", async (event, action) => {
  assertTrustedIpcSender(event);
  await ensureLocalDevice();
  return dispatchDaemonAction(action);
});

ipcMain.handle("finitechat:daemon-attachments", async (event, upload) => {
  assertTrustedIpcSender(event);
  await ensureLocalDevice();
  return uploadDaemonAttachments(upload);
});

ipcMain.handle("finitechat:daemon-subscribe", async (event) => {
  assertTrustedIpcSender(event);
  await ensureLocalDevice();
  updateRelay().replay((channel, value) => event.sender.send(channel, value));
  return true;
});

ipcMain.handle("finitechat:device-link-subscribe", (event) => {
  assertTrustedIpcSender(event);
  if (lastDeviceLinkStatus) {
    event.sender.send("finitechat:device-link-status", lastDeviceLinkStatus);
  }
  return true;
});
