import type { CubeState } from "@/components/status-prism";
import type { CoreRuntimeHealth, CoreRuntimeStatus } from "@/lib/core-client";

export function runtimePrismState(status: CoreRuntimeStatus): CubeState {
  if (status === "online") return "happy";
  if (status === "stale") return "working";
  if (status === "offline") return "off";
  return "stuck";
}

export function runtimeCanPresentActivity(status: CoreRuntimeStatus) {
  return status === "online";
}

type RuntimeHealthAgeSource = Pick<CoreRuntimeHealth, "observed_at" | "reported_at">;

/**
 * Seconds since the runner last read the runtime (`observed_at`, falling back
 * to Core's `reported_at`), or null when it never has. Clamped at zero so a
 * runner clock slightly ahead of ours never reads as "in the future".
 */
export function runtimeHealthAgeSeconds(
  health: RuntimeHealthAgeSource | null | undefined,
  nowMs: number = Date.now()
): number | null {
  const stamp = health?.observed_at ?? health?.reported_at;
  if (!stamp) return null;
  const parsed = Date.parse(stamp);
  if (!Number.isFinite(parsed)) return null;
  return Math.max(0, Math.round((nowMs - parsed) / 1000));
}

export function formatAgeSeconds(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3_600) return `${Math.round(seconds / 60)}m`;
  if (seconds < 86_400) return `${Math.round(seconds / 3_600)}h`;
  return `${Math.round(seconds / 86_400)}d`;
}

/**
 * "last checked 45s ago" / "not yet checked" — the age of the standing
 * health evidence the derived runtime status rests on. Lower-case so callers
 * can embed it in a sentence.
 */
export function runtimeHealthAgeLabel(
  health: RuntimeHealthAgeSource | null | undefined,
  nowMs: number = Date.now()
): string {
  const age = runtimeHealthAgeSeconds(health, nowMs);
  return age === null ? "not yet checked" : `last checked ${formatAgeSeconds(age)} ago`;
}

/**
 * One sentence to append to a status description: the check age for every
 * status that rests on report freshness, and the not-ready reason when the
 * runtime is offline because its last check said so.
 */
export function runtimeHealthSentence(
  status: CoreRuntimeStatus,
  health: CoreRuntimeHealth | null | undefined,
  nowMs: number = Date.now()
): string {
  if (status === "offline") {
    if (health?.status !== "not_ready") return "";
    const reason = health.reason === "unreachable" ? "not reachable" : health.reason;
    return reason ? `Last check: ${reason}.` : "Last check found it not ready.";
  }
  const label = runtimeHealthAgeLabel(health, nowMs);
  return `${label.charAt(0).toUpperCase()}${label.slice(1)}.`;
}
