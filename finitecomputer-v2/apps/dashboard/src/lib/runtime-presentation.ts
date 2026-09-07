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
 * health evidence Core's derived status rests on. Lower-case so callers can
 * embed it in a sentence.
 */
export function runtimeHealthAgeLabel(
  health: RuntimeHealthAgeSource | null | undefined,
  nowMs: number = Date.now()
): string {
  const age = runtimeHealthAgeSeconds(health, nowMs);
  return age === null ? "not yet checked" : `last checked ${formatAgeSeconds(age)} ago`;
}

/**
 * A sentence to append to Core's status wording, purely as an annotation:
 * Core derives `runtime_status` server-side and the dashboard never
 * re-derives it. When Core sends no `runtime_health` (an older Core, or a
 * mock without it) there is nothing to annotate and the wording is
 * unchanged.
 */
export function runtimeHealthAnnotation(
  health: RuntimeHealthAgeSource | null | undefined,
  nowMs: number = Date.now()
): string {
  if (!health) return "";
  const label = runtimeHealthAgeLabel(health, nowMs);
  return `${label.charAt(0).toUpperCase()}${label.slice(1)}.`;
}
