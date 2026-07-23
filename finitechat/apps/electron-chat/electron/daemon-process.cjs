const crypto = require("node:crypto");
const fs = require("node:fs");
const net = require("node:net");
const path = require("node:path");

const READY_LINE_LIMIT_BYTES = 4 * 1024;
const STARTUP_DOCUMENT_LIMIT_BYTES = 2 * 1024;
const DEFAULT_READY_TIMEOUT_MS = 10_000;
const DEFAULT_STOP_TIMEOUT_MS = 2_000;
const DEVICE_LINK_LINE_LIMIT_BYTES = 4 * 1024;

const DEVICE_LINK_FAILURE_MESSAGES = Object.freeze({
  FINITECHAT_DEVICE_LINK_INVALID_CONFIGURATION:
    "Finite Chat has an invalid device-link configuration. Restart the app and try again.",
  FINITECHAT_DEVICE_LINK_ENTROPY:
    "This desktop could not create a secure device link. Restart the app and try again.",
  FINITECHAT_DEVICE_LINK_REQUEST:
    "Finite Chat could not reach the device-link service. Check your connection and start a new link.",
  FINITECHAT_DEVICE_LINK_SERVER_STATUS:
    "The device-link service rejected this request. Start a new link and try again.",
  FINITECHAT_DEVICE_LINK_INVALID_RESPONSE:
    "The device-link service returned an invalid response. Start a new link and try again.",
  FINITECHAT_DEVICE_LINK_EXPIRED: "This device link expired. Start a new link to try again.",
  FINITECHAT_DEVICE_LINK_PAYLOAD_REJECTED:
    "The approved device-link payload did not match this link. Start a new link to try again.",
  FINITECHAT_DEVICE_LINK_RESULT_PIPE:
    "This desktop could not securely receive the linked account. Start a new link to try again.",
});

const DEVICE_LINK_BOOTSTRAP_FAILURE_CODES = new Map([
  ["invalid device-link configuration", "FINITECHAT_DEVICE_LINK_INVALID_CONFIGURATION"],
  ["device-link entropy generation failed", "FINITECHAT_DEVICE_LINK_ENTROPY"],
  ["device-link server request failed", "FINITECHAT_DEVICE_LINK_REQUEST"],
  ["device-link server returned an invalid response", "FINITECHAT_DEVICE_LINK_INVALID_RESPONSE"],
  ["device-link request expired", "FINITECHAT_DEVICE_LINK_EXPIRED"],
  ["device-link payload failed authentication", "FINITECHAT_DEVICE_LINK_PAYLOAD_REJECTED"],
  ["device-link result pipe failed", "FINITECHAT_DEVICE_LINK_RESULT_PIPE"],
]);

function deviceLinkFailure(code) {
  const message = DEVICE_LINK_FAILURE_MESSAGES[code];
  if (!message) {
    return null;
  }
  const error = new Error(message);
  error.code = code;
  return error;
}

/** Classify only the daemon's bounded, public DeviceLinkBootstrapError text. */
function parseDeviceLinkBootstrapError(line) {
  if (typeof line !== "string" || Buffer.byteLength(line) > DEVICE_LINK_LINE_LIMIT_BYTES) {
    return null;
  }
  const exactCode = DEVICE_LINK_BOOTSTRAP_FAILURE_CODES.get(line);
  if (exactCode) {
    return deviceLinkFailure(exactCode);
  }
  if (/^device-link server rejected the request \([1-9][0-9]{2}\)$/.test(line)) {
    return deviceLinkFailure("FINITECHAT_DEVICE_LINK_SERVER_STATUS");
  }
  return null;
}

function deviceLinkFailureMessage(
  error,
  fallback = "This desktop could not be linked. Start a new link to try again."
) {
  const code = error?.code;
  return typeof code === "string" && Object.prototype.hasOwnProperty.call(DEVICE_LINK_FAILURE_MESSAGES, code)
    ? DEVICE_LINK_FAILURE_MESSAGES[code]
    : fallback;
}

function legacyHostnameDeviceId(hostname) {
  // This intentionally matches the exact derivation used before Device ids
  // were persisted. Changing its normalization would fork an existing store.
  return `electron-${String(hostname ?? "").replace(/[^a-zA-Z0-9._-]/g, "-")}`;
}

function validDeviceId(value) {
  return typeof value === "string" && value.length > 0 && value.length <= 255 && /^[a-zA-Z0-9._-]+$/.test(value);
}

function directoryHasEntries(directory, fileSystem = fs) {
  try {
    return fileSystem.readdirSync(directory).length > 0;
  } catch (error) {
    if (error?.code === "ENOENT") {
      return false;
    }
    throw error;
  }
}

function readJsonObject(filePath, fileSystem = fs) {
  try {
    const value = JSON.parse(fileSystem.readFileSync(filePath, "utf8"));
    return value && typeof value === "object" && !Array.isArray(value) ? value : {};
  } catch (error) {
    if (error?.code === "ENOENT") {
      return {};
    }
    if (error instanceof SyntaxError) {
      throw new Error("Finite Chat desktop settings are invalid");
    }
    throw error;
  }
}

function writeJsonObject(filePath, value, fileSystem = fs) {
  fileSystem.mkdirSync(path.dirname(filePath), { recursive: true });
  const temporaryPath = `${filePath}.${process.pid}.tmp`;
  fileSystem.writeFileSync(temporaryPath, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
  fileSystem.renameSync(temporaryPath, filePath);
  try {
    fileSystem.chmodSync(filePath, 0o600);
  } catch (error) {
    if (process.platform !== "win32") {
      throw error;
    }
  }
}

/**
 * Return this installation's stable Device id.
 *
 * Fresh installations receive an opaque random id. An installation which has
 * daemon data from the hostname-derived pre-alpha keeps that old id exactly
 * once and persists it, so an upgrade does not strand its local MLS state or
 * silently enroll a duplicate Device. A later hostname change cannot change
 * either form of persisted id.
 */
function loadOrCreateDeviceId({
  settingsFile,
  daemonDataDirectory,
  hostname,
  randomUUID = crypto.randomUUID,
  fileSystem = fs,
}) {
  const settings = readJsonObject(settingsFile, fileSystem);
  if (validDeviceId(settings.deviceId)) {
    return settings.deviceId;
  }
  if (Object.prototype.hasOwnProperty.call(settings, "deviceId")) {
    throw new Error("Finite Chat desktop settings contain an invalid Device id");
  }

  const deviceId = directoryHasEntries(daemonDataDirectory, fileSystem)
    ? legacyHostnameDeviceId(hostname)
    : `electron-${randomUUID()}`;
  if (!validDeviceId(deviceId)) {
    throw new Error("Failed to create a valid Finite Chat Device id");
  }
  settings.deviceId = deviceId;
  writeJsonObject(settingsFile, settings, fileSystem);
  return deviceId;
}

function pathExists(filePath, fileSystem = fs) {
  try {
    fileSystem.statSync(filePath);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

/**
 * Preserve a revoked Device's complete local cryptographic profile, then
 * clear only the identity fields needed for the existing link flow to create
 * a fresh Device. The dashboard Chromium profile and sign-in session stay in
 * place. Any partial filesystem failure rolls the old profile back.
 */
function archiveRevokedDeviceProfile({
  userDataDirectory,
  daemonDataDirectory,
  settingsFile,
  secretFile,
  deviceId,
  now = new Date(),
  randomUUID = crypto.randomUUID,
  fileSystem = fs,
}) {
  if (!path.isAbsolute(userDataDirectory) || !validDeviceId(deviceId)) {
    throw new Error("Finite Chat cannot archive an invalid Device profile");
  }
  const settings = readJsonObject(settingsFile, fileSystem);
  if (settings.deviceId !== deviceId) {
    throw new Error("Finite Chat Device settings changed before recovery");
  }

  const stamp = now.toISOString().replace(/[:.]/g, "-");
  const backupRoot = path.join(userDataDirectory, "revoked-device-backups");
  const backupDirectory = path.join(backupRoot, `${stamp}-${randomUUID()}`);
  const backupDaemonDirectory = path.join(backupDirectory, "finitechatd");
  const backupSecretFile = path.join(backupDirectory, "account-secret.safe");
  const backupSettingsFile = path.join(backupDirectory, "desktop-settings.json");
  const settingsExisted = pathExists(settingsFile, fileSystem);
  let movedDaemon = false;
  let movedSecret = false;

  fileSystem.mkdirSync(backupRoot, { recursive: true, mode: 0o700 });
  fileSystem.mkdirSync(backupDirectory, { recursive: false, mode: 0o700 });
  try {
    writeJsonObject(backupSettingsFile, settings, fileSystem);
    if (pathExists(daemonDataDirectory, fileSystem)) {
      fileSystem.renameSync(daemonDataDirectory, backupDaemonDirectory);
      movedDaemon = true;
    }
    if (pathExists(secretFile, fileSystem)) {
      fileSystem.renameSync(secretFile, backupSecretFile);
      movedSecret = true;
    }

    const nextSettings = { ...settings };
    delete nextSettings.deviceId;
    delete nextSettings.pendingDeviceLink;
    writeJsonObject(settingsFile, nextSettings, fileSystem);
    writeJsonObject(
      path.join(backupDirectory, "recovery.json"),
      {
        version: 1,
        reason: "device_revoked",
        device_id: deviceId,
        archived_at: now.toISOString(),
      },
      fileSystem
    );
    return { backupDirectory };
  } catch (error) {
    try {
      if (settingsExisted) {
        writeJsonObject(settingsFile, settings, fileSystem);
      } else {
        fileSystem.rmSync(settingsFile, { force: true });
      }
      if (movedSecret && !pathExists(secretFile, fileSystem)) {
        fileSystem.renameSync(backupSecretFile, secretFile);
      }
      if (movedDaemon && !pathExists(daemonDataDirectory, fileSystem)) {
        fileSystem.renameSync(backupDaemonDirectory, daemonDataDirectory);
      }
      fileSystem.rmSync(backupDirectory, { recursive: true, force: true });
    } catch {
      // Preserve the original failure. The bounded backup path remains the
      // recovery boundary if a rollback operation itself cannot complete.
    }
    throw error;
  }
}

class AccountSecretStore {
  constructor({ secretPath, safeStorage, fileSystem = fs }) {
    this.secretPath = secretPath;
    this.provisionalPath = `${secretPath}.linking`;
    this.safeStorage = safeStorage;
    this.fileSystem = fileSystem;
  }

  isAvailable() {
    return (
      Boolean(this.safeStorage?.isEncryptionAvailable?.()) &&
      this.safeStorage.getSelectedStorageBackend?.() !== "basic_text"
    );
  }

  read() {
    if (!this.isAvailable()) {
      return null;
    }
    let encrypted;
    try {
      encrypted = this.fileSystem.readFileSync(this.secretPath);
    } catch (error) {
      if (error?.code === "ENOENT") {
        return null;
      }
      throw error;
    }
    const secret = this.safeStorage.decryptString(encrypted).trim();
    return secret || null;
  }

  writeProvisional(secret) {
    if (!this.isAvailable()) {
      throw new Error("Secure storage is unavailable on this desktop session");
    }
    const trimmed = String(secret ?? "").trim();
    if (!trimmed) {
      throw new Error("Account secret is empty");
    }
    this.fileSystem.mkdirSync(path.dirname(this.secretPath), { recursive: true });
    this.fileSystem.writeFileSync(this.provisionalPath, this.safeStorage.encryptString(trimmed), { mode: 0o600 });
  }

  promoteProvisional() {
    this.fileSystem.renameSync(this.provisionalPath, this.secretPath);
  }

  discardProvisional() {
    this.fileSystem.rmSync(this.provisionalPath, { force: true });
  }

  clear() {
    this.discardProvisional();
    this.fileSystem.rmSync(this.secretPath, { force: true });
  }
}

function assertExecutableFile(filePath, fileSystem = fs) {
  let stat;
  try {
    stat = fileSystem.statSync(filePath);
  } catch (error) {
    if (error?.code === "ENOENT") {
      throw new Error(`Finite Chat daemon binary was not found at ${filePath}`);
    }
    throw error;
  }
  if (!stat.isFile()) {
    throw new Error(`Finite Chat daemon binary is not a file: ${filePath}`);
  }
  return filePath;
}

/** Resolve only an explicitly supplied development binary or packaged asset. */
function resolveDaemonBinary({
  explicitPath,
  isPackaged,
  resourcesPath,
  platform = process.platform,
  cwd = process.cwd(),
  fileSystem = fs,
}) {
  const binaryName = platform === "win32" ? "finitechatd.exe" : "finitechatd";
  if (explicitPath) {
    return assertExecutableFile(path.resolve(cwd, explicitPath), fileSystem);
  }
  if (isPackaged) {
    return assertExecutableFile(path.join(resourcesPath, binaryName), fileSystem);
  }
  throw new Error(
    "Finite Chat daemon binary is required in development; set FINITECHAT_DAEMON_BINARY to a built finitechatd"
  );
}

function isLoopbackHostname(hostname) {
  if (hostname === "localhost") {
    return true;
  }
  const normalized = hostname.startsWith("[") && hostname.endsWith("]") ? hostname.slice(1, -1) : hostname;
  const family = net.isIP(normalized);
  if (family === 4) {
    return normalized.startsWith("127.");
  }
  return family === 6 && normalized === "::1";
}

function parseReadyRecord(line) {
  if (typeof line !== "string" || Buffer.byteLength(line) > READY_LINE_LIMIT_BYTES) {
    throw new Error("Finite Chat daemon emitted an invalid ready record");
  }
  let record;
  try {
    record = JSON.parse(line);
  } catch {
    throw new Error("Finite Chat daemon emitted an invalid ready record");
  }
  if (
    !record ||
    typeof record !== "object" ||
    Array.isArray(record) ||
    record.event !== "ready" ||
    typeof record.url !== "string" ||
    Object.keys(record).some((key) => key !== "event" && key !== "url")
  ) {
    throw new Error("Finite Chat daemon emitted an invalid ready record");
  }
  let url;
  try {
    url = new URL(record.url);
  } catch {
    throw new Error("Finite Chat daemon emitted an invalid ready record");
  }
  if (
    url.protocol !== "http:" ||
    !isLoopbackHostname(url.hostname) ||
    !url.port ||
    url.pathname !== "/" ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  ) {
    throw new Error("Finite Chat daemon emitted an unsafe ready address");
  }
  url.username = "";
  url.password = "";
  url.search = "";
  url.hash = "";
  return url.origin;
}

function startupDocument(authToken, accountSecret) {
  const document = {
    auth_token: authToken,
    ...(accountSecret ? { account_secret: accountSecret } : {}),
  };
  const serialized = `${JSON.stringify(document)}\n`;
  if (Buffer.byteLength(serialized) > STARTUP_DOCUMENT_LIMIT_BYTES) {
    throw new Error("Finite Chat daemon startup document is too large");
  }
  return serialized;
}

function parseDeviceLinkReadyRecord(line) {
  if (typeof line !== "string" || Buffer.byteLength(line) > DEVICE_LINK_LINE_LIMIT_BYTES) {
    throw new Error("Finite Chat device link emitted an invalid status record");
  }
  let record;
  try {
    record = JSON.parse(line);
  } catch {
    throw new Error("Finite Chat device link emitted an invalid status record");
  }
  if (
    !record ||
    typeof record !== "object" ||
    Array.isArray(record) ||
    record.event !== "link_ready" ||
    typeof record.link_session_id !== "string" ||
    !record.link_session_id ||
    typeof record.target_device_id !== "string" ||
    !validDeviceId(record.target_device_id) ||
    typeof record.approval_url !== "string"
  ) {
    throw new Error("Finite Chat device link emitted an invalid status record");
  }
  let approvalUrl;
  try {
    approvalUrl = new URL(record.approval_url);
  } catch {
    throw new Error("Finite Chat device link emitted an invalid approval address");
  }
  if (!matchesAllowedWebProtocol(approvalUrl) || approvalUrl.username || approvalUrl.password) {
    throw new Error("Finite Chat device link emitted an invalid approval address");
  }
  return {
    link_session_id: record.link_session_id,
    target_device_id: record.target_device_id,
    approval_url: approvalUrl.toString(),
  };
}

function matchesAllowedWebProtocol(url) {
  return url.protocol === "https:" || (url.protocol === "http:" && isLoopbackHostname(url.hostname));
}

function parseDeviceLinkSecretRecord(line) {
  if (typeof line !== "string" || Buffer.byteLength(line) > DEVICE_LINK_LINE_LIMIT_BYTES) {
    throw new Error("Finite Chat device link emitted an invalid private result");
  }
  let record;
  try {
    record = JSON.parse(line);
  } catch {
    throw new Error("Finite Chat device link emitted an invalid private result");
  }
  if (
    !record ||
    typeof record !== "object" ||
    Array.isArray(record) ||
    typeof record.account_secret !== "string" ||
    !/^[0-9a-fA-F]{64}$/.test(record.account_secret)
  ) {
    throw new Error("Finite Chat device link emitted an invalid private result");
  }
  return record.account_secret;
}

function validDeviceLinkApproval(ready, dashboardUrl) {
  try {
    const approval = new URL(ready.approval_url);
    const dashboard = new URL(dashboardUrl);
    const queryKeys = [...approval.searchParams.keys()];
    return (
      approval.origin === dashboard.origin &&
      approval.pathname === "/dashboard/device-link" &&
      queryKeys.length === 2 &&
      queryKeys.includes("link_session_id") &&
      queryKeys.includes("target_device_id") &&
      approval.searchParams.get("link_session_id") === ready.link_session_id &&
      approval.searchParams.get("target_device_id") === ready.target_device_id
    );
  } catch {
    return false;
  }
}

class BoundedLineReader {
  constructor(stream, { limitBytes = DEVICE_LINK_LINE_LIMIT_BYTES, onLine, onFailure }) {
    this.stream = stream;
    this.limitBytes = limitBytes;
    this.onLine = onLine;
    this.onFailure = onFailure;
    this.buffer = Buffer.alloc(0);
    this.onData = this.#onData.bind(this);
    stream.on("data", this.onData);
  }

  #onData(chunk) {
    this.buffer = Buffer.concat([this.buffer, Buffer.from(chunk)]);
    if (this.buffer.length > this.limitBytes && this.buffer.indexOf(0x0a) === -1) {
      this.onFailure(new Error("Finite Chat device link record is too large"));
      this.close();
      return;
    }
    while (true) {
      const newline = this.buffer.indexOf(0x0a);
      if (newline === -1) {
        return;
      }
      const line = this.buffer.subarray(0, newline).toString("utf8").trimEnd();
      this.buffer = this.buffer.subarray(newline + 1);
      if (Buffer.byteLength(line) > this.limitBytes) {
        this.onFailure(new Error("Finite Chat device link record is too large"));
        this.close();
        return;
      }
      this.onLine(line);
    }
  }

  close() {
    this.stream.removeListener("data", this.onData);
  }
}

class DeviceLinkSupervisor {
  constructor({
    spawnProcess,
    binaryPath,
    serverUrl,
    dashboardUrl,
    deviceId,
    cwd,
    storeAccountSecret,
    promoteAccountSecret,
    stopTimeoutMs = DEFAULT_STOP_TIMEOUT_MS,
  }) {
    this.spawnProcess = spawnProcess;
    this.binaryPath = binaryPath;
    this.serverUrl = serverUrl;
    this.dashboardUrl = dashboardUrl;
    this.deviceId = deviceId;
    this.cwd = cwd;
    this.storeAccountSecret = storeAccountSecret;
    this.promoteAccountSecret = promoteAccountSecret;
    this.stopTimeoutMs = stopTimeoutMs;
    this.child = null;
    this.ready = null;
    this.cancelled = false;
    this.settled = false;
    this.sawLinked = false;
    this.secretStored = false;
    this.secretPromoted = false;
    this.childFailure = null;
    this.privateResultPromise = null;
    this.promotionPromise = null;
    this.readers = [];
    this.readyPromise = null;
    this.completion = null;
  }

  begin() {
    if (this.child) {
      return this.readyPromise;
    }
    const child = this.spawnProcess(
      this.binaryPath,
      [
        "link",
        "--server-url",
        this.serverUrl,
        "--dashboard-url",
        this.dashboardUrl,
        "--device-id",
        this.deviceId,
        "--result-fd",
        "3",
        "--confirm-fd",
        "4",
      ],
      {
        cwd: this.cwd,
        stdio: ["ignore", "pipe", "pipe", "pipe", "pipe"],
        windowsHide: true,
      }
    );
    this.child = child;

    let resolveReady;
    let rejectReady;
    let resolveCompletion;
    let rejectCompletion;
    this.readyPromise = new Promise((resolve, reject) => {
      resolveReady = resolve;
      rejectReady = reject;
    });
    this.completion = new Promise((resolve, reject) => {
      resolveCompletion = resolve;
      rejectCompletion = reject;
    });
    // Main attaches a completion handler after awaiting readiness. Keep a fast
    // startup failure from becoming a process-level unhandled rejection.
    this.completion.catch(() => {});
    this.resolveReady = resolveReady;
    this.rejectReady = rejectReady;
    this.resolveCompletion = resolveCompletion;
    this.rejectCompletion = rejectCompletion;

    const fail = (error) => this.#fail(error);
    this.readers.push(
      new BoundedLineReader(child.stdout, {
        onLine: (line) => this.#onPublicLine(line),
        onFailure: fail,
      }),
      new BoundedLineReader(child.stdio[3], {
        onLine: (line) => void this.#onPrivateLine(line),
        onFailure: fail,
      }),
      new BoundedLineReader(child.stderr, {
        onLine: (line) => {
          this.childFailure ??= parseDeviceLinkBootstrapError(line);
        },
        // stderr is diagnostic-only. Unknown or oversized output must neither
        // enter renderer state nor override the generic close failure.
        // Keep draining after the bounded classifier detaches so a noisy child
        // cannot block while trying to exit on a full stderr pipe.
        onFailure: () => child.stderr.resume(),
      })
    );
    child.once("error", () => fail(new Error("Finite Chat device link could not start")));
    // `exit` can precede the final stdout/stderr data. Node's `close` event is
    // emitted only after the child has exited and its stdio streams have
    // closed, so keep the line readers attached until then.
    child.once("close", (code, signal) => this.#onClose(code, signal));
    return this.readyPromise;
  }

  #onPublicLine(line) {
    if (!this.ready) {
      try {
        this.ready = parseDeviceLinkReadyRecord(line);
      } catch (error) {
        this.#fail(error);
        return;
      }
      if (this.ready.target_device_id !== this.deviceId) {
        this.#fail(new Error("Finite Chat device link targeted a different Device"));
        return;
      }
      if (!validDeviceLinkApproval(this.ready, this.dashboardUrl)) {
        this.#fail(new Error("Finite Chat device link emitted an invalid approval address"));
        return;
      }
      this.resolveReady(this.ready);
      return;
    }
    let record;
    try {
      record = JSON.parse(line);
    } catch {
      this.#fail(new Error("Finite Chat device link emitted an invalid status record"));
      return;
    }
    if (record?.event !== "linked" || !this.secretStored) {
      this.#fail(new Error("Finite Chat device link emitted an invalid status transition"));
      return;
    }
    this.sawLinked = true;
    this.promotionPromise = this.#promotePrivateResult();
  }

  async #onPrivateLine(line) {
    if (!this.ready || this.privateResultPromise || this.secretStored || this.settled || this.cancelled) {
      this.#fail(new Error("Finite Chat device link emitted an invalid private transition"));
      return;
    }
    this.privateResultPromise = this.#storePrivateResult(line);
    await this.privateResultPromise;
  }

  async #storePrivateResult(line) {
    let accountSecret;
    try {
      accountSecret = parseDeviceLinkSecretRecord(line);
      await this.storeAccountSecret(accountSecret);
      this.secretStored = true;
      if (this.cancelled || this.settled || !this.child) {
        return;
      }
      this.child.stdio[4].end("stored\n");
    } catch {
      this.#fail(new Error("Finite Chat could not securely store the linked account"));
    } finally {
      accountSecret = null;
    }
  }

  async #promotePrivateResult() {
    try {
      await this.promoteAccountSecret();
      this.secretPromoted = true;
    } catch {
      this.#fail(new Error("Finite Chat could not commit the linked account"));
    }
  }

  async #onClose(code, signal) {
    this.#closeReaders();
    this.child = null;
    if (this.privateResultPromise) {
      await this.privateResultPromise.catch(() => {});
    }
    if (this.promotionPromise) {
      await this.promotionPromise.catch(() => {});
    }
    if (this.settled) {
      return;
    }
    this.settled = true;
    if (this.cancelled) {
      const error = new Error("Finite Chat device link was cancelled");
      if (!this.ready) {
        this.rejectReady(error);
      }
      this.rejectCompletion(error);
      return;
    }
    if (code === 0 && this.sawLinked && this.secretStored && this.secretPromoted) {
      this.resolveCompletion();
      return;
    }
    const error =
      this.childFailure ??
      new Error(`Finite Chat device link stopped before completion (${code ?? signal ?? "unknown"})`);
    if (!this.ready) {
      this.rejectReady(error);
    }
    this.rejectCompletion(error);
  }

  #fail(error) {
    if (this.settled) {
      return;
    }
    this.settled = true;
    this.#closeReaders();
    if (!this.ready) {
      this.rejectReady(error);
    }
    this.rejectCompletion(error);
    const child = this.child;
    this.child = null;
    if (child && child.exitCode === null && child.signalCode === null) {
      child.kill();
    }
  }

  #closeReaders() {
    for (const reader of this.readers) {
      reader.close();
    }
    this.readers = [];
  }

  async cancel() {
    if (this.settled) {
      return;
    }
    this.cancelled = true;
    const child = this.child;
    if (child) {
      child.kill();
      await waitForChildExit(child, this.stopTimeoutMs);
    }
    if (this.privateResultPromise) {
      await this.privateResultPromise.catch(() => {});
    }
    if (child && child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL");
      await waitForChildExit(child, this.stopTimeoutMs);
    }
  }
}

async function startDaemonRuntime({ startDaemonProcess, dispatchDaemonAction }) {
  const started = await startDaemonProcess();
  if (!started) {
    return null;
  }
  return dispatchDaemonAction({ StartRuntime: null });
}

function daemonRequestVersionMatches(
  requestGeneration,
  requestConnection,
  currentGeneration,
  currentConnection
) {
  return Boolean(
    requestConnection &&
      currentConnection &&
      requestGeneration === currentGeneration &&
      requestConnection.url === currentConnection.url &&
      requestConnection.authToken === currentConnection.authToken
  );
}

class DaemonUpdateRelay {
  constructor(deliver) {
    this.deliver = deliver;
    this.generation = 0;
    this.latestState = null;
  }

  beginGeneration() {
    this.generation += 1;
    this.latestState = null;
    this.deliver("finitechat:daemon-generation", { generation: this.generation });
    return this.generation;
  }

  update(state) {
    if (
      this.latestState &&
      Number.isSafeInteger(this.latestState.rev) &&
      Number.isSafeInteger(state?.rev) &&
      this.latestState.rev === state.rev
    ) {
      return false;
    }
    this.latestState = state;
    this.deliver("finitechat:daemon-update", state);
    return true;
  }

  replay(deliver = this.deliver) {
    if (this.generation === 0) {
      return;
    }
    deliver("finitechat:daemon-generation", { generation: this.generation });
    if (this.latestState) {
      deliver("finitechat:daemon-update", this.latestState);
    }
  }
}

function waitForChildExit(child, timeoutMs, setTimer = setTimeout, clearTimer = clearTimeout) {
  return new Promise((resolve) => {
    if (child.exitCode !== null || child.signalCode !== null) {
      resolve();
      return;
    }
    let timer = null;
    const done = () => {
      if (timer) {
        clearTimer(timer);
      }
      child.removeListener("exit", done);
      resolve();
    };
    child.once("exit", done);
    timer = setTimer(done, timeoutMs);
  });
}

class DaemonSupervisor {
  constructor({
    spawnProcess,
    binaryPath,
    args,
    cwd,
    accountSecret,
    fetchImpl = globalThis.fetch,
    randomBytes = crypto.randomBytes,
    readyTimeoutMs = DEFAULT_READY_TIMEOUT_MS,
    stopTimeoutMs = DEFAULT_STOP_TIMEOUT_MS,
    onUnexpectedExit = () => {},
  }) {
    this.spawnProcess = spawnProcess;
    this.binaryPath = binaryPath;
    this.args = [...args];
    this.cwd = cwd;
    this.accountSecret = accountSecret;
    this.fetchImpl = fetchImpl;
    this.randomBytes = randomBytes;
    this.readyTimeoutMs = readyTimeoutMs;
    this.stopTimeoutMs = stopTimeoutMs;
    this.onUnexpectedExit = onUnexpectedExit;
    this.child = null;
    this.connection = null;
    this.startPromise = null;
    this.stopping = false;
  }

  async start() {
    if (this.connection) {
      return false;
    }
    if (this.startPromise) {
      await this.startPromise;
      return false;
    }
    this.startPromise = this.#startOnce();
    try {
      await this.startPromise;
      return true;
    } finally {
      this.startPromise = null;
    }
  }

  async #startOnce() {
    const authToken = this.randomBytes(32).toString("hex");
    const child = this.spawnProcess(this.binaryPath, this.args, {
      cwd: this.cwd,
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });
    this.child = child;
    this.stopping = false;
    child.stderr.resume();

    const ready = this.#waitForReady(child);
    child.once("exit", (code, signal) => {
      const wasCurrent = this.child === child;
      const wasReady = Boolean(this.connection);
      if (wasCurrent) {
        this.child = null;
        this.connection = null;
      }
      if (wasCurrent && wasReady && !this.stopping) {
        this.onUnexpectedExit(new Error(`Finite Chat daemon stopped unexpectedly (${code ?? signal ?? "unknown"})`));
      }
    });
    child.stdin.on("error", () => {
      // A startup/exit waiter reports the failure without exposing stdin data.
    });

    try {
      child.stdin.end(startupDocument(authToken, this.accountSecret));
      const url = await ready;
      child.stdout.resume();
      const readinessAbort = new AbortController();
      const readinessTimer = setTimeout(() => readinessAbort.abort(), this.readyTimeoutMs);
      let response;
      try {
        response = await this.fetchImpl(`${url}/v1/healthz`, {
          headers: { authorization: `Bearer ${authToken}` },
          signal: readinessAbort.signal,
        });
      } finally {
        clearTimeout(readinessTimer);
      }
      if (!response.ok) {
        throw new Error("Finite Chat daemon failed its authenticated readiness check");
      }
      if (this.child !== child) {
        throw new Error("Finite Chat daemon stopped during startup");
      }
      this.connection = { url, authToken };
    } catch (error) {
      if (this.child === child) {
        this.stopping = true;
        child.kill();
        await waitForChildExit(child, this.stopTimeoutMs);
        this.child = null;
        this.stopping = false;
      }
      this.connection = null;
      throw error;
    }
  }

  #waitForReady(child) {
    return new Promise((resolve, reject) => {
      let buffer = Buffer.alloc(0);
      let settled = false;
      const finish = (callback, value) => {
        if (settled) {
          return;
        }
        settled = true;
        clearTimeout(timer);
        child.stdout.removeListener("data", onData);
        child.removeListener("error", onError);
        child.removeListener("exit", onExit);
        callback(value);
      };
      const onData = (chunk) => {
        buffer = Buffer.concat([buffer, Buffer.from(chunk)]);
        if (buffer.length > READY_LINE_LIMIT_BYTES) {
          finish(reject, new Error("Finite Chat daemon emitted an invalid ready record"));
          return;
        }
        const newline = buffer.indexOf(0x0a);
        if (newline === -1) {
          return;
        }
        try {
          finish(resolve, parseReadyRecord(buffer.subarray(0, newline).toString("utf8").trimEnd()));
        } catch (error) {
          finish(reject, error);
        }
      };
      const onError = () => finish(reject, new Error("Finite Chat daemon could not be started"));
      const onExit = () => finish(reject, new Error("Finite Chat daemon stopped during startup"));
      const timer = setTimeout(
        () => finish(reject, new Error("Finite Chat daemon did not become ready in time")),
        this.readyTimeoutMs
      );
      child.stdout.on("data", onData);
      child.once("error", onError);
      child.once("exit", onExit);
    });
  }

  async requestJson(pathname, init = {}) {
    if (!this.connection) {
      throw new Error("Finite Chat daemon is unavailable");
    }
    const headers = new Headers(init.headers);
    headers.set("authorization", `Bearer ${this.connection.authToken}`);
    if (init.body && !headers.has("content-type")) {
      headers.set("content-type", "application/json");
    }
    const response = await this.fetchImpl(`${this.connection.url}${pathname}`, { ...init, headers });
    if (!response.ok) {
      const body = await response.text();
      let message = `Finite Chat daemon request failed (${response.status})`;
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
    return response.json();
  }

  connectionForUpdateStream() {
    if (!this.connection) {
      throw new Error("Finite Chat daemon is unavailable");
    }
    return { ...this.connection };
  }

  async stop() {
    const child = this.child;
    this.connection = null;
    if (!child) {
      return;
    }
    this.stopping = true;
    child.kill();
    await waitForChildExit(child, this.stopTimeoutMs);
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL");
      await waitForChildExit(child, this.stopTimeoutMs);
    }
    if (this.child === child) {
      this.child = null;
    }
    this.stopping = false;
  }

  async restart({ accountSecret = this.accountSecret } = {}) {
    await this.stop();
    this.accountSecret = accountSecret;
    return this.start();
  }
}

module.exports = {
  AccountSecretStore,
  DaemonUpdateRelay,
  DaemonSupervisor,
  DeviceLinkSupervisor,
  daemonRequestVersionMatches,
  deviceLinkFailureMessage,
  directoryHasEntries,
  legacyHostnameDeviceId,
  loadOrCreateDeviceId,
  archiveRevokedDeviceProfile,
  parseReadyRecord,
  parseDeviceLinkReadyRecord,
  parseDeviceLinkSecretRecord,
  parseDeviceLinkBootstrapError,
  resolveDaemonBinary,
  startDaemonRuntime,
  startupDocument,
  validDeviceLinkApproval,
  validDeviceId,
};
