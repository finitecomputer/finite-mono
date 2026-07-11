type DesktopDeviceLinkReady = {
  link_session_id: string;
  target_device_id: string;
  approval_url: string;
};

type DesktopDeviceLinkStatus =
  | { status: "idle" }
  | { status: "waiting"; ready: DesktopDeviceLinkReady }
  | { status: "linked" }
  | { status: "failed"; message: string }
  | { status: "cancelled" };

interface FiniteChatDesktopBridge {
  daemonConnection(): Promise<"finitechat-desktop-ipc">;
  daemonState(): Promise<import("./daemon").AppState>;
  dispatchDaemonAction(action: import("./daemon").AppAction): Promise<import("./daemon").AppState>;
  uploadDaemonAttachments(upload: import("./daemon").AttachmentUpload): Promise<import("./daemon").AppState>;
  consumePendingTargetUrl(): Promise<string | null>;
  identityStatus(): Promise<{
    secureStorageAvailable: boolean;
    hasStoredAccountSecret: boolean;
    linking: boolean;
  }>;
  onboardingStatus(): Promise<{ completed: boolean }>;
  completeOnboarding(): Promise<{ completed: boolean }>;
  clearAccountSecret(): Promise<{
    secureStorageAvailable: boolean;
    hasStoredAccountSecret: boolean;
    linking: boolean;
  }>;
  beginDeviceLink(): Promise<DesktopDeviceLinkReady>;
  openDeviceLinkApproval(approvalUrl: string): Promise<boolean>;
  cancelDeviceLink(): Promise<void>;
  copyText(text: string): Promise<boolean>;
  onTargetUrl(callback: (url: string) => void): () => void;
  onDaemonUpdate(callback: (state: import("./daemon").AppState) => void): () => void;
  onDaemonGeneration(callback: (generation: { generation: number }) => void): () => void;
  onDaemonError(callback: (message: string) => void): () => void;
  onDeviceLinkStatus(callback: (status: DesktopDeviceLinkStatus) => void): () => void;
}

interface Window {
  finiteChatDesktop?: FiniteChatDesktopBridge;
}
