type ProductClientUnlockRetryOptions = {
  attempts?: number;
  timeoutMs: number;
  waitForUnlock: (timeoutMs: number) => Promise<void>;
  reload: () => Promise<void>;
};

export async function retryProductClientUnlock({
  attempts = 3,
  timeoutMs,
  waitForUnlock,
  reload,
}: ProductClientUnlockRetryOptions): Promise<void> {
  if (!Number.isInteger(attempts) || attempts < 1) {
    throw new Error("Product Client unlock attempts must be a positive integer");
  }
  if (!Number.isFinite(timeoutMs) || timeoutMs < 1) {
    throw new Error("Product Client unlock timeout must be positive");
  }

  const attemptTimeoutMs = Math.max(1, Math.ceil(timeoutMs / attempts));
  let lastError: unknown = null;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      await waitForUnlock(attemptTimeoutMs);
      return;
    } catch (error) {
      lastError = error;
    }
    if (attempt < attempts) await reload();
  }

  throw lastError;
}
