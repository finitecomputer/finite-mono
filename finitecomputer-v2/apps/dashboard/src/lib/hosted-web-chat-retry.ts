const INITIAL_RETRY_BASE_DELAY_MS = 500;
const INITIAL_RETRY_LIMIT = 5;

export type HostedChatRetryAttempt = "succeeded" | "retry" | "stop";

export function initialHostedChatRetryDelay(failedAttempts: number) {
  if (!Number.isInteger(failedAttempts) || failedAttempts < 1) return null;
  if (failedAttempts > INITIAL_RETRY_LIMIT) return null;
  return INITIAL_RETRY_BASE_DELAY_MS * 2 ** (failedAttempts - 1);
}

export function waitForHostedChatRetry(delayMs: number, signal: AbortSignal) {
  if (signal.aborted) return Promise.resolve(false);

  return new Promise<boolean>((resolve) => {
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", cancel);
      resolve(true);
    }, delayMs);
    const cancel = () => {
      clearTimeout(timer);
      signal.removeEventListener("abort", cancel);
      resolve(false);
    };
    signal.addEventListener("abort", cancel, { once: true });
  });
}

export function shouldRetryHostedChatRequest(status: number | null) {
  return status === null || status === 408 || status === 429 || status >= 500;
}

export async function runInitialHostedChatRetries(
  attempt: () => Promise<HostedChatRetryAttempt>,
  signal: AbortSignal,
  wait: (delayMs: number, signal: AbortSignal) => Promise<boolean> = waitForHostedChatRetry
) {
  let failedAttempts = 0;
  while (!signal.aborted) {
    const result = await attempt();
    if (result !== "retry") return result;
    failedAttempts += 1;
    const delay = initialHostedChatRetryDelay(failedAttempts);
    if (delay === null) return "stop";
    if (!(await wait(delay, signal))) return "stop";
  }
  return "stop";
}
