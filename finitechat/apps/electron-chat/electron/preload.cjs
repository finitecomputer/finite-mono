const { contextBridge, ipcRenderer } = require("electron");

// Sandboxed preloads may require Electron but not sibling files. Keep this
// public, non-secret projection self-contained and mirror its constants in the
// main-process validators' tests.
const DESKTOP_BRIDGE_CONTRACT_VERSION = 3;
const attachmentMediaScheme = "finitechat-media";
const maxOpaqueIdBytes = 1024;

function attachmentMediaUrl({ room_id, message_id, attachment_id } = {}) {
  const values = [room_id, message_id, attachment_id];
  if (!values.every(validOpaqueId)) {
    throw new Error("Finite Chat attachment media identifier is invalid");
  }
  return `${attachmentMediaScheme}://attachment/${values.map(encodeURIComponent).join("/")}`;
}

function validOpaqueId(value) {
  return (
    typeof value === "string"
    && value.length > 0
    && new TextEncoder().encode(value).byteLength <= maxOpaqueIdBytes
    && value !== "."
    && value !== ".."
    && !/[\\/]/u.test(value)
    && !/[\p{Cc}\p{Cf}]/u.test(value)
  );
}

function subscribe(channel, callback, start) {
  if (typeof callback !== "function") {
    throw new TypeError("Finite Chat desktop subscription requires a callback");
  }
  const listener = (_event, value) => callback(value);
  ipcRenderer.on(channel, listener);
  if (start) {
    void ipcRenderer.invoke(start).catch(() => {});
  }
  return () => ipcRenderer.removeListener(channel, listener);
}

contextBridge.exposeInMainWorld("finiteChatDesktop", Object.freeze({
  version: DESKTOP_BRIDGE_CONTRACT_VERSION,
  capabilities: Object.freeze([
    "local-chat-v1",
    "automatic-device-link-v1",
    "revoked-device-recovery-v1",
    "durable-chat-archive-v1",
  ]),
  ensureLocalDevice: () => ipcRenderer.invoke("finitechat:ensure-local-device"),
  recoverLocalDevice: () => ipcRenderer.invoke("finitechat:recover-local-device"),
  daemonState: () => ipcRenderer.invoke("finitechat:daemon-state"),
  dispatchDaemonAction: (action) => ipcRenderer.invoke("finitechat:daemon-action", action),
  uploadDaemonAttachments: (upload) => ipcRenderer.invoke("finitechat:daemon-attachments", upload),
  attachmentUrl: (coordinates) => attachmentMediaUrl(coordinates),
  onDaemonUpdate: (callback) => subscribe("finitechat:daemon-update", callback),
  onDaemonGeneration: (callback) =>
    subscribe("finitechat:daemon-generation", callback, "finitechat:daemon-subscribe"),
  onDaemonError: (callback) => subscribe("finitechat:daemon-error", callback),
  onDeviceLinkStatus: (callback) =>
    subscribe("finitechat:device-link-status", callback, "finitechat:device-link-subscribe"),
}));
