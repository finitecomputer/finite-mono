const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("finiteChatDesktop", {
  daemonConnection: () => ipcRenderer.invoke("finitechat:daemon-connection"),
  daemonState: () => ipcRenderer.invoke("finitechat:daemon-state"),
  dispatchDaemonAction: (action) => ipcRenderer.invoke("finitechat:daemon-action", action),
  uploadDaemonAttachments: (upload) => ipcRenderer.invoke("finitechat:daemon-attachments", upload),
  consumePendingTargetUrl: () => ipcRenderer.invoke("finitechat:consume-pending-target-url"),
  identityStatus: () => ipcRenderer.invoke("finitechat:identity-status"),
  onboardingStatus: () => ipcRenderer.invoke("finitechat:onboarding-status"),
  completeOnboarding: () => ipcRenderer.invoke("finitechat:complete-onboarding"),
  clearAccountSecret: () => ipcRenderer.invoke("finitechat:clear-account-secret"),
  beginDeviceLink: () => ipcRenderer.invoke("finitechat:begin-device-link"),
  openDeviceLinkApproval: (approvalUrl) =>
    ipcRenderer.invoke("finitechat:open-device-link-approval", approvalUrl),
  cancelDeviceLink: () => ipcRenderer.invoke("finitechat:cancel-device-link"),
  copyText: (text) => ipcRenderer.invoke("finitechat:copy-text", text),
  onTargetUrl: (callback) => {
    const listener = (_event, url) => callback(url);
    ipcRenderer.on("finitechat:target-url", listener);
    return () => ipcRenderer.removeListener("finitechat:target-url", listener);
  },
  onDaemonUpdate: (callback) => {
    const listener = (_event, state) => callback(state);
    ipcRenderer.on("finitechat:daemon-update", listener);
    return () => ipcRenderer.removeListener("finitechat:daemon-update", listener);
  },
  onDaemonGeneration: (callback) => {
    const listener = (_event, generation) => callback(generation);
    ipcRenderer.on("finitechat:daemon-generation", listener);
    void ipcRenderer.invoke("finitechat:daemon-subscribe").catch(() => {});
    return () => ipcRenderer.removeListener("finitechat:daemon-generation", listener);
  },
  onDaemonError: (callback) => {
    const listener = (_event, message) => callback(message);
    ipcRenderer.on("finitechat:daemon-error", listener);
    return () => ipcRenderer.removeListener("finitechat:daemon-error", listener);
  },
  onDeviceLinkStatus: (callback) => {
    const listener = (_event, status) => callback(status);
    ipcRenderer.on("finitechat:device-link-status", listener);
    return () => ipcRenderer.removeListener("finitechat:device-link-status", listener);
  },
});
