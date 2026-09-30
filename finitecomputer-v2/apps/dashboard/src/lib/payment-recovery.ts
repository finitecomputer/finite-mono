import type { CubeState } from "@/components/status-prism";
import type { CoreRuntimeStatus, CoreVisibleProject } from "./core-client";

export function paymentRecoveryPresentation(
  recovery: CoreVisibleProject["runtime_recovery"],
  runtimeStatus: CoreRuntimeStatus
): { description: string; state: CubeState; failed: boolean } | null {
  if (recovery === "failed") {
    return { description: "We couldn’t restart your agent after payment. Your home, data, and history are retained. Contact support for help restarting this agent.", state: "stuck", failed: true };
  }
  if (recovery === "restarting") {
    return { description: "Restarting your agent. Your home, data, and history are retained.", state: "working", failed: false };
  }
  if (runtimeStatus === "online") {
    return { description: "Your agent is ready.", state: "happy", failed: false };
  }
  return null;
}
