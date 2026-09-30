import type { CubeState } from "@/components/status-prism";
import type { CoreVisibleProject } from "./core-client";

export function paymentRecoveryPresentation(
  recovery: CoreVisibleProject["runtime_recovery"]
): { description: string; state: CubeState; failed: boolean } | null {
  if (recovery === "restart_failed") {
    return { description: "We couldn’t restart your agent. Your home, data, and history are retained. Choose Restart agent to retry, or contact your Finite team for help.", state: "stuck", failed: true };
  }
  if (recovery === "failed") {
    return { description: "We couldn’t restart your agent after payment. Your home, data, and history are retained. Contact support for help restarting this agent.", state: "stuck", failed: true };
  }
  if (recovery === "restarting") {
    return { description: "Restarting your agent. Your home, data, and history are retained.", state: "working", failed: false };
  }
  if (recovery === "restart_pending") {
    return { description: "Waiting for your agent to be ready. Your home, data, and history are retained.", state: "working", failed: false };
  }
  // Without an observed recovery, the ordinary runtime overview (including its
  // health freshness annotation) stays authoritative.
  return null;
}
