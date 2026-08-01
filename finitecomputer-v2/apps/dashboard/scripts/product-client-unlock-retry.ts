type ProductClientBoundaryRetryOptions<T> = {
  attempts?: number;
  timeoutMs: number;
  attempt: (timeoutMs: number) => Promise<T>;
  reload: () => Promise<void>;
};

export async function retryProductClientBoundary<T>({
  attempts = 3,
  timeoutMs,
  attempt,
  reload,
}: ProductClientBoundaryRetryOptions<T>): Promise<T> {
  if (!Number.isInteger(attempts) || attempts < 1) {
    throw new Error("Product Client retry attempts must be a positive integer");
  }
  if (!Number.isFinite(timeoutMs) || timeoutMs < 1) {
    throw new Error("Product Client retry timeout must be positive");
  }

  const attemptTimeoutMs = Math.max(1, Math.ceil(timeoutMs / attempts));
  let lastError: unknown = null;
  for (let attemptNumber = 1; attemptNumber <= attempts; attemptNumber += 1) {
    try {
      return await attempt(attemptTimeoutMs);
    } catch (error) {
      lastError = error;
    }
    if (attemptNumber < attempts) await reload();
  }

  throw lastError;
}

type ProductClientUnlockRetryOptions = Omit<
  ProductClientBoundaryRetryOptions<void>,
  "attempt"
> & {
  waitForUnlock: (timeoutMs: number) => Promise<void>;
};

export async function retryProductClientUnlock({
  waitForUnlock,
  ...options
}: ProductClientUnlockRetryOptions): Promise<void> {
  await retryProductClientBoundary({
    ...options,
    attempt: waitForUnlock,
  });
}
