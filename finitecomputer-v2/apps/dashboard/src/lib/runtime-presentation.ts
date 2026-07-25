import type { CubeState } from "@/components/status-prism";
import type { CoreRuntimeStatus } from "@/lib/core-client";

export function runtimePrismState(status: CoreRuntimeStatus): CubeState {
  if (status === "online") return "happy";
  if (status === "stale") return "working";
  if (status === "offline") return "off";
  return "stuck";
}

export function runtimeCanPresentActivity(status: CoreRuntimeStatus) {
  return status === "online";
}
