/** One observer per mounted route. Core remains the access authority; polling
 * only reconciles the visible server-rendered dashboard with access changes. */
export function startTrialAccessMonitor({
  blocked,
  pathname,
  refresh,
  replace,
}: {
  blocked: boolean;
  pathname: string;
  refresh: () => void;
  replace: (path: string) => void;
}) {
  const controller = new AbortController();
  let lastBlocked = blocked;
  let refreshAttempts = 0;
  let pending = false;
  let redirecting = false;
  async function check() {
    if (pending || redirecting || controller.signal.aborted) return;
    pending = true;
    try {
      const response = await fetch("/api/billing/trial-access", { cache: "no-store", signal: controller.signal });
      if (!response.ok) return;
      const status = await response.json();
      if (controller.signal.aborted || typeof status?.blocked !== "boolean") return;
      const changed = status.blocked !== lastBlocked;
      if (changed) refreshAttempts = 0;
      lastBlocked = status.blocked;
      if (status.blocked && pathname !== "/dashboard") {
        redirecting = true;
        replace("/dashboard");
      } else if ((changed || status.blocked !== blocked) && refreshAttempts < 3) {
        // Retry a stale/failed RSC refresh on later polls, at most three times
        // per access transition. Acknowledged server props restart the effect;
        // unchanged access must never produce an unbounded refresh loop.
        refreshAttempts++;
        refresh();
      }
    } catch { /* New agent requests still fail closed if Core is unavailable. */ }
    finally { pending = false; }
  }
  void check();
  const timer = setInterval(() => void check(), 30_000);
  return () => { clearInterval(timer); controller.abort(); };
}
