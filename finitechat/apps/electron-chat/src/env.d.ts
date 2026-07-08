interface FiniteChatDesktopBridge {
  daemonUrl(): Promise<string>;
  consumePendingTargetUrl(): Promise<string | null>;
  identityStatus(): Promise<{ secureStorageAvailable: boolean; hasStoredAccountSecret: boolean }>;
  onboardingStatus(): Promise<{ completed: boolean }>;
  completeOnboarding(): Promise<{ completed: boolean }>;
  importAccountSecret(secret: string): Promise<{ secureStorageAvailable: boolean; hasStoredAccountSecret: boolean }>;
  clearAccountSecret(): Promise<{ secureStorageAvailable: boolean; hasStoredAccountSecret: boolean }>;
  copyText(text: string): Promise<boolean>;
  onTargetUrl(callback: (url: string) => void): () => void;
}

interface Window {
  finiteChatDesktop?: FiniteChatDesktopBridge;
}
