const path = require("node:path");
const fs = require("node:fs");
const os = require("node:os");
const { spawn } = require("node:child_process");
const { pathToFileURL } = require("node:url");
const { app, BrowserWindow, clipboard, ipcMain, net, protocol, safeStorage, session, shell } = require("electron");
const {
  attachmentActionUsesBinaryTransport,
  forwardAttachmentUpload,
} = require("./attachment-upload.cjs");
const {
  attachmentMediaScheme,
  parseAttachmentMediaUrl,
} = require("./attachment-media.cjs");
const {
  AccountSecretStore,
  DaemonUpdateRelay,
  DaemonSupervisor,
  DeviceLinkSupervisor,
  daemonRequestVersionMatches,
  deviceLinkFailureMessage,
  loadOrCreateDeviceId,
  resolveDaemonBinary,
  startDaemonRuntime,
  validDeviceId,
} = require("./daemon-process.cjs");

let mainWindow = null;
let pendingTargetUrl = null;
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

const rendererUrl = process.env.FINITECHAT_RENDERER_URL;
const defaultServerUrl = process.env.FINITECHAT_SERVER_URL || "https://chat.finite.computer";
const defaultDashboardUrl = process.env.FINITECHAT_DASHBOARD_URL || "https://finite.computer";
const appProtocol = "finitechat-app";
const desktopIpcConnection = "finitechat-desktop-ipc";
const maxIpcActionBytes = 16 * 1024 * 1024;

if (process.env.FINITECHAT_USER_DATA_DIR) {
  app.setPath("userData", process.env.FINITECHAT_USER_DATA_DIR);
}

protocol.registerSchemesAsPrivileged([
  {
    scheme: appProtocol,
    privileges: {
      standard: true,
      secure: true,
      supportFetchAPI: true,
      corsEnabled: true,
    },
  },
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

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1280,
    height: 860,
    minWidth: 900,
    minHeight: 640,
    backgroundColor: "#f6f7f9",
    show: false,
    title: "Finite Chat",
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  });

  mainWindow.once("ready-to-show", () => {
    mainWindow.show();
    mainWindow.focus();
  });
  mainWindow.on("closed", () => {
    mainWindow = null;
  });
  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith("https://")) {
      shell.openExternal(url);
    }
    return { action: "deny" };
  });
  if (rendererUrl) {
    mainWindow.loadURL(rendererUrl);
  } else {
    mainWindow.loadURL(`${appProtocol}://app/index.html`);
  }

  mainWindow.webContents.once("did-finish-load", () => {
    if (pendingTargetUrl) {
      mainWindow.webContents.send("finitechat:target-url", pendingTargetUrl);
    }
    maybeCaptureWindow();
  });
}

function repoRoot() {
  return path.resolve(__dirname, "../../..");
}

function rendererRoot() {
  return path.resolve(__dirname, "../dist");
}

function registerAppProtocol() {
  protocol.handle(appProtocol, (request) => {
    const url = new URL(request.url);
    const pathname = decodeURIComponent(url.pathname === "/" ? "/index.html" : url.pathname);
    const root = rendererRoot();
    const filePath = path.normalize(path.join(root, pathname));
    if (filePath !== root && !filePath.startsWith(`${root}${path.sep}`)) {
      return new Response("Not found", { status: 404 });
    }
    return net.fetch(pathToFileURL(filePath).toString());
  });
}

function registerAttachmentMediaProtocol() {
  protocol.handle(attachmentMediaScheme, async (request) => {
    if (request.method !== "GET") {
      return new Response("Method not allowed", { status: 405 });
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

function isAllowedNavigation(navigationUrl) {
  const parsed = new URL(navigationUrl);
  if (parsed.protocol === `${appProtocol}:`) {
    return true;
  }
  if (parsed.protocol === `${attachmentMediaScheme}:`) {
    try {
      parseAttachmentMediaUrl(navigationUrl);
      return true;
    } catch {
      return false;
    }
  }
  if (rendererUrl && parsed.origin === new URL(rendererUrl).origin) {
    return true;
  }
  return false;
}

function configureSessionSecurity() {
  session.defaultSession.setPermissionRequestHandler((_webContents, _permission, callback) => {
    callback(false);
  });
  app.on("web-contents-created", (_event, contents) => {
    contents.on("will-navigate", (event, navigationUrl) => {
      if (!isAllowedNavigation(navigationUrl)) {
        event.preventDefault();
      }
    });
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
  fs.writeFileSync(settingsPath(), `${JSON.stringify(settings, null, 2)}\n`, { mode: 0o600 });
}

function desktopOnboardingStatus() {
  return {
    completed: readDesktopSettings().onboardingCompleted === true,
  };
}

function completeDesktopOnboarding() {
  const settings = readDesktopSettings();
  settings.onboardingCompleted = true;
  writeDesktopSettings(settings);
  return desktopOnboardingStatus();
}

function resetDesktopOnboarding() {
  const settings = readDesktopSettings();
  settings.onboardingCompleted = false;
  writeDesktopSettings(settings);
  return desktopOnboardingStatus();
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

function clearStoredAccountSecret() {
  try {
    identityStore().clear();
  } catch (error) {
    console.error(`failed to clear stored Finite identity: ${error.message}`);
  }
}

function desktopIdentityStatus() {
  return {
    secureStorageAvailable: secureStorageAvailable(),
    hasStoredAccountSecret: Boolean(readStoredAccountSecret()),
    linking: Boolean(activeDeviceLink),
  };
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
  if (mainWindow && !mainWindow.isDestroyed() && !mainWindow.webContents.isDestroyed()) {
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
      stopUpdatePump();
      recordDaemonFailure(error);
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
    dispatchDaemonAction,
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

function restartDaemon() {
  return enqueueDaemonLifecycle(async () => {
    stopUpdatePump();
    const accountSecret = readStoredAccountSecret();
    if (!accountSecret) {
      if (daemonSupervisor) {
        await daemonSupervisor.stop();
        daemonSupervisor = null;
      }
      daemonFailure = null;
      return;
    }
    try {
      if (!daemonSupervisor) {
        daemonSupervisor = createDaemonSupervisor(accountSecret);
        await startDaemonProcessRuntime(() => daemonSupervisor.start());
      } else {
        await startDaemonProcessRuntime(() => daemonSupervisor.restart({ accountSecret }));
      }
    } catch (error) {
      stopUpdatePump();
      recordDaemonFailure(error);
      throw error;
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

async function dispatchDaemonAction(action) {
  if (!action || typeof action !== "object" || Array.isArray(action)) {
    throw new Error("Finite Chat action is invalid");
  }
  if (attachmentActionUsesBinaryTransport(action)) {
    throw new Error("Finite Chat attachments must use the binary upload transport");
  }
  const body = JSON.stringify(action);
  if (Buffer.byteLength(body) > maxIpcActionBytes) {
    throw new Error("Finite Chat action is too large");
  }
  if (!daemonSupervisor) {
    throw new Error(rendererDaemonError());
  }
  try {
    const requestGeneration = updateGeneration;
    const requestConnection = daemonSupervisor.connectionForUpdateStream();
    const state = await daemonSupervisor.requestJson("/v1/app/actions", {
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
  broadcastToRenderer("finitechat:device-link-status", status);
}

async function finishSuccessfulDeviceLink(link) {
  if (activeDeviceLink !== link) {
    return;
  }
  activeDeviceLink = null;
  try {
    await restartDaemon();
    deviceLinkStatus({ status: "linked" });
  } catch {
    deviceLinkStatus({
      status: "failed",
      message: "This desktop is linked, but its local runtime could not start. Restart Finite Chat to continue.",
    });
  }
}

function finishFailedDeviceLink(link, error) {
  if (activeDeviceLink !== link) {
    return;
  }
  activeDeviceLink = null;
  discardProvisionalAccountSecret();
  deviceLinkStatus({
    status: "failed",
    message: deviceLinkFailureMessage(error),
  });
}

async function beginDeviceLink() {
  if (!secureStorageAvailable()) {
    throw new Error("Secure storage is required to link this desktop");
  }
  if (readStoredAccountSecret()) {
    throw new Error("This desktop is already linked");
  }
  if (activeDeviceLink) {
    return activeDeviceLink.begin();
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
  let readyPromise;
  try {
    readyPromise = link.begin();
  } catch {
    activeDeviceLink = null;
    throw new Error("This desktop could not start device linking");
  }
  void link.completion.then(
    () => finishSuccessfulDeviceLink(link),
    (error) => finishFailedDeviceLink(link, error)
  );
  try {
    const ready = await readyPromise;
    if (activeDeviceLink !== link) {
      throw new Error("Finite Chat device link was cancelled");
    }
    deviceLinkStatus({ status: "waiting", ready });
    return ready;
  } catch (error) {
    finishFailedDeviceLink(link, error);
    throw new Error(deviceLinkFailureMessage(error, "This desktop could not start device linking"));
  }
}

async function openDeviceLinkApproval(approvalUrl) {
  const expected = activeDeviceLink?.ready?.approval_url;
  if (!expected || approvalUrl !== expected) {
    throw new Error("Finite Chat device-link approval address is invalid");
  }
  await shell.openExternal(expected);
  return true;
}

async function cancelDeviceLink() {
  const link = activeDeviceLink;
  if (!link) {
    return;
  }
  activeDeviceLink = null;
  await link.cancel();
  discardProvisionalAccountSecret();
  deviceLinkStatus({ status: "cancelled" });
}

function findTargetUrl(argv) {
  return argv.find((arg) => typeof arg === "string" && arg.startsWith("finite://")) || null;
}

function handleTargetUrl(url) {
  if (!url || !url.startsWith("finite://")) {
    return;
  }
  pendingTargetUrl = url;
  if (mainWindow) {
    mainWindow.webContents.send("finitechat:target-url", url);
    mainWindow.focus();
  } else {
    pendingTargetUrl = url;
  }
}

const useSingleInstanceLock = process.env.FINITECHAT_DISABLE_SINGLE_INSTANCE_LOCK !== "1";
const gotLock = useSingleInstanceLock ? app.requestSingleInstanceLock() : true;
if (!gotLock) {
  app.quit();
} else {
  const initialTargetUrl = findTargetUrl(process.argv);
  if (initialTargetUrl) {
    pendingTargetUrl = initialTargetUrl;
  }

  if (useSingleInstanceLock) {
    app.on("second-instance", (_event, argv) => {
      const targetUrl = findTargetUrl(argv);
      if (targetUrl) {
        handleTargetUrl(targetUrl);
      }
    });
  }

  app.on("open-url", (event, url) => {
    event.preventDefault();
    handleTargetUrl(url);
  });

  app.whenReady().then(async () => {
    discardProvisionalAccountSecret();
    registerAppProtocol();
    registerAttachmentMediaProtocol();
    configureSessionSecurity();
    if (process.env.FINITECHAT_SKIP_PROTOCOL_REGISTRATION !== "1") {
      if (process.defaultApp && process.argv.length >= 2) {
        app.setAsDefaultProtocolClient("finite", process.execPath, [path.resolve(process.argv[1])]);
      } else {
        app.setAsDefaultProtocolClient("finite");
      }
    }
    await startDaemon().catch(() => {});
    createWindow();
  });

  app.on("activate", async () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      await startDaemon().catch(() => {});
      createWindow();
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
  if (!mainWindow || mainWindow.isDestroyed() || event.sender !== mainWindow.webContents) {
    throw new Error("Finite Chat desktop request came from an untrusted renderer");
  }
}

ipcMain.handle("finitechat:daemon-connection", (event) => {
  assertTrustedIpcSender(event);
  return desktopIpcConnection;
});

ipcMain.handle("finitechat:daemon-state", (event) => {
  assertTrustedIpcSender(event);
  return requestDaemonState();
});

ipcMain.handle("finitechat:daemon-action", (event, action) => {
  assertTrustedIpcSender(event);
  return dispatchDaemonAction(action);
});

ipcMain.handle("finitechat:daemon-attachments", (event, upload) => {
  assertTrustedIpcSender(event);
  return uploadDaemonAttachments(upload);
});

ipcMain.handle("finitechat:daemon-subscribe", (event) => {
  assertTrustedIpcSender(event);
  updateRelay().replay((channel, value) => event.sender.send(channel, value));
  return true;
});

ipcMain.handle("finitechat:begin-device-link", (event) => {
  assertTrustedIpcSender(event);
  return beginDeviceLink();
});

ipcMain.handle("finitechat:open-device-link-approval", (event, approvalUrl) => {
  assertTrustedIpcSender(event);
  return openDeviceLinkApproval(approvalUrl);
});

ipcMain.handle("finitechat:cancel-device-link", (event) => {
  assertTrustedIpcSender(event);
  return cancelDeviceLink();
});

ipcMain.handle("finitechat:consume-pending-target-url", (event) => {
  assertTrustedIpcSender(event);
  const url = pendingTargetUrl;
  pendingTargetUrl = null;
  return url;
});

ipcMain.handle("finitechat:identity-status", (event) => {
  assertTrustedIpcSender(event);
  return desktopIdentityStatus();
});

ipcMain.handle("finitechat:onboarding-status", (event) => {
  assertTrustedIpcSender(event);
  return desktopOnboardingStatus();
});

ipcMain.handle("finitechat:complete-onboarding", (event) => {
  assertTrustedIpcSender(event);
  return completeDesktopOnboarding();
});

ipcMain.handle("finitechat:clear-account-secret", async (event) => {
  assertTrustedIpcSender(event);
  await cancelDeviceLink();
  clearStoredAccountSecret();
  resetDesktopOnboarding();
  await restartDaemon();
  return desktopIdentityStatus();
});

ipcMain.handle("finitechat:copy-text", (event, text) => {
  assertTrustedIpcSender(event);
  const value = String(text ?? "");
  if (!value) {
    throw new Error("Nothing to copy");
  }
  clipboard.writeText(value);
  return true;
});
