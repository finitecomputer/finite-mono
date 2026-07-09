const path = require("node:path");
const fs = require("node:fs");
const os = require("node:os");
const { spawn } = require("node:child_process");
const { pathToFileURL } = require("node:url");
const { app, BrowserWindow, clipboard, ipcMain, net, protocol, safeStorage, session, shell } = require("electron");

let mainWindow = null;
let pendingTargetUrl = null;
let daemonProcess = null;

const rendererUrl = process.env.FINITECHAT_RENDERER_URL;
const daemonUrl = process.env.FINITECHAT_DAEMON_URL || "http://127.0.0.1:38917";
const shouldStartBundledDaemon = !process.env.FINITECHAT_DAEMON_URL || process.env.FINITECHAT_START_DAEMON === "1";
const defaultServerUrl = process.env.FINITECHAT_SERVER_URL || "https://chat.finite.computer";
const appProtocol = "finitechat-app";

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

function isAllowedNavigation(navigationUrl) {
  const parsed = new URL(navigationUrl);
  if (parsed.protocol === `${appProtocol}:`) {
    return true;
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

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForDaemonReady(timeoutMs = 6000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${daemonUrl}/v1/healthz`);
      if (response.ok) {
        return true;
      }
    } catch {
      // The daemon is still starting.
    }
    await delay(150);
  }
  return false;
}

function daemonBindAddress() {
  const url = new URL(daemonUrl);
  return `${url.hostname}:${url.port || (url.protocol === "https:" ? "443" : "80")}`;
}

function daemonDataDir() {
  return process.env.FINITECHAT_HOME || path.join(app.getPath("userData"), "finitechatd");
}

function identitySecretPath() {
  return path.join(app.getPath("userData"), "account-secret.safe");
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

function secureStorageAvailable() {
  if (!safeStorage?.isEncryptionAvailable?.()) {
    return false;
  }
  if (safeStorage.getSelectedStorageBackend?.() === "basic_text") {
    return false;
  }
  return true;
}

function readStoredAccountSecret() {
  if (!secureStorageAvailable()) {
    return null;
  }
  try {
    const encrypted = fs.readFileSync(identitySecretPath());
    const secret = safeStorage.decryptString(encrypted).trim();
    return secret || null;
  } catch (error) {
    if (error?.code !== "ENOENT") {
      console.error(`failed to read stored Finite identity: ${error.message}`);
    }
    return null;
  }
}

function writeStoredAccountSecret(secret) {
  if (!secureStorageAvailable()) {
    throw new Error("Secure storage is unavailable on this desktop session");
  }
  const trimmed = String(secret ?? "").trim();
  if (!trimmed) {
    throw new Error("Account secret is empty");
  }
  fs.mkdirSync(path.dirname(identitySecretPath()), { recursive: true });
  fs.writeFileSync(identitySecretPath(), safeStorage.encryptString(trimmed), { mode: 0o600 });
}

function clearStoredAccountSecret() {
  try {
    fs.rmSync(identitySecretPath(), { force: true });
  } catch (error) {
    console.error(`failed to clear stored Finite identity: ${error.message}`);
  }
}

function desktopIdentityStatus() {
  return {
    secureStorageAvailable: secureStorageAvailable(),
    hasStoredAccountSecret: Boolean(readStoredAccountSecret()),
  };
}

function daemonDeviceId() {
  if (process.env.FINITECHAT_DEVICE_ID) {
    return process.env.FINITECHAT_DEVICE_ID;
  }
  return `electron-${os.hostname().replace(/[^a-zA-Z0-9._-]/g, "-")}`;
}

function startDaemon() {
  if (!shouldStartBundledDaemon || daemonProcess) {
    return;
  }
  const root = repoRoot();
  const debugBinary = path.join(
    root,
    "target",
    "debug",
    process.platform === "win32" ? "finitechatd.exe" : "finitechatd"
  );
  const args = [
    "--bind",
    daemonBindAddress(),
    "--data-dir",
    daemonDataDir(),
    "--server-url",
    defaultServerUrl,
    "--device-id",
    daemonDeviceId(),
  ];
  const storedAccountSecret = readStoredAccountSecret();
  if (storedAccountSecret) {
    args.push("--account-secret-stdin");
  }

  if (fs.existsSync(debugBinary)) {
    daemonProcess = spawn(debugBinary, args, { cwd: root, stdio: "pipe" });
  } else {
    daemonProcess = spawn("cargo", ["run", "-p", "finitechat-daemon", "--", ...args], {
      cwd: root,
      stdio: "pipe",
    });
  }
  if (storedAccountSecret) {
    daemonProcess.stdin.end(`${storedAccountSecret}\n`);
  }
  daemonProcess.stdout.on("data", (chunk) => console.log(`[finitechatd] ${chunk.toString().trimEnd()}`));
  daemonProcess.stderr.on("data", (chunk) => console.error(`[finitechatd] ${chunk.toString().trimEnd()}`));
  daemonProcess.on("exit", (code, signal) => {
    if (code !== 0 && signal !== "SIGTERM") {
      console.error(`finitechatd exited with code=${code} signal=${signal}`);
    }
    daemonProcess = null;
  });
}

function stopDaemon() {
  if (!daemonProcess) {
    return;
  }
  const exiting = daemonProcess;
  daemonProcess = null;
  exiting.kill();
}

function restartDaemon() {
  stopDaemon();
  setTimeout(() => startDaemon(), 300);
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
    registerAppProtocol();
    configureSessionSecurity();
    if (process.env.FINITECHAT_SKIP_PROTOCOL_REGISTRATION !== "1") {
      if (process.defaultApp && process.argv.length >= 2) {
        app.setAsDefaultProtocolClient("finite", process.execPath, [path.resolve(process.argv[1])]);
      } else {
        app.setAsDefaultProtocolClient("finite");
      }
    }
    startDaemon();
    await waitForDaemonReady();
    createWindow();
  });

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });

  app.on("window-all-closed", () => {
    stopDaemon();
    if (process.platform !== "darwin") {
      app.quit();
    }
  });
}

ipcMain.handle("finitechat:daemon-url", () => {
  return daemonUrl;
});

ipcMain.handle("finitechat:consume-pending-target-url", () => {
  const url = pendingTargetUrl;
  pendingTargetUrl = null;
  return url;
});

ipcMain.handle("finitechat:identity-status", () => {
  return desktopIdentityStatus();
});

ipcMain.handle("finitechat:onboarding-status", () => {
  return desktopOnboardingStatus();
});

ipcMain.handle("finitechat:complete-onboarding", () => {
  return completeDesktopOnboarding();
});

ipcMain.handle("finitechat:import-account-secret", (_event, secret) => {
  writeStoredAccountSecret(secret);
  restartDaemon();
  return desktopIdentityStatus();
});

ipcMain.handle("finitechat:clear-account-secret", () => {
  clearStoredAccountSecret();
  restartDaemon();
  return desktopIdentityStatus();
});

ipcMain.handle("finitechat:copy-text", (_event, text) => {
  const value = String(text ?? "");
  if (!value) {
    throw new Error("Nothing to copy");
  }
  clipboard.writeText(value);
  return true;
});
