const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("finiteChatDesktop", {
  daemonUrl: () => ipcRenderer.invoke("finitechat:daemon-url"),
  consumePendingTargetUrl: () => ipcRenderer.invoke("finitechat:consume-pending-target-url"),
  identityStatus: () => ipcRenderer.invoke("finitechat:identity-status"),
  onboardingStatus: () => ipcRenderer.invoke("finitechat:onboarding-status"),
  completeOnboarding: () => ipcRenderer.invoke("finitechat:complete-onboarding"),
  importAccountSecret: (secret) => ipcRenderer.invoke("finitechat:import-account-secret", secret),
  clearAccountSecret: () => ipcRenderer.invoke("finitechat:clear-account-secret"),
  copyText: (text) => ipcRenderer.invoke("finitechat:copy-text", text),
  onTargetUrl: (callback) => {
    const listener = (_event, url) => callback(url);
    ipcRenderer.on("finitechat:target-url", listener);
    return () => ipcRenderer.removeListener("finitechat:target-url", listener);
  },
});
